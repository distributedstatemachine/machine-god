//! Explicitly approved monitor effects. Descriptions and saved monitor state
//! never create native authority; a live, owner-bound grant is required.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::NativeOwnedWorkerSpawner;
use crate::background_process::ValidatedBackgroundEnvironment;
use crate::terminal_captured_exec::{TerminalCapturedExec, TerminalCapturedExecError};
use crate::terminal_monitor::{
    PROBE_OUTPUT_BYTES, PROBE_TIMEOUT_MS, TerminalPathBaseline, TerminalProbeEvidence,
    TerminalProbeFailure, TerminalProbeObservation, TerminalProbeRequest, TerminalProbeTarget,
};
use crate::terminal_shell::TerminalShell;
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, TerminalExecRequest, TerminalExecStatus,
    TerminalMonitorId, TerminalSessionId,
};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

const MAX_ADDRESSES: usize = 4;
const CONNECT_BUDGET: Duration = Duration::from_millis(250);
const HTTP_IO_BUDGET: Duration = Duration::from_millis(500);
const IO_SLICE: Duration = Duration::from_millis(25);
const HTTP_PREFIX_BYTES: usize = 1024;
type Result<T> = std::result::Result<T, TerminalProbeFailure>;

/// Immutable captured shell/environment shared by all authorized monitor grants.
pub(crate) struct TerminalProbeCustomContext {
    shell: TerminalShell,
    environment: Arc<ValidatedBackgroundEnvironment>,
}
impl TerminalProbeCustomContext {
    pub(crate) fn new(
        shell: TerminalShell,
        environment: Vec<(OsString, OsString)>,
    ) -> Result<Self> {
        Ok(Self {
            shell,
            environment: Arc::new(
                ValidatedBackgroundEnvironment::new(environment)
                    .map_err(|_| TerminalProbeFailure::InvalidEvidence)?,
            ),
        })
    }

    pub(crate) fn with_environment(
        shell: TerminalShell,
        environment: Arc<ValidatedBackgroundEnvironment>,
    ) -> Self {
        Self { shell, environment }
    }
}

/// Supplied only after the host separately approves this exact effect. DNS
/// resolution and workspace resolution belong to that explicit authority step.
pub(crate) enum TerminalProbeAuthority {
    Tcp {
        host: String,
        port: u16,
        addresses: Vec<SocketAddr>,
    },
    Http {
        url: String,
        addresses: Vec<SocketAddr>,
    },
    Path {
        path: String,
        parent: OwnedFd,
        leaf: OsString,
    },
    Custom {
        command: String,
        cwd: String,
        canonical_cwd: PathBuf,
        directory: OwnedFd,
        context: Arc<TerminalProbeCustomContext>,
    },
}

enum CapturedAuthority {
    Tcp {
        addresses: Vec<SocketAddr>,
    },
    Http {
        addresses: Vec<SocketAddr>,
        request: Vec<u8>,
    },
    Path {
        parent: OwnedFd,
        leaf: OsString,
    },
    Custom {
        request: TerminalExecRequest,
        directory: OwnedFd,
        context: Arc<TerminalProbeCustomContext>,
        cwd_sha256: [u8; 32],
    },
}

/// Nonserializable, reusable approval for one live owner/session/monitor
/// generation. Sharing this object does not re-open paths or resolve names.
pub(crate) struct TerminalProbeGrant {
    owner: BackgroundOutputOwner,
    session_id: TerminalSessionId,
    monitor_id: TerminalMonitorId,
    generation: u64,
    target: TerminalProbeTarget,
    authority: Arc<CapturedAuthority>,
    revocation: CancellationToken,
}
impl TerminalProbeGrant {
    pub(crate) fn new(
        owner: BackgroundOutputOwner,
        session_id: TerminalSessionId,
        monitor_id: TerminalMonitorId,
        generation: u64,
        authority: TerminalProbeAuthority,
    ) -> Result<Self> {
        if generation == 0 {
            return Err(TerminalProbeFailure::InvalidEvidence);
        }
        let (target, authority) = capture_authority(authority)?;
        Ok(Self {
            owner,
            session_id,
            monitor_id,
            generation,
            target,
            authority: Arc::new(authority),
            revocation: CancellationToken::new(),
        })
    }
    pub(crate) fn owner(&self) -> &BackgroundOutputOwner {
        &self.owner
    }
    pub(crate) fn session_id(&self) -> &TerminalSessionId {
        &self.session_id
    }
    pub(crate) fn monitor_id(&self) -> &TerminalMonitorId {
        &self.monitor_id
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn target(&self) -> &TerminalProbeTarget {
        &self.target
    }
    /// The host must revoke before retiring/replacing a grant or shutting down.
    /// Arc clones and already-queued calls cannot preserve revoked authority.
    pub(crate) fn revoke(&self) {
        self.revocation.cancel();
    }
    pub(crate) fn is_revoked(&self) -> bool {
        self.revocation.is_cancelled()
    }
    /// Only an explicitly authorized live resume receipt may renew a retained
    /// approval template. This does not discover authority from saved state.
    pub(crate) fn renew_generation(&self, generation: u64) -> Result<Self> {
        if generation <= self.generation {
            return Err(TerminalProbeFailure::Denied);
        }
        Ok(Self {
            owner: self.owner.clone(),
            session_id: self.session_id.clone(),
            monitor_id: self.monitor_id.clone(),
            generation,
            target: self.target.clone(),
            authority: Arc::clone(&self.authority),
            revocation: CancellationToken::new(),
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TerminalProbeClock {
    pub(crate) now_ms: i64,
    pub(crate) observed_at: Instant,
}

pub(crate) struct AuthorizedTerminalProbe {
    grant: Arc<TerminalProbeGrant>,
    identity: EvidenceIdentity,
    deadline: Instant,
}
impl AuthorizedTerminalProbe {
    /// Pure binding only. The grant is distinct from the scheduled description,
    /// and the supplied clock anchor preserves elapsed scheduler/queue time.
    pub(crate) fn new(
        owner: &BackgroundOutputOwner,
        request: TerminalProbeRequest,
        grant: Arc<TerminalProbeGrant>,
        clock: TerminalProbeClock,
    ) -> Result<Self> {
        if grant.is_revoked()
            || grant.owner() != owner
            || grant.session_id() != &request.session_id
            || grant.monitor_id() != &request.monitor_id
            || grant.generation() != request.generation
            || !same_target(grant.target(), &request.target)
        {
            return Err(TerminalProbeFailure::Denied);
        }
        if request.request_sequence == 0
            || request.started_at_ms < 0
            || request.deadline_ms.checked_sub(request.started_at_ms)
                != Some(PROBE_TIMEOUT_MS.cast_signed())
            || request.output_limit_bytes != PROBE_OUTPUT_BYTES
            || clock.now_ms < request.started_at_ms
        {
            return Err(TerminalProbeFailure::InvalidEvidence);
        }
        let remaining = request
            .deadline_ms
            .checked_sub(clock.now_ms)
            .filter(|remaining| *remaining > 0)
            .ok_or(TerminalProbeFailure::Timeout)?;
        let deadline = clock
            .observed_at
            .checked_add(Duration::from_millis(remaining.cast_unsigned()))
            .ok_or(TerminalProbeFailure::InvalidEvidence)?;
        Ok(Self {
            grant,
            deadline,
            identity: EvidenceIdentity {
                session_id: request.session_id,
                monitor_id: request.monitor_id,
                generation: request.generation,
                request_sequence: request.request_sequence,
                deadline_ms: request.deadline_ms,
                clock,
            },
        })
    }
}

#[derive(Clone)]
struct EvidenceIdentity {
    session_id: TerminalSessionId,
    monitor_id: TerminalMonitorId,
    generation: u64,
    request_sequence: u64,
    deadline_ms: i64,
    clock: TerminalProbeClock,
}
impl EvidenceIdentity {
    fn finish(&self, mut run: ProbeRun, cancellation: &CancellationToken) -> TerminalProbeEvidence {
        let elapsed = self.clock.observed_at.elapsed();
        let completed_at_ms = self
            .clock
            .now_ms
            .saturating_add(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
        let timed_out = completed_at_ms >= self.deadline_ms
            || matches!(run.result, Err(TerminalProbeFailure::Timeout));
        if cancellation.is_cancelled() {
            run.result = Err(TerminalProbeFailure::Unavailable);
        } else if timed_out && run.result.is_ok() {
            run.result = Err(TerminalProbeFailure::Timeout);
        }
        TerminalProbeEvidence {
            session_id: self.session_id.clone(),
            monitor_id: self.monitor_id.clone(),
            generation: self.generation,
            request_sequence: self.request_sequence,
            completed_at_ms,
            output_bytes: run.output_bytes,
            truncated: run.truncated,
            timed_out,
            result: run.result,
        }
    }
}

/// Inert configuration. Each polled call runs on the production owned-worker
/// collector; abandoned futures cancel work without abandoning native ownership.
pub(crate) struct NativeTerminalProbeExecutor {
    captured: Arc<TerminalCapturedExec>,
    active: Arc<AtomicUsize>,
    maximum_active: usize,
}
impl NativeTerminalProbeExecutor {
    pub(crate) fn new(captured: Arc<TerminalCapturedExec>, maximum_active: usize) -> Result<Self> {
        if !(1..=16).contains(&maximum_active) {
            return Err(TerminalProbeFailure::InvalidEvidence);
        }
        Ok(Self {
            captured,
            active: Arc::new(AtomicUsize::new(0)),
            maximum_active,
        })
    }
    pub(crate) fn execute(
        &self,
        request: AuthorizedTerminalProbe,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, TerminalProbeEvidence> {
        let active = Arc::clone(&self.active);
        let captured = Arc::clone(&self.captured);
        let maximum = self.maximum_active;
        Box::pin(async move {
            let identity = request.identity.clone();
            let revocation = request.grant.revocation.clone();
            if revocation.is_cancelled() {
                return identity.finish(
                    ProbeRun::failed(TerminalProbeFailure::Denied),
                    &cancellation,
                );
            }
            if cancellation.is_cancelled() {
                return identity.finish(
                    ProbeRun::failed(TerminalProbeFailure::Unavailable),
                    &cancellation,
                );
            }
            if active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < maximum).then_some(count + 1)
                })
                .is_err()
            {
                return identity.finish(
                    ProbeRun::failed(TerminalProbeFailure::Unavailable),
                    &cancellation,
                );
            }
            let permit = ProbePermit(active);
            let stop = CancellationToken::new();
            let _stop_on_drop = StopOnDrop(stop.clone());
            let worker_cancellation = cancellation.clone();
            match NativeOwnedWorkerSpawner::new()
                .run(move || {
                    let mut run = run_probe(
                        &request,
                        &captured,
                        &worker_cancellation,
                        &[&stop, &request.grant.revocation],
                    );
                    if request.grant.is_revoked() {
                        run.result = Err(TerminalProbeFailure::Denied);
                    }
                    (request.identity.finish(run, &worker_cancellation), permit)
                })
                .await
            {
                Ok((mut evidence, _permit)) => {
                    if revocation.is_cancelled() {
                        evidence.result = Err(TerminalProbeFailure::Denied);
                    } else if cancellation.is_cancelled() {
                        evidence.result = Err(TerminalProbeFailure::Unavailable);
                    }
                    evidence
                }
                Err(_) => identity.finish(
                    ProbeRun::failed(TerminalProbeFailure::Unavailable),
                    &cancellation,
                ),
            }
        })
    }
}
struct ProbePermit(Arc<AtomicUsize>);
impl Drop for ProbePermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct StopOnDrop(CancellationToken);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct ProbeRun {
    result: Result<TerminalProbeObservation>,
    output_bytes: u64,
    truncated: bool,
}
impl ProbeRun {
    fn failed(failure: TerminalProbeFailure) -> Self {
        Self {
            result: Err(failure),
            output_bytes: 0,
            truncated: false,
        }
    }
    fn observed(observation: TerminalProbeObservation, output_bytes: u64) -> Self {
        Self {
            result: Ok(observation),
            output_bytes,
            truncated: false,
        }
    }
}

fn boundary(
    deadline: Instant,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<()> {
    if cancellation.is_cancelled() || stop.iter().any(|token| token.is_cancelled()) {
        Err(TerminalProbeFailure::Unavailable)
    } else if Instant::now() >= deadline {
        Err(TerminalProbeFailure::Timeout)
    } else {
        Ok(())
    }
}

fn run_probe(
    request: &AuthorizedTerminalProbe,
    captured: &TerminalCapturedExec,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> ProbeRun {
    if Instant::now() < request.identity.clock.observed_at {
        return ProbeRun::failed(TerminalProbeFailure::InvalidEvidence);
    }
    if let Err(error) = boundary(request.deadline, cancellation, stop) {
        return ProbeRun::failed(error);
    }
    let result = match request.grant.authority.as_ref() {
        CapturedAuthority::Tcp { addresses } => {
            connect(addresses, request.deadline, cancellation, stop).map(|stream| {
                ProbeRun::observed(
                    TerminalProbeObservation::Tcp {
                        connected: stream.is_some(),
                    },
                    0,
                )
            })
        }
        CapturedAuthority::Http {
            addresses,
            request: wire,
        } => http(addresses, wire, request.deadline, cancellation, stop).map(|response_prefix| {
            let bytes = response_prefix.len() as u64;
            ProbeRun::observed(TerminalProbeObservation::Http { response_prefix }, bytes)
        }),
        CapturedAuthority::Path { parent, leaf } => path_baseline_checked(parent, leaf, || {
            boundary(request.deadline, cancellation, stop)
        })
        .map(|baseline| ProbeRun::observed(TerminalProbeObservation::Path { baseline }, 0)),
        CapturedAuthority::Custom {
            request: command,
            directory,
            context,
            cwd_sha256,
        } => custom(
            captured,
            command,
            directory,
            &context.shell,
            &context.environment,
            *cwd_sha256,
            request.deadline,
            cancellation,
            stop,
        ),
    };
    let mut result = result.unwrap_or_else(ProbeRun::failed);
    if let Err(error) = boundary(request.deadline, cancellation, stop)
        && result.result.is_ok()
    {
        result.result = Err(error);
    }
    result
}

fn text(value: &str, maximum: usize) -> Result<()> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        Err(TerminalProbeFailure::InvalidEvidence)
    } else {
        Ok(())
    }
}
fn addresses_valid(addresses: &[SocketAddr], host: &str, port: u16) -> Result<()> {
    if addresses.is_empty()
        || addresses.len() > MAX_ADDRESSES
        || port == 0
        || addresses.iter().any(|address| address.port() != port)
    {
        return Err(TerminalProbeFailure::InvalidEvidence);
    }
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>()
        && addresses.iter().any(|address| address.ip() != ip)
    {
        return Err(TerminalProbeFailure::Denied);
    }
    Ok(())
}

pub(crate) fn canonical_fingerprint(path: &std::path::Path) -> Result<(&str, [u8; 32])> {
    let canonical = path.to_str().ok_or(TerminalProbeFailure::InvalidEvidence)?;
    text(canonical, 4096)?;
    if !path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Err(TerminalProbeFailure::InvalidEvidence);
    }
    Ok((canonical, Sha256::digest(canonical.as_bytes()).into()))
}

fn validate_leaf(leaf: &std::ffi::OsStr) -> Result<()> {
    let bytes = leaf.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 4096
        || bytes.contains(&0)
        || (bytes != b"."
            && bytes.split(|byte| *byte == b'/').any(|component| {
                component.is_empty() || component.len() > 255 || matches!(component, b"." | b"..")
            }))
    {
        return Err(TerminalProbeFailure::InvalidEvidence);
    }
    Ok(())
}

fn capture_authority(
    authority: TerminalProbeAuthority,
) -> Result<(TerminalProbeTarget, CapturedAuthority)> {
    Ok(match authority {
        TerminalProbeAuthority::Tcp {
            host,
            port,
            addresses,
        } => {
            text(&host, 4096)?;
            addresses_valid(&addresses, &host, port)?;
            (
                TerminalProbeTarget::Tcp { host, port },
                CapturedAuthority::Tcp { addresses },
            )
        }
        TerminalProbeAuthority::Http { url, addresses } => {
            text(&url, 4096)?;
            let parsed =
                url::Url::parse(&url).map_err(|_| TerminalProbeFailure::InvalidEvidence)?;
            let authority = url
                .split_once("://")
                .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""));
            if parsed.scheme() != "http"
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || authority.is_none_or(|authority| authority.contains('@'))
            {
                return Err(TerminalProbeFailure::InvalidEvidence);
            }
            let host = parsed
                .host_str()
                .ok_or(TerminalProbeFailure::InvalidEvidence)?;
            addresses_valid(
                &addresses,
                host,
                parsed
                    .port_or_known_default()
                    .ok_or(TerminalProbeFailure::InvalidEvidence)?,
            )?;
            let target = &parsed[url::Position::BeforePath..url::Position::AfterQuery];
            let wire =
                format!("GET {target} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                    .into_bytes();
            if wire.len() > PROBE_OUTPUT_BYTES {
                return Err(TerminalProbeFailure::InvalidEvidence);
            }
            (
                TerminalProbeTarget::Http { url },
                CapturedAuthority::Http {
                    addresses,
                    request: wire,
                },
            )
        }
        TerminalProbeAuthority::Path { path, parent, leaf } => {
            text(&path, 4096)?;
            validate_leaf(&leaf)?;
            (
                TerminalProbeTarget::Path { path },
                CapturedAuthority::Path { parent, leaf },
            )
        }
        TerminalProbeAuthority::Custom {
            command,
            cwd,
            canonical_cwd,
            directory,
            context,
        } => {
            text(&command, 64 * 1024)?;
            text(&cwd, 4096)?;
            let (canonical, cwd_sha256) = canonical_fingerprint(&canonical_cwd)?;
            let request = TerminalExecRequest {
                command: command.clone(),
                cwd: canonical.into(),
                profile: Some(context.shell.profile()),
            };
            request
                .validate()
                .map_err(|_| TerminalProbeFailure::InvalidEvidence)?;
            (
                TerminalProbeTarget::Custom {
                    command,
                    cwd,
                    approved_cwd_sha256: cwd_sha256,
                },
                CapturedAuthority::Custom {
                    request,
                    directory,
                    context,
                    cwd_sha256,
                },
            )
        }
    })
}

fn same_target(left: &TerminalProbeTarget, right: &TerminalProbeTarget) -> bool {
    match (left, right) {
        (
            TerminalProbeTarget::Tcp { host: a, port: b },
            TerminalProbeTarget::Tcp { host: c, port: d },
        ) => a == c && b == d,
        (TerminalProbeTarget::Http { url: a }, TerminalProbeTarget::Http { url: b })
        | (TerminalProbeTarget::Path { path: a }, TerminalProbeTarget::Path { path: b }) => a == b,
        (
            TerminalProbeTarget::Custom {
                command: left_command,
                cwd: left_cwd,
                approved_cwd_sha256: left_digest,
            },
            TerminalProbeTarget::Custom {
                command: right_command,
                cwd: right_cwd,
                approved_cwd_sha256: right_digest,
            },
        ) => left_command == right_command && left_cwd == right_cwd && left_digest == right_digest,
        _ => false,
    }
}

fn connect(
    addresses: &[SocketAddr],
    deadline: Instant,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<Option<TcpStream>> {
    for address in addresses {
        boundary(deadline, cancellation, stop)?;
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .min(CONNECT_BUDGET);
        if remaining.is_zero() {
            return Err(TerminalProbeFailure::Timeout);
        }
        if let Ok(stream) = TcpStream::connect_timeout(address, remaining) {
            boundary(deadline, cancellation, stop)?;
            return Ok(Some(stream));
        }
    }
    boundary(deadline, cancellation, stop)?;
    Ok(None)
}

fn http(
    addresses: &[SocketAddr],
    request: &[u8],
    deadline: Instant,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<Vec<u8>> {
    let mut stream = connect(addresses, deadline, cancellation, stop)?
        .ok_or(TerminalProbeFailure::Unavailable)?;
    let write_deadline = deadline.min(Instant::now() + HTTP_IO_BUDGET);
    let mut written = 0;
    while written < request.len() {
        boundary(write_deadline, cancellation, stop)?;
        stream
            .set_write_timeout(Some(
                IO_SLICE.min(write_deadline.saturating_duration_since(Instant::now())),
            ))
            .map_err(unavailable)?;
        match stream.write(&request[written..]) {
            Ok(0) => return Err(TerminalProbeFailure::Unavailable),
            Ok(count) => written += count,
            Err(error) if retry_io(&error) => {}
            Err(_) => return Err(TerminalProbeFailure::Unavailable),
        }
    }
    let read_deadline = deadline.min(Instant::now() + HTTP_IO_BUDGET);
    let mut response = [0u8; HTTP_PREFIX_BYTES];
    loop {
        boundary(read_deadline, cancellation, stop)?;
        stream
            .set_read_timeout(Some(
                IO_SLICE.min(read_deadline.saturating_duration_since(Instant::now())),
            ))
            .map_err(unavailable)?;
        match stream.read(&mut response) {
            Ok(count) => return Ok(response[..count].to_vec()),
            Err(error) if retry_io(&error) => {}
            Err(_) => return Err(TerminalProbeFailure::Unavailable),
        }
    }
}
fn retry_io(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}
fn unavailable(_: impl std::fmt::Debug) -> TerminalProbeFailure {
    TerminalProbeFailure::Unavailable
}

pub(crate) fn path_baseline(parent: &OwnedFd, leaf: &OsString) -> Result<TerminalPathBaseline> {
    path_baseline_checked(parent, leaf, || Ok(()))
}

fn path_baseline_checked(
    parent: &OwnedFd,
    leaf: &OsString,
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<TerminalPathBaseline> {
    validate_leaf(leaf)?;
    checkpoint()?;
    let parent_stat = rustix::fs::fstat(parent).map_err(unavailable)?;
    if FileType::from_raw_mode(parent_stat.st_mode) != FileType::Directory {
        return Err(TerminalProbeFailure::Denied);
    }
    let mut directory = rustix::io::fcntl_dupfd_cloexec(parent, 3).map_err(unavailable)?;
    let mut components = leaf.as_bytes().split(|byte| *byte == b'/').peekable();
    let final_leaf = loop {
        checkpoint()?;
        let component = components
            .next()
            .ok_or(TerminalProbeFailure::InvalidEvidence)?;
        if components.peek().is_none() {
            break std::ffi::OsStr::from_bytes(component);
        }
        directory = match rustix::fs::openat(
            &directory,
            std::ffi::OsStr::from_bytes(component),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        ) {
            Ok(directory) => directory,
            Err(rustix::io::Errno::NOENT) => {
                return Ok(TerminalPathBaseline {
                    exists: false,
                    size: 0,
                    modified_ns: 0,
                });
            }
            Err(_) => return Err(TerminalProbeFailure::Denied),
        };
    };
    checkpoint()?;
    match rustix::fs::statat(&directory, final_leaf, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(TerminalPathBaseline {
            exists: true,
            size: u64::try_from(stat.st_size).map_err(unavailable)?,
            modified_ns: i128::from(stat.st_mtime) * 1_000_000_000 + i128::from(stat.st_mtime_nsec),
        }),
        Err(rustix::io::Errno::NOENT) => Ok(TerminalPathBaseline {
            exists: false,
            size: 0,
            modified_ns: 0,
        }),
        Err(_) => Err(TerminalProbeFailure::Unavailable),
    }
}

#[allow(clippy::too_many_arguments)]
fn custom(
    captured: &TerminalCapturedExec,
    request: &TerminalExecRequest,
    directory: &OwnedFd,
    shell: &TerminalShell,
    environment: &ValidatedBackgroundEnvironment,
    cwd_sha256: [u8; 32],
    deadline: Instant,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<ProbeRun> {
    boundary(deadline, cancellation, stop)?;
    let held = rustix::fs::fstat(directory).map_err(unavailable)?;
    let named = rustix::fs::statat(rustix::fs::CWD, &request.cwd, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(unavailable)?;
    if FileType::from_raw_mode(held.st_mode) != FileType::Directory
        || FileType::from_raw_mode(named.st_mode) != FileType::Directory
        || held.st_dev != named.st_dev
        || held.st_ino != named.st_ino
    {
        return Err(TerminalProbeFailure::Denied);
    }
    let directory = rustix::io::fcntl_dupfd_cloexec(directory, 3).map_err(unavailable)?;
    let outcome = captured
        .execute_probe_on_worker(
            request,
            shell,
            environment,
            directory,
            deadline,
            PROBE_OUTPUT_BYTES,
            cancellation,
            stop,
        )
        .map_err(|error| match error {
            TerminalCapturedExecError::Invalid => TerminalProbeFailure::Denied,
            _ => TerminalProbeFailure::Unavailable,
        })?;
    let result = match outcome.status {
        TerminalExecStatus::Exited { exit_code }
            if !outcome.truncated && outcome.output_bytes <= PROBE_OUTPUT_BYTES as u64 =>
        {
            Ok(TerminalProbeObservation::Custom {
                exit_code,
                cwd_sha256,
            })
        }
        TerminalExecStatus::TimedOut {} => Err(TerminalProbeFailure::Timeout),
        TerminalExecStatus::OutputLimit {} => Err(TerminalProbeFailure::OutputLimit),
        _ => Err(TerminalProbeFailure::Unavailable),
    };
    Ok(ProbeRun {
        result,
        output_bytes: outcome.output_bytes,
        truncated: outcome.truncated || outcome.output_bytes > PROBE_OUTPUT_BYTES as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{SessionId, SessionIncarnationId};
    use rustix::fs::{Mode, OFlags};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicU64;

    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn owner() -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("probe-owner").unwrap(),
            SessionIncarnationId::new("probe-incarnation").unwrap(),
        )
    }
    fn grant(authority: TerminalProbeAuthority) -> Arc<TerminalProbeGrant> {
        Arc::new(
            TerminalProbeGrant::new(
                owner(),
                TerminalSessionId::new("terminal-probe").unwrap(),
                TerminalMonitorId::new("monitor-1").unwrap(),
                1,
                authority,
            )
            .unwrap(),
        )
    }
    fn request(grant: &TerminalProbeGrant) -> TerminalProbeRequest {
        TerminalProbeRequest {
            session_id: grant.session_id().clone(),
            monitor_id: grant.monitor_id().clone(),
            generation: grant.generation(),
            request_sequence: 7,
            started_at_ms: 100,
            deadline_ms: 2100,
            output_limit_bytes: PROBE_OUTPUT_BYTES,
            target: grant.target().clone(),
        }
    }
    fn bind(grant: Arc<TerminalProbeGrant>) -> AuthorizedTerminalProbe {
        AuthorizedTerminalProbe::new(
            &owner(),
            request(&grant),
            grant,
            TerminalProbeClock {
                now_ms: 100,
                observed_at: Instant::now(),
            },
        )
        .unwrap()
    }
    fn directory(path: &std::path::Path) -> OwnedFd {
        rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap()
    }
    struct Fixture {
        root: PathBuf,
        executor: NativeTerminalProbeExecutor,
        harness_bytes: usize,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "machine-god-probe-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            let (program, arguments, harness_bytes) =
                std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY").map_or_else(
                    || {
                        (
                            std::env::current_exe().unwrap(),
                            vec![
                                "--exact".into(),
                                "terminal_captured_exec::tests::captured_helper_child".into(),
                                "--ignored".into(),
                                "--nocapture".into(),
                                "--quiet".into(),
                            ],
                            b"\nrunning 1 test\n".len(),
                        )
                    },
                    |program| {
                        (
                            PathBuf::from(program),
                            vec![
                                crate::terminal_captured_exec::TERMINAL_CAPTURED_HELPER_ARGUMENT
                                    .into(),
                            ],
                            0,
                        )
                    },
                );
            let captured = Arc::new(
                TerminalCapturedExec::new(program, arguments, Duration::from_secs(20), 4).unwrap(),
            );
            Self {
                root,
                executor: NativeTerminalProbeExecutor::new(captured, 2).unwrap(),
                harness_bytes,
            }
        }
        fn custom(&self, command: String) -> Arc<TerminalProbeGrant> {
            grant(TerminalProbeAuthority::Custom {
                command,
                cwd: ".".into(),
                canonical_cwd: self.root.clone(),
                directory: directory(&self.root),
                context: Arc::new(
                    TerminalProbeCustomContext::new(
                        TerminalShell::from_executable("/bin/bash".as_ref(), true).unwrap(),
                        vec![("PATH".into(), "/usr/bin:/bin".into())],
                    )
                    .unwrap(),
                ),
            })
        }
        fn run(&self, grant: Arc<TerminalProbeGrant>) -> TerminalProbeEvidence {
            futures_executor::block_on(self.executor.execute(bind(grant), CancellationToken::new()))
        }
        fn settled(&self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.executor.active.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(self.executor.active.load(Ordering::Acquire), 0);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.settled();
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn probe_grants_bind_owner_generation_target_sequence_and_budget_without_effects() {
        let fixture = Fixture::new();
        let grant = fixture.custom("printf forbidden > forbidden".into());
        let clock = TerminalProbeClock {
            now_ms: 100,
            observed_at: Instant::now(),
        };
        for field in 0..8 {
            let mut request = request(&grant);
            match field {
                0 => request.session_id = TerminalSessionId::new("other").unwrap(),
                1 => request.monitor_id = TerminalMonitorId::new("monitor-2").unwrap(),
                2 => request.generation += 1,
                3 => {
                    request.target = TerminalProbeTarget::Path {
                        path: "forbidden".into(),
                    }
                }
                4 => request.request_sequence = 0,
                5 => request.deadline_ms += 1,
                6 => request.output_limit_bytes += 1,
                _ => {
                    if let TerminalProbeTarget::Custom {
                        approved_cwd_sha256,
                        ..
                    } = &mut request.target
                    {
                        *approved_cwd_sha256 = [0; 32];
                    }
                }
            }
            assert!(
                AuthorizedTerminalProbe::new(&owner(), request, Arc::clone(&grant), clock).is_err()
            );
        }
        let wrong_owner = BackgroundOutputOwner::new(
            SessionId::new("probe-owner").unwrap(),
            SessionIncarnationId::new("other").unwrap(),
        );
        assert!(matches!(
            AuthorizedTerminalProbe::new(&wrong_owner, request(&grant), Arc::clone(&grant), clock),
            Err(TerminalProbeFailure::Denied)
        ));
        assert!(!fixture.root.join("forbidden").exists());
        let first = fixture.run(Arc::clone(&grant));
        let second = fixture.run(Arc::clone(&grant));
        for evidence in [first, second] {
            assert_eq!(evidence.session_id, *grant.session_id());
            assert_eq!(evidence.monitor_id, *grant.monitor_id());
            assert_eq!(evidence.generation, grant.generation());
            assert_eq!(evidence.request_sequence, 7);
            assert!(matches!(
                evidence.result,
                Ok(TerminalProbeObservation::Custom { exit_code: 0, .. })
            ));
        }
    }

    #[test]
    fn probe_custom_capture_exact_limit_and_one_over_do_not_weaken_public_exec() {
        let fixture = Fixture::new();
        for over in [0, 1] {
            let bytes = PROBE_OUTPUT_BYTES - fixture.harness_bytes + over;
            let evidence =
                fixture.run(fixture.custom(format!("/usr/bin/head -c {bytes} /dev/zero")));
            assert_eq!(evidence.output_bytes, (PROBE_OUTPUT_BYTES + over) as u64);
            assert_eq!(evidence.truncated, over != 0);
            if over == 0 {
                assert!(matches!(
                    evidence.result,
                    Ok(TerminalProbeObservation::Custom { exit_code: 0, .. })
                ));
            } else {
                assert!(matches!(
                    evidence.result,
                    Err(TerminalProbeFailure::OutputLimit)
                ));
            }
        }
        let evidence = fixture.run(fixture.custom("exit 19".into()));
        assert!(matches!(
            evidence.result,
            Ok(TerminalProbeObservation::Custom { exit_code: 19, .. })
        ));
        let evidence = fixture.run(fixture.custom("kill -KILL $$".into()));
        assert!(matches!(
            evidence.result,
            Err(TerminalProbeFailure::Unavailable)
        ));
    }

    #[test]
    fn probe_queue_delay_and_custom_timeout_keep_original_deadline() {
        let fixture = Fixture::new();
        let grant = fixture.custom("printf forbidden > forbidden".into());
        let expired = AuthorizedTerminalProbe::new(
            &owner(),
            request(&grant),
            grant,
            TerminalProbeClock {
                now_ms: 100,
                observed_at: Instant::now().checked_sub(Duration::from_secs(3)).unwrap(),
            },
        )
        .unwrap();
        let evidence =
            futures_executor::block_on(fixture.executor.execute(expired, CancellationToken::new()));
        assert!(evidence.timed_out);
        assert!(matches!(
            evidence.result,
            Err(TerminalProbeFailure::Timeout)
        ));
        assert!(!fixture.root.join("forbidden").exists());
        let grant = fixture.custom("exec /bin/sleep 30".into());
        let short = AuthorizedTerminalProbe::new(
            &owner(),
            request(&grant),
            grant,
            TerminalProbeClock {
                now_ms: 2000,
                observed_at: Instant::now(),
            },
        )
        .unwrap();
        let started = Instant::now();
        let evidence =
            futures_executor::block_on(fixture.executor.execute(short, CancellationToken::new()));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(evidence.timed_out);
        assert!(matches!(
            evidence.result,
            Err(TerminalProbeFailure::Timeout)
        ));
    }

    #[test]
    fn probe_path_baselines_use_retained_parent_and_do_not_follow_leaf_symlinks() {
        let fixture = Fixture::new();
        let grant = grant(TerminalProbeAuthority::Path {
            path: "ready".into(),
            parent: directory(&fixture.root),
            leaf: "ready".into(),
        });
        let absent = fixture.run(Arc::clone(&grant));
        assert!(matches!(
            absent.result,
            Ok(TerminalProbeObservation::Path {
                baseline: TerminalPathBaseline {
                    exists: false,
                    size: 0,
                    modified_ns: 0
                }
            })
        ));
        std::fs::write(fixture.root.join("ready"), b"12345").unwrap();
        let present = fixture.run(Arc::clone(&grant));
        assert!(matches!(
            present.result,
            Ok(TerminalProbeObservation::Path {
                baseline: TerminalPathBaseline {
                    exists: true,
                    size: 5,
                    ..
                }
            })
        ));
        std::fs::remove_file(fixture.root.join("ready")).unwrap();
        std::os::unix::fs::symlink("unavailable-destination", fixture.root.join("ready")).unwrap();
        let link = fixture.run(grant);
        assert!(matches!(
            link.result,
            Ok(TerminalProbeObservation::Path {
                baseline: TerminalPathBaseline { exists: true, .. }
            })
        ));
        for leaf in ["", "..", "../b", "a//b", "a/./b", "/a"] {
            assert!(
                TerminalProbeGrant::new(
                    owner(),
                    TerminalSessionId::new("terminal-probe").unwrap(),
                    TerminalMonitorId::new("monitor-1").unwrap(),
                    1,
                    TerminalProbeAuthority::Path {
                        path: "ready".into(),
                        parent: directory(&fixture.root),
                        leaf: leaf.into()
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn probe_custom_directory_replacement_cannot_redirect_execution() {
        let fixture = Fixture::new();
        let grant = fixture.custom("printf forbidden > forbidden".into());
        let moved = fixture.root.with_extension("retained");
        std::fs::rename(&fixture.root, &moved).unwrap();
        std::fs::create_dir(&fixture.root).unwrap();
        let evidence = fixture.run(grant);
        assert!(matches!(evidence.result, Err(TerminalProbeFailure::Denied)));
        assert!(!fixture.root.join("forbidden").exists());
        assert!(!moved.join("forbidden").exists());
        std::fs::remove_dir_all(moved).unwrap();
    }

    fn http_server(response: Vec<u8>) -> (SocketAddr, std::thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(Instant::now() < until, "probe never connected");
                std::thread::sleep(Duration::from_millis(5));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 512];
            while !request.ends_with(b"\r\n\r\n") {
                let count = stream.read(&mut bytes).unwrap();
                assert!(count > 0 && request.len() + count < PROBE_OUTPUT_BYTES);
                request.extend_from_slice(&bytes[..count]);
            }
            let _ = stream.write_all(&response);
            request
        });
        (address, server)
    }

    #[test]
    fn probe_native_http_uses_approved_address_exact_target_and_never_redirects() {
        let fixture = Fixture::new();
        let redirect = TcpListener::bind("127.0.0.1:0").unwrap();
        redirect.set_nonblocking(true).unwrap();
        let response = format!(
            "HTTP/1.0 302 Found\r\nLocation: http://{}/redirect\r\n\r\n",
            redirect.local_addr().unwrap()
        )
        .into_bytes();
        let (address, server) = http_server(response.clone());
        let grant = grant(TerminalProbeAuthority::Http {
            url: format!("http://{address}/ready?name=%E7%95%8C#ignored"),
            addresses: vec![address],
        });
        let evidence = fixture.run(grant);
        match evidence.result {
            Ok(TerminalProbeObservation::Http { response_prefix }) => {
                assert_eq!(response_prefix, response);
            }
            _ => panic!("HTTP probe failed"),
        }
        assert_eq!(evidence.output_bytes, response.len() as u64);
        assert!(!evidence.truncated && !evidence.timed_out);
        assert_eq!(
            server.join().unwrap(),
            b"GET /ready?name=%E7%95%8C HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        );
        assert!(matches!(redirect.accept(),Err(error) if error.kind()==io::ErrorKind::WouldBlock));
        for url in [
            "https://localhost/",
            "http://user@localhost/",
            "http://@localhost/",
        ] {
            assert!(
                TerminalProbeGrant::new(
                    owner(),
                    TerminalSessionId::new("terminal-probe").unwrap(),
                    TerminalMonitorId::new("monitor-1").unwrap(),
                    1,
                    TerminalProbeAuthority::Http {
                        url: url.into(),
                        addresses: vec!["127.0.0.1:80".parse().unwrap()]
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn probe_http_prefix_is_bounded_and_tcp_connections_are_real() {
        let fixture = Fixture::new();
        let mut response = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
        response.extend_from_slice(&vec![b'x'; 32 * 1024]);
        let (address, server) = http_server(response);
        let evidence = fixture.run(grant(TerminalProbeAuthority::Http {
            url: format!("http://{address}"),
            addresses: vec![address],
        }));
        assert_eq!(evidence.output_bytes, HTTP_PREFIX_BYTES as u64);
        assert!(
            matches!(evidence.result,Ok(TerminalProbeObservation::Http {response_prefix}) if response_prefix.len()==HTTP_PREFIX_BYTES)
        );
        server.join().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let approved = grant(TerminalProbeAuthority::Tcp {
            host: "127.0.0.1".into(),
            port: address.port(),
            addresses: vec![address],
        });
        assert!(matches!(
            fixture.run(Arc::clone(&approved)).result,
            Ok(TerminalProbeObservation::Tcp { connected: true })
        ));
        drop(listener);
        assert!(matches!(
            fixture.run(approved).result,
            Ok(TerminalProbeObservation::Tcp { connected: false })
        ));
    }

    #[test]
    fn probe_revocation_denies_queued_clones_and_stops_unpolled_running_workers() {
        let fixture = Fixture::new();
        let approved = fixture.custom("printf forbidden > forbidden".into());
        let queued = bind(Arc::clone(&approved));
        let clone = Arc::clone(&approved);
        approved.revoke();
        assert!(clone.is_revoked());
        assert!(matches!(
            AuthorizedTerminalProbe::new(
                &owner(),
                request(&clone),
                clone,
                TerminalProbeClock {
                    now_ms: 100,
                    observed_at: Instant::now()
                }
            ),
            Err(TerminalProbeFailure::Denied)
        ));
        assert!(matches!(
            futures_executor::block_on(fixture.executor.execute(queued, CancellationToken::new()))
                .result,
            Err(TerminalProbeFailure::Denied)
        ));
        assert!(!fixture.root.join("forbidden").exists());

        let approved = fixture.custom("printf '%s' \"$$\" > leader; exec /bin/sleep 30".into());
        let mut future = fixture
            .executor
            .execute(bind(Arc::clone(&approved)), CancellationToken::new());
        futures_executor::block_on(async {
            assert!(futures_util::poll!(&mut future).is_pending());
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let pid = loop {
            if let Ok(text) = std::fs::read_to_string(fixture.root.join("leader"))
                && let Ok(pid) = text.parse::<i32>()
            {
                break rustix::process::Pid::from_raw(pid).unwrap();
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        approved.revoke();
        // No more future polls: the worker observes the grant token itself.
        while rustix::process::test_kill_process(pid).is_ok() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(
            futures_executor::block_on(future).result,
            Err(TerminalProbeFailure::Denied)
        ));
        fixture.settled();
    }

    #[test]
    fn probe_revocation_rejects_late_completed_evidence() {
        let fixture = Fixture::new();
        let approved = grant(TerminalProbeAuthority::Path {
            path: ".".into(),
            parent: directory(&fixture.root),
            leaf: "absent".into(),
        });
        let mut future = fixture
            .executor
            .execute(bind(Arc::clone(&approved)), CancellationToken::new());
        futures_executor::block_on(async {
            assert!(futures_util::poll!(&mut future).is_pending());
        });
        // The retained response owns a permit; do not consume the evidence until
        // after authority is retired, even if the worker already finished.
        std::thread::sleep(Duration::from_millis(100));
        approved.revoke();
        assert!(matches!(
            futures_executor::block_on(future).result,
            Err(TerminalProbeFailure::Denied)
        ));
        fixture.settled();
    }

    #[test]
    fn probe_unpolled_cancelled_and_abandoned_calls_leave_no_process() {
        let fixture = Fixture::new();
        let grant = fixture.custom("printf '%s' \"$$\" > leader; exec /bin/sleep 30".into());
        drop(
            fixture
                .executor
                .execute(bind(Arc::clone(&grant)), CancellationToken::new()),
        );
        assert!(!fixture.root.join("leader").exists());
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let evidence = futures_executor::block_on(
            fixture
                .executor
                .execute(bind(Arc::clone(&grant)), cancelled),
        );
        assert!(matches!(
            evidence.result,
            Err(TerminalProbeFailure::Unavailable)
        ));
        assert!(!fixture.root.join("leader").exists());
        for cancel in [false, true] {
            let cancellation = CancellationToken::new();
            let mut future = fixture
                .executor
                .execute(bind(Arc::clone(&grant)), cancellation.clone());
            futures_executor::block_on(async {
                assert!(futures_util::poll!(&mut future).is_pending());
            });
            let deadline = Instant::now() + Duration::from_secs(1);
            let pid = loop {
                if let Ok(text) = std::fs::read_to_string(fixture.root.join("leader"))
                    && let Ok(pid) = text.parse::<i32>()
                {
                    break pid;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            };
            if cancel {
                cancellation.cancel();
                assert!(matches!(
                    futures_executor::block_on(future).result,
                    Err(TerminalProbeFailure::Unavailable)
                ));
            } else {
                drop(future);
            }
            fixture.settled();
            assert_eq!(
                rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()),
                Err(rustix::io::Errno::SRCH)
            );
            std::fs::remove_file(fixture.root.join("leader")).unwrap();
        }
    }
}
