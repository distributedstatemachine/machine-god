//! Explicitly approved monitor effects. Descriptions and saved monitor state
//! never create native authority; a live, owner-bound grant is required.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::background_process::ValidatedBackgroundEnvironment;
use crate::terminal_captured_exec::{TerminalCapturedExec, TerminalCapturedExecError};
use crate::terminal_monitor::{
    PROBE_OUTPUT_BYTES, PROBE_TIMEOUT_MS, TerminalPathBaseline, TerminalProbeEvidence,
    TerminalProbeFailure, TerminalProbeObservation, TerminalProbeRequest, TerminalProbeTarget,
};
use crate::terminal_shell::TerminalShell;
use crate::{NativeOwnedWorkerScope, NativeOwnedWorkerSpawner};
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
    #[cfg(test)]
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

    fn for_installed_monitor(self: &Arc<Self>) -> Result<Arc<Self>> {
        let Some(sandbox) = self.shell.sandbox() else {
            return Ok(Arc::clone(self));
        };
        Ok(Arc::new(Self::with_environment(
            self.shell.clone().with_sandbox(Arc::new(
                sandbox
                    .for_installed_monitor()
                    .map_err(|_| TerminalProbeFailure::Denied)?,
            )),
            Arc::clone(&self.environment),
        )))
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
    worker_scope: Option<NativeOwnedWorkerScope>,
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
            worker_scope: None,
        })
    }
    pub(crate) fn with_worker_scope(mut self, scope: NativeOwnedWorkerScope) -> Self {
        self.worker_scope = Some(scope);
        self
    }
    pub(crate) fn execute(
        &self,
        request: AuthorizedTerminalProbe,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, TerminalProbeEvidence> {
        let active = Arc::clone(&self.active);
        let captured = Arc::clone(&self.captured);
        let maximum = self.maximum_active;
        let worker_scope = self.worker_scope.clone();
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
            let operation = move || {
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
            };
            let reply = match worker_scope {
                Some(scope) => scope.run(operation),
                None => NativeOwnedWorkerSpawner::new().run(operation),
            };
            match reply.await {
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
            let context = context.for_installed_monitor()?;
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
mod tests;
