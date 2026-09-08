//! Authorized monitor preparation and bounded owner-loop probe scheduling.

use crate::background_process::ValidatedBackgroundEnvironment;
use crate::terminal_host_authority::CapturedTerminalHostAuthority;
use crate::terminal_monitor::{
    TerminalMonitorActivation, TerminalMonitorMutation, TerminalProbeFailure,
};
use crate::terminal_probe_effects::{
    AuthorizedTerminalProbe, NativeTerminalProbeExecutor, TerminalProbeAuthority,
    TerminalProbeClock, TerminalProbeCustomContext, TerminalProbeGrant, canonical_fingerprint,
    path_baseline,
};
use crate::terminal_registry::{TerminalRegistry, TerminalRegistryStep};
use crate::terminal_runtime::TerminalRuntimeRequester;
use crate::terminal_session::TerminalSessionBackend;
use crate::terminal_shell::TerminalShell;
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, TerminalLifecycle,
    TerminalMonitorCondition as Condition, TerminalMonitorDefinition, TerminalMonitorId,
    TerminalSessionId, ToolError, ToolErrorKind,
};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Instant;
#[cfg(test)]
use tests::ProbeDiagnostics;

const MAX_GRANTS: usize = 16 * 64;
const MAX_ACTIVE: usize = 16;
type Result<T> = std::result::Result<T, ToolError>;

/// Captured once on the existing effect worker; all monitor grants share the
/// immutable environment. PATH-selected shells are resolved during custom
/// preparation relative to its canonical cwd, independently of terminal profiles.
pub(crate) struct TerminalHostProbePreparer {
    host: Arc<CapturedTerminalHostAuthority>,
    environment: Arc<ValidatedBackgroundEnvironment>,
}

/// Prepared authority is distinct from the persisted definition. It becomes a
/// grant only when the owner supplies the successful live mutation receipt.
pub(crate) struct PreparedTerminalMonitor {
    authority: Option<TerminalProbeAuthority>,
    activation: TerminalMonitorActivation,
}
impl PreparedTerminalMonitor {
    pub(crate) const fn activation(&self) -> TerminalMonitorActivation {
        self.activation
    }
}

impl TerminalHostProbePreparer {
    pub(crate) fn new_on_worker(
        host: Arc<CapturedTerminalHostAuthority>,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        check(deadline, cancellation)?;
        let environment = Arc::new(
            ValidatedBackgroundEnvironment::new(host.environment_on_worker())
                .map_err(|_| invalid())?,
        );
        check(deadline, cancellation)?;
        Ok(Self { host, environment })
    }

    /// Call only after authorization of this exact action/definition. All DNS,
    /// path acquisition and activation observations remain off the owner loop.
    /// Indivisible system resolver/filesystem calls are checked before and after;
    /// they cannot extend the deadline for any subsequent probe effect.
    pub(crate) fn prepare_on_worker(
        &self,
        definition: &TerminalMonitorDefinition,
        session_cwd: &str,
        sandbox: Option<Arc<crate::NativeSandboxLaunch>>,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<PreparedTerminalMonitor> {
        definition.validate().map_err(|_| invalid())?;
        check(deadline, cancellation)?;
        let mut activation = TerminalMonitorActivation::default();
        let authority = match &definition.condition {
            Condition::TcpReady { host, port } => Some(TerminalProbeAuthority::Tcp {
                host: host.clone(),
                port: *port,
                addresses: resolve_addresses(host, *port, deadline, cancellation)?,
            }),
            Condition::HttpReady { url } => {
                let parsed = url::Url::parse(url).map_err(|_| invalid())?;
                let raw_authority = url
                    .split_once("://")
                    .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""));
                if parsed.scheme() != "http"
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || raw_authority.is_none_or(|value| value.contains('@'))
                {
                    return Err(invalid());
                }
                let host = parsed.host_str().ok_or_else(invalid)?;
                let port = parsed.port_or_known_default().ok_or_else(invalid)?;
                Some(TerminalProbeAuthority::Http {
                    url: url.clone(),
                    addresses: resolve_addresses(host, port, deadline, cancellation)?,
                })
            }
            Condition::PathExists { path }
            | Condition::PathChanged { path }
            | Condition::PathSize { path, .. } => {
                let path_resolved = monitor_path(session_cwd, path)?;
                let (parent, leaf) = self.resolve_path(
                    path_resolved.to_str().ok_or_else(invalid)?,
                    deadline,
                    cancellation,
                )?;
                if matches!(definition.condition, Condition::PathChanged { .. }) {
                    activation.path_baseline =
                        Some(path_baseline(&parent, &leaf).map_err(probe_error)?);
                }
                Some(TerminalProbeAuthority::Path {
                    path: path.clone(),
                    parent,
                    leaf,
                })
            }
            Condition::CustomProbe { command, cwd } => {
                let resolved = monitor_path(session_cwd, cwd)?;
                let (canonical, directory) = self.host.resolve_directory(
                    resolved.to_str().ok_or_else(invalid)?,
                    deadline,
                    cancellation,
                )?;
                let canonical_cwd = std::path::PathBuf::from(canonical);
                activation.cwd_sha256 = Some(
                    canonical_fingerprint(&canonical_cwd)
                        .map_err(probe_error)?
                        .1,
                );
                let shell = self.legacy_shell(&canonical_cwd, deadline, cancellation)?;
                let shell = match sandbox {
                    Some(sandbox) => shell.with_sandbox(sandbox),
                    None => shell,
                };
                let context = Arc::new(TerminalProbeCustomContext::with_environment(
                    shell,
                    Arc::clone(&self.environment),
                ));
                Some(TerminalProbeAuthority::Custom {
                    command: command.clone(),
                    cwd: cwd.clone(),
                    canonical_cwd,
                    directory,
                    context,
                })
            }
            _ => None,
        };
        check(deadline, cancellation)?;
        Ok(PreparedTerminalMonitor {
            authority,
            activation,
        })
    }

    fn legacy_shell(
        &self,
        cwd: &Path,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<TerminalShell> {
        let path = self
            .environment
            .entries()
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value)
            .ok_or_else(unavailable)?;
        for directory in std::env::split_paths(path) {
            check(deadline, cancellation)?;
            let candidate = cwd.join(directory).join("sh");
            if candidate.as_os_str().len() > 4096 {
                continue;
            }
            if std::fs::metadata(&candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            }) {
                check(deadline, cancellation)?;
                return TerminalShell::legacy_captured_sh(candidate).map_err(|_| invalid());
            }
        }
        Err(unavailable())
    }

    fn resolve_path(
        &self,
        raw: &str,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<(rustix::fd::OwnedFd, OsString)> {
        let mut current = PathBuf::from(raw);
        let mut missing = Vec::new();
        loop {
            check(deadline, cancellation)?;
            match std::fs::canonicalize(&current) {
                Ok(mut canonical) => {
                    if missing.is_empty()
                        && !std::fs::metadata(&canonical)
                            .map_err(|_| unavailable())?
                            .is_dir()
                    {
                        missing.push(canonical.file_name().ok_or_else(invalid)?.to_owned());
                        canonical.pop();
                    }
                    let (_, descriptor) = self.host.resolve_directory(
                        canonical.to_str().ok_or_else(invalid)?,
                        deadline,
                        cancellation,
                    )?;
                    let relative = if missing.is_empty() {
                        PathBuf::from(".")
                    } else {
                        missing.into_iter().rev().collect::<PathBuf>()
                    };
                    check(deadline, cancellation)?;
                    return Ok((descriptor, relative.into_os_string()));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // Never turn an unresolved symlink into future path authority.
                    if std::fs::symlink_metadata(&current)
                        .is_ok_and(|metadata| metadata.file_type().is_symlink())
                    {
                        return Err(invalid());
                    }
                    missing.push(current.file_name().ok_or_else(invalid)?.to_owned());
                    if !current.pop() {
                        return Err(invalid());
                    }
                }
                Err(_) => return Err(unavailable()),
            }
        }
    }
}

fn monitor_path(session_cwd: &str, raw: &str) -> Result<PathBuf> {
    if !Path::new(session_cwd).is_absolute()
        || session_cwd.len() > 4096
        || raw.is_empty()
        || raw.len() > 4096
        || raw.contains('\0')
        || session_cwd.contains('\0')
    {
        return Err(invalid());
    }
    let mut resolved = PathBuf::new();
    for component in Path::new(session_cwd).join(raw).components() {
        match component {
            Component::RootDir => resolved.push("/"),
            Component::Normal(component) => resolved.push(component),
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) => return Err(invalid()),
        }
    }
    if resolved.as_os_str().len() > 4096 {
        return Err(invalid());
    }
    Ok(resolved)
}

fn resolve_addresses(
    host: &str,
    port: u16,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<Vec<SocketAddr>> {
    check(deadline, cancellation)?;
    let host = host.trim_matches(['[', ']']);
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        (host, port)
            .to_socket_addrs()
            .map_err(|_| unavailable())?
            .take(4)
            .collect()
    };
    check(deadline, cancellation)?;
    if addresses.is_empty() || port == 0 {
        return Err(unavailable());
    }
    Ok(addresses)
}

struct GrantEntry {
    grant: Arc<TerminalProbeGrant>,
    generation: u64,
    paused: bool,
}
struct QueuedProbe {
    authorized: AuthorizedTerminalProbe,
    grant: Arc<TerminalProbeGrant>,
}

/// Worker-owned scheduler. Pending descriptions are bounded separately from the
/// sixteen active probe/publication futures. Polling never blocks the owner.
pub(crate) struct TerminalHostProbes {
    executor: Arc<NativeTerminalProbeExecutor>,
    stop: CancellationToken,
    grants: Vec<GrantEntry>,
    queued: VecDeque<QueuedProbe>,
    active: Vec<BoxFuture<'static, ()>>,
    housekeeping: Option<BoxFuture<'static, ()>>,
    next_housekeeping_ms: i64,
    closed: bool,
    #[cfg(test)]
    diagnostics: Option<Arc<tests::ProbeDiagnostics>>,
}
impl TerminalHostProbes {
    pub(crate) fn new(executor: Arc<NativeTerminalProbeExecutor>, stop: CancellationToken) -> Self {
        Self {
            executor,
            stop,
            grants: Vec::new(),
            queued: VecDeque::new(),
            active: Vec::new(),
            housekeeping: None,
            next_housekeeping_ms: 0,
            closed: false,
            #[cfg(test)]
            diagnostics: None,
        }
    }

    /// Must run in the same owner callback as the successful monitor mutation.
    /// A fresh preparation is required for Add/Update, including local conditions.
    pub(crate) fn install(
        &mut self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
        mutation: &TerminalMonitorMutation,
        prepared: PreparedTerminalMonitor,
    ) -> Result<()> {
        if self.closed || self.stop.is_cancelled() || mutation.removed || mutation.generation == 0 {
            return Err(invalid());
        }
        let previous = self.index(owner, session, &mutation.monitor_id);
        if previous.is_some_and(|index| self.grants[index].generation >= mutation.generation) {
            return Err(invalid());
        }
        if prepared.authority.is_some() && previous.is_none() && self.grants.len() == MAX_GRANTS {
            return Err(unavailable());
        }
        if prepared.authority.is_some() && previous.is_none() {
            let same_session = self
                .grants
                .iter()
                .filter(|entry| entry.grant.owner() == owner && entry.grant.session_id() == session)
                .count();
            if same_session == 64 || (same_session == 0 && self.namespaces().len() == 16) {
                return Err(unavailable());
            }
        }
        let grant = prepared
            .authority
            .map(|authority| {
                TerminalProbeGrant::new(
                    owner.clone(),
                    session.clone(),
                    mutation.monitor_id.clone(),
                    mutation.generation,
                    authority,
                )
                .map(Arc::new)
            })
            .transpose()
            .map_err(probe_error)?;
        if let Some(index) = previous {
            self.remove_index(index);
        }
        if let Some(grant) = grant {
            self.grants.push(GrantEntry {
                grant,
                generation: mutation.generation,
                paused: false,
            });
        }
        Ok(())
    }

    pub(crate) fn pause(
        &mut self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
        mutation: &TerminalMonitorMutation,
    ) -> Result<()> {
        if let Some(index) = self.index(owner, session, &mutation.monitor_id) {
            let entry = &mut self.grants[index];
            if mutation.removed
                || entry.paused
                || entry.generation.checked_add(1) != Some(mutation.generation)
            {
                return Err(invalid());
            }
            entry.grant.revoke();
            entry.generation = mutation.generation;
            entry.paused = true;
            self.remove_revoked_queue();
        }
        Ok(())
    }

    pub(crate) fn resume(
        &mut self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
        mutation: &TerminalMonitorMutation,
    ) -> Result<()> {
        if self.closed || self.stop.is_cancelled() {
            return Err(unavailable());
        }
        if let Some(index) = self.index(owner, session, &mutation.monitor_id) {
            let entry = &mut self.grants[index];
            if mutation.removed
                || !entry.paused
                || entry.generation.checked_add(1) != Some(mutation.generation)
            {
                return Err(invalid());
            }
            entry.grant = Arc::new(
                entry
                    .grant
                    .renew_generation(mutation.generation)
                    .map_err(probe_error)?,
            );
            entry.generation = mutation.generation;
            entry.paused = false;
        }
        Ok(())
    }

    pub(crate) fn remove(
        &mut self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
        mutation: &TerminalMonitorMutation,
    ) -> Result<()> {
        if !mutation.removed {
            return Err(invalid());
        }
        if let Some(index) = self.index(owner, session, &mutation.monitor_id) {
            if self.grants[index].generation != mutation.generation {
                return Err(invalid());
            }
            self.remove_index(index);
        }
        Ok(())
    }

    pub(crate) fn retire_session(
        &mut self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
    ) {
        self.grants.retain(|entry| {
            let keep = entry.grant.owner() != owner || entry.grant.session_id() != session;
            if !keep {
                entry.grant.revoke();
            }
            keep
        });
        self.remove_revoked_queue();
    }

    /// Owner-only metadata reconciliation immediately before new grant admission.
    /// Inspect every retained namespace: stale grants elsewhere also consume the
    /// global/namespace quotas. Paused templates keep their live generation.
    pub(crate) fn reconcile_live<B: TerminalSessionBackend>(
        &mut self,
        registry: &TerminalRegistry<B>,
    ) {
        self.reconcile_live_with(|owner, session| {
            registry.live_monitor_generations(owner, session).ok()
        });
    }

    fn reconcile_live_with(
        &mut self,
        mut live: impl FnMut(
            &BackgroundOutputOwner,
            &TerminalSessionId,
        ) -> Option<Vec<(TerminalMonitorId, u64)>>,
    ) {
        for (owner, session) in self.namespaces() {
            match live(&owner, &session) {
                Some(identities) => self.retain_session_monitors(&owner, &session, &identities),
                None => self.retire_session(&owner, &session),
            }
        }
    }

    /// Prune automatic expiration/until-match removal using a trusted live view,
    /// never persisted descriptions. Paused templates still count toward quota.
    pub(crate) fn retain_session_monitors(
        &mut self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
        live: &[(TerminalMonitorId, u64)],
    ) {
        self.grants.retain(|entry| {
            let keep = entry.grant.owner() != owner
                || entry.grant.session_id() != session
                || live.iter().any(|(id, generation)| {
                    id == entry.grant.monitor_id() && *generation == entry.generation
                });
            if !keep {
                entry.grant.revoke();
            }
            keep
        });
        self.remove_revoked_queue();
    }

    pub(crate) fn shutdown(&mut self) {
        self.stop.cancel();
        self.closed = true;
        for entry in &self.grants {
            entry.grant.revoke();
        }
        self.grants.clear();
        self.queued.clear();
        self.active.clear();
        self.housekeeping = None;
    }

    /// Called on every owner pump, including empty steps. Completion requests are
    /// non-owning and acquire the registry's exact profile transaction themselves.
    pub(crate) fn observe<B: TerminalSessionBackend + Send + 'static, S: 'static>(
        &mut self,
        steps: Vec<TerminalRegistryStep>,
        clock: TerminalProbeClock,
        requester: &TerminalRuntimeRequester<B, S>,
        access: fn(&mut S) -> &mut Self,
    ) {
        if self.closed || self.stop.is_cancelled() {
            self.shutdown();
            return;
        }
        for step in steps {
            let Ok(result) = step.result else {
                continue;
            };
            if !matches!(
                result.lifecycle,
                TerminalLifecycle::Starting | TerminalLifecycle::Running
            ) {
                self.retire_session(&step.owner, &step.session_id);
                continue;
            }
            for request in result.probes {
                if self.queued.len() == MAX_GRANTS {
                    break;
                }
                let Some(index) = self.index(&step.owner, &step.session_id, &request.monitor_id)
                else {
                    continue;
                };
                let entry = &self.grants[index];
                if entry.paused || entry.grant.is_revoked() {
                    continue;
                }
                let grant = Arc::clone(&entry.grant);
                #[cfg(test)]
                if let Some(trace) = &self.diagnostics {
                    trace.scheduled(&request, clock.now_ms);
                }
                if let Ok(authorized) =
                    AuthorizedTerminalProbe::new(&step.owner, request, Arc::clone(&grant), clock)
                {
                    self.queued
                        .retain(|queued| !Arc::ptr_eq(&queued.grant, &grant));
                    self.queued.push_back(QueuedProbe { authorized, grant });
                }
            }
        }
        while self.active.len() < MAX_ACTIVE {
            let Some(queued) = self.queued.pop_front() else {
                break;
            };
            if queued.grant.is_revoked() {
                continue;
            }
            let future = self.executor.execute(queued.authorized, self.stop.clone());
            let requester = requester.clone();
            let stop = self.stop.clone();
            #[cfg(test)]
            let trace = self.diagnostics.clone();
            self.active.push(Box::pin(async move {
                let evidence = future.await;
                #[cfg(test)]
                tests::ProbeDiagnostics::evidence(trace.as_deref(), &evidence);
                if stop.is_cancelled() || queued.grant.is_revoked() {
                    #[cfg(test)]
                    tests::ProbeDiagnostics::discarded(trace.as_deref(), "evidence");
                    return;
                }
                #[cfg(test)]
                let owner_log = trace.clone();
                let publication = requester
                    .request_with_context(stop.clone(), move |context| {
                        if stop.is_cancelled() || queued.grant.is_revoked() {
                            #[cfg(test)]
                            ProbeDiagnostics::discarded(owner_log.as_deref(), "publication");
                            return;
                        }
                        #[cfg(test)]
                        let sequence = evidence.request_sequence;
                        let result = context.registry.mutate_with_profile(
                            context.store,
                            context.budget,
                            queued.grant.owner(),
                            queued.grant.session_id(),
                            |session, persistence| {
                                session.complete_probe_with(persistence, evidence, context.now_ms)
                            },
                        );
                        #[cfg(test)]
                        if let Some(trace) = &owner_log {
                            trace.publication(sequence, context.now_ms, &result);
                        }
                        let _ = result;
                    })
                    .await;
                #[cfg(test)]
                tests::ProbeDiagnostics::request_completed(trace.as_deref(), &publication);
                let _ = publication;
            }));
        }
        self.poll_active();
        self.poll_housekeeping(clock.now_ms, requester, access);
        #[cfg(test)]
        if let Some(trace) = &self.diagnostics {
            trace.counts((self.grants.len(), self.active.len(), self.queued.len()));
        }
    }

    fn namespaces(&self) -> Vec<(BackgroundOutputOwner, TerminalSessionId)> {
        let mut namespaces = Vec::new();
        for entry in &self.grants {
            if !namespaces.iter().any(|(owner, session)| {
                owner == entry.grant.owner() && session == entry.grant.session_id()
            }) {
                namespaces.push((
                    entry.grant.owner().clone(),
                    entry.grant.session_id().clone(),
                ));
            }
        }
        namespaces
    }

    fn poll_housekeeping<B: TerminalSessionBackend + Send + 'static, S: 'static>(
        &mut self,
        now_ms: i64,
        requester: &TerminalRuntimeRequester<B, S>,
        access: fn(&mut S) -> &mut Self,
    ) {
        let mut context = Context::from_waker(Waker::noop());
        if let Some(future) = self.housekeeping.as_mut() {
            if future.as_mut().poll(&mut context).is_ready() {
                self.housekeeping = None;
            }
            return;
        }
        if self.housekeeping.is_none()
            && !self.grants.is_empty()
            && now_ms >= self.next_housekeeping_ms
        {
            let namespaces = self.namespaces();
            let stop = self.stop.clone();
            let requester = requester.clone();
            self.next_housekeeping_ms = now_ms.saturating_add(250);
            self.housekeeping = Some(Box::pin(async move {
                let _ = requester
                    .request_with_context(stop.clone(), move |context| {
                        if stop.is_cancelled() {
                            return;
                        }
                        for (owner, session) in namespaces {
                            if let Ok(live) =
                                context.registry.live_monitor_generations(&owner, &session)
                            {
                                access(context.state)
                                    .retain_session_monitors(&owner, &session, &live);
                            } else {
                                access(context.state).retire_session(&owner, &session);
                            }
                        }
                    })
                    .await;
            }));
            if self
                .housekeeping
                .as_mut()
                .is_some_and(|future| future.as_mut().poll(&mut context).is_ready())
            {
                self.housekeeping = None;
            }
        }
    }

    fn poll_active(&mut self) {
        let mut context = Context::from_waker(Waker::noop());
        let mut index = 0;
        while index < self.active.len() {
            if matches!(
                self.active[index].as_mut().poll(&mut context),
                Poll::Ready(())
            ) {
                drop(self.active.swap_remove(index));
            } else {
                index += 1;
            }
        }
    }
    fn index(
        &self,
        owner: &BackgroundOutputOwner,
        session: &TerminalSessionId,
        monitor: &TerminalMonitorId,
    ) -> Option<usize> {
        self.grants.iter().position(|entry| {
            entry.grant.owner() == owner
                && entry.grant.session_id() == session
                && entry.grant.monitor_id() == monitor
        })
    }
    fn remove_index(&mut self, index: usize) {
        self.grants.swap_remove(index).grant.revoke();
        self.remove_revoked_queue();
    }
    fn remove_revoked_queue(&mut self) {
        self.queued.retain(|queued| !queued.grant.is_revoked());
    }
}
impl Drop for TerminalHostProbes {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn check(deadline: Instant, cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(ToolError::new(
            ToolErrorKind::Cancelled,
            "terminal_cancelled",
            "terminal monitor preparation cancelled",
            false,
        ));
    }
    if Instant::now() >= deadline {
        return Err(unavailable());
    }
    Ok(())
}
fn probe_error(_: TerminalProbeFailure) -> ToolError {
    invalid()
}
fn invalid() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "terminal_invalid_probe_authority",
        "invalid terminal monitor authority",
        false,
    )
}
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "terminal_probe_authority_unavailable",
        "terminal monitor authority unavailable",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NativeOwnedWorkerSpawner;
    use crate::background_input::{BackgroundInputReceipt, BackgroundInputStatus};
    use crate::terminal_captured_exec::TerminalCapturedExec;
    use crate::terminal_history::TerminalHistory;
    use crate::terminal_host_authority::{
        TerminalHostAccountShell, TerminalHostAuthority, TerminalHostAuthorityInputs,
    };
    use crate::terminal_journal::TerminalJournalLimits;
    use crate::terminal_profile::{
        TerminalProfileBudget, TerminalProfileLimits, TerminalProfileMutationContext,
    };
    use crate::terminal_profile_store::TerminalProfileStore;
    use crate::terminal_pty::{TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
    use crate::terminal_registry::TerminalRegistry;
    use crate::terminal_runtime::{TerminalRuntime, TerminalRuntimeWorker};
    use crate::terminal_session::TerminalSession;
    use machine_god_core::{
        SessionId, SessionIncarnationId, TerminalDimensions, TerminalMonitorLifetime,
        TerminalMonitorOperation, TerminalNotifySchedule, TerminalSchedule, TerminalSignal,
    };
    use rustix::fd::OwnedFd;
    use rustix::fs::{Mode, OFlags};
    use std::fs::DirBuilder;
    use std::os::unix::fs::{DirBuilderExt, symlink};
    use std::path::PathBuf;
    use std::sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    /// Test-local metadata only: no command/output payloads or owner requests.
    #[derive(Default, Debug)]
    pub(super) struct ProbeDiagnostics {
        state: Mutex<((usize, usize, usize), VecDeque<String>)>,
    }
    impl ProbeDiagnostics {
        pub(super) fn scheduled(
            &self,
            request: &crate::terminal_monitor::TerminalProbeRequest,
            now: i64,
        ) {
            self.record(format!(
                "scheduled sequence={} started={} deadline={} observed={now}",
                request.request_sequence, request.started_at_ms, request.deadline_ms,
            ));
        }
        pub(super) fn evidence(
            trace: Option<&Self>,
            evidence: &crate::terminal_monitor::TerminalProbeEvidence,
        ) {
            let Some(trace) = trace else {
                return;
            };
            trace.record(format!(
                "evidence sequence={} completed={} timed_out={} truncated={} bytes={} result={}",
                evidence.request_sequence,
                evidence.completed_at_ms,
                evidence.timed_out,
                evidence.truncated,
                evidence.output_bytes,
                match &evidence.result {
                    Ok(crate::terminal_monitor::TerminalProbeObservation::Custom {
                        exit_code,
                        ..
                    }) => format!("custom exit {exit_code}"),
                    Ok(_) => "non-custom observation".into(),
                    Err(error) => format!("{error:?}"),
                },
            ));
        }
        pub(super) fn discarded(trace: Option<&Self>, phase: &str) {
            if let Some(trace) = trace {
                trace.record(format!("{phase} discarded: stopped/revoked"));
            }
        }
        pub(super) fn publication(&self, sequence: u64, now: i64, result: &impl std::fmt::Debug) {
            self.record(format!(
                "publication sequence={sequence} now={now} result={result:?}"
            ));
        }
        pub(super) fn request_completed(trace: Option<&Self>, result: &impl std::fmt::Debug) {
            if let Some(trace) = trace {
                trace.record(format!("request completed: {result:?}"));
            }
        }
        pub(super) fn record(&self, mut event: String) {
            event.truncate(event.floor_char_boundary(512));
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.1.len() == 16 {
                state.1.pop_front();
            }
            state.1.push_back(event);
        }
        pub(super) fn counts(&self, counts: (usize, usize, usize)) {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0 = counts;
        }
    }

    fn owner() -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("probe-owner").unwrap(),
            SessionIncarnationId::new("probe-incarnation").unwrap(),
        )
    }
    fn session_id() -> TerminalSessionId {
        TerminalSessionId::new("probe-host-session").unwrap()
    }
    fn open(path: &Path) -> OwnedFd {
        rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap()
    }
    fn definition(condition: Condition) -> TerminalMonitorDefinition {
        TerminalMonitorDefinition {
            check_schedule: condition
                .requires_polling()
                .then_some(TerminalSchedule { interval_ms: 20 }),
            condition,
            notify: TerminalNotifySchedule::OnMatch,
            lifetime: TerminalMonitorLifetime::UntilMatch,
        }
    }
    fn mutation(generation: u64, removed: bool) -> TerminalMonitorMutation {
        TerminalMonitorMutation {
            monitor_id: TerminalMonitorId::new("monitor-test").unwrap(),
            generation,
            removed,
        }
    }
    struct Fixture {
        path: PathBuf,
        preparer: Arc<TerminalHostProbePreparer>,
        executor: Arc<NativeTerminalProbeExecutor>,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-host-probes-{:032x}",
                u128::from_le_bytes(random)
            ));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            let path = std::fs::canonicalize(path).unwrap();
            for child in ["workspace", "artifacts", "profile", "outside"] {
                DirBuilder::new()
                    .mode(0o700)
                    .create(path.join(child))
                    .unwrap();
            }
            let (program, arguments) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY")
                .map_or_else(
                    || {
                        (
                            std::env::current_exe().unwrap(),
                            vec![
                                "--ignored".into(),
                                "--exact".into(),
                                "terminal_captured_exec::tests::captured_helper_child".into(),
                                "--nocapture".into(),
                            ],
                        )
                    },
                    |path| {
                        (
                            PathBuf::from(path),
                            vec![crate::TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
                        )
                    },
                );
            let inputs = TerminalHostAuthorityInputs {
                workspace: open(&path.join("workspace")),
                workspace_path: path.join("workspace"),
                default_cwd: path.join("workspace"),
                environment: vec![("PATH".into(), "/usr/bin:/bin".into())],
                account_shell: TerminalHostAccountShell::Explicit(Some("/bin/bash".into())),
                cli_executable: program.clone(),
                tmux_executable: None,
                artifacts: open(&path.join("artifacts")),
                artifact_path: path.join("artifacts"),
            };
            let preparer =
                futures_executor::block_on(NativeOwnedWorkerSpawner::new().run(move || {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    let host = TerminalHostAuthority::new(inputs)
                        .unwrap()
                        .capture_on_worker(deadline, &CancellationToken::new())
                        .unwrap();
                    TerminalHostProbePreparer::new_on_worker(
                        Arc::new(host),
                        deadline,
                        &CancellationToken::new(),
                    )
                    .unwrap()
                }))
                .unwrap();
            let captured = Arc::new(
                TerminalCapturedExec::new(program, arguments, Duration::from_secs(10), 16)
                    .unwrap()
                    .with_test_inventory_helper(),
            );
            Self {
                path,
                preparer: Arc::new(preparer),
                executor: Arc::new(NativeTerminalProbeExecutor::new(captured, 16).unwrap()),
            }
        }
        fn prepare(&self, definition: &TerminalMonitorDefinition) -> PreparedTerminalMonitor {
            self.prepare_at(definition, self.path.join("workspace"))
        }
        fn prepare_at(
            &self,
            definition: &TerminalMonitorDefinition,
            session_cwd: PathBuf,
        ) -> PreparedTerminalMonitor {
            let definition = definition.clone();
            let preparer = Arc::clone(&self.preparer);
            futures_executor::block_on(NativeOwnedWorkerSpawner::new().run(move || {
                preparer
                    .prepare_on_worker(
                        &definition,
                        session_cwd.to_str().unwrap(),
                        None,
                        Instant::now() + Duration::from_secs(5),
                        &CancellationToken::new(),
                    )
                    .unwrap()
            }))
            .unwrap()
        }
        fn runtime(
            &self,
            definition: TerminalMonitorDefinition,
            prepared: Option<PreparedTerminalMonitor>,
        ) -> Harness {
            self.runtime_at(
                definition,
                prepared,
                self.path.join("workspace"),
                "/bin/bash",
                machine_god_core::TerminalProfile::Clean,
            )
        }
        #[allow(
            clippy::too_many_lines,
            reason = "complete real-runtime profile fixture setup"
        )]
        fn runtime_at(
            &self,
            definition: TerminalMonitorDefinition,
            prepared: Option<PreparedTerminalMonitor>,
            cwd: PathBuf,
            shell: &str,
            profile: machine_god_core::TerminalProfile,
        ) -> Harness {
            let path = self.path.clone();
            let shell = shell.to_owned();
            let workspace = path.join("workspace").to_str().unwrap().to_owned();
            let executor = Arc::clone(&self.executor);
            let stop = CancellationToken::new();
            let worker_stop = stop.clone();
            let done = Arc::new(AtomicBool::new(false));
            let worker_done = Arc::clone(&done);
            let diagnostics = Arc::new(ProbeDiagnostics::default());
            let worker_diagnostics = Arc::clone(&diagnostics);
            let label = format!("shell={shell} profile={profile:?} cwd={cwd:?}");
            let requester = Arc::new(OnceLock::<TerminalRuntimeRequester<Backend, State>>::new());
            let worker_requester = Arc::clone(&requester);
            let runtime = TerminalRuntime::new(
                move || {
                    let started = Instant::now();
                    let store = TerminalProfileStore::prepare(open(&path.join("profile"))).unwrap();
                    let budget =
                        TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
                    let mut registry = TerminalRegistry::new(workspace.clone()).unwrap();
                    let mut transaction = store.transaction().unwrap();
                    let mut catalog = transaction
                        .prepare_catalog(workspace.clone(), owner())
                        .unwrap();
                    drop(
                        transaction
                            .create_session(&mut catalog, &session_id())
                            .unwrap(),
                    );
                    let namespace = catalog.namespace_key();
                    let created = budget
                        .create_journal(
                            &mut transaction,
                            namespace,
                            &session_id(),
                            TerminalJournalLimits::default(),
                        )
                        .unwrap();
                    created.accounting.unwrap();
                    let mut persistence =
                        TerminalProfileMutationContext::new(&mut transaction, budget, namespace);
                    let history = TerminalHistory::create_with(
                        &mut persistence,
                        created.operation.unwrap(),
                        &TerminalDimensions::new(3, 20).unwrap(),
                    )
                    .unwrap();
                    let mut metadata = crate::terminal_session_record::test_metadata();
                    metadata.workspace = workspace;
                    metadata.cwd = cwd.to_str().unwrap().into();
                    metadata.shell = shell;
                    metadata.profile = profile;
                    let mut session = TerminalSession::new_with(
                        &mut persistence,
                        Backend(false),
                        history,
                        owner(),
                        session_id(),
                        metadata,
                        0,
                    )
                    .unwrap();
                    session.shell_ready_with(&mut persistence, 0).unwrap();
                    let activation = prepared.as_ref().map_or_else(
                        TerminalMonitorActivation::default,
                        PreparedTerminalMonitor::activation,
                    );
                    let mutation = session
                        .monitor_with(
                            &mut persistence,
                            &owner(),
                            TerminalMonitorOperation::Add { definition },
                            activation,
                            0,
                        )
                        .unwrap();
                    registry
                        .start(owner(), session_id(), || Ok(session))
                        .unwrap();
                    drop(transaction);
                    let mut probes = TerminalHostProbes::new(executor, worker_stop);
                    probes.diagnostics = Some(worker_diagnostics);
                    if let Some(prepared) = prepared {
                        probes
                            .install(&owner(), &session_id(), &mutation, prepared)
                            .unwrap();
                    }
                    Ok(TerminalRuntimeWorker::new_with_state(
                        registry,
                        store,
                        budget,
                        State {
                            probes,
                            done: worker_done,
                        },
                        move || i64::try_from(started.elapsed().as_millis()).unwrap(),
                        move |state, steps| {
                            state.probes.observe(
                                steps,
                                TerminalProbeClock {
                                    now_ms: i64::try_from(started.elapsed().as_millis()).unwrap(),
                                    observed_at: Instant::now(),
                                },
                                worker_requester.get().unwrap(),
                                access,
                            );
                        },
                    ))
                },
                Arc::new(NativeOwnedWorkerSpawner::new()),
            );
            assert!(requester.set(runtime.requester()).is_ok());
            futures_executor::block_on(
                runtime.request_with_context(CancellationToken::new(), |_| ()),
            )
            .unwrap();
            Harness {
                runtime: Some(runtime),
                stop,
                done,
                diagnostics,
                label,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    struct State {
        probes: TerminalHostProbes,
        done: Arc<AtomicBool>,
    }
    impl Drop for State {
        fn drop(&mut self) {
            if let Some(trace) = &self.probes.diagnostics {
                trace.record("state drop: shutdown begins".into());
            }
            self.probes.shutdown();
            self.done.store(true, Ordering::Release);
            if let Some(trace) = &self.probes.diagnostics {
                trace.record("state drop: shutdown done".into());
            }
        }
    }
    fn access(state: &mut State) -> &mut TerminalHostProbes {
        &mut state.probes
    }
    struct Harness {
        runtime: Option<TerminalRuntime<Backend, State>>,
        stop: CancellationToken,
        done: Arc<AtomicBool>,
        diagnostics: Arc<ProbeDiagnostics>,
        label: String,
    }
    impl Harness {
        fn runtime(&self) -> &TerminalRuntime<Backend, State> {
            self.runtime.as_ref().unwrap()
        }
        fn counts(&self) -> (usize, usize, usize) {
            futures_executor::block_on(self.runtime().request_with_context(
                CancellationToken::new(),
                |context| {
                    (
                        context.state.probes.grants.len(),
                        context.state.probes.active.len(),
                        context.state.probes.queued.len(),
                    )
                },
            ))
            .unwrap()
        }
        #[track_caller]
        fn until(condition: impl FnMut() -> bool) {
            Self::until_described("condition", condition, String::new);
        }
        #[track_caller]
        fn until_described(
            phase: &str,
            mut condition: impl FnMut() -> bool,
            details: impl FnOnce() -> String,
        ) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !condition() {
                assert!(
                    Instant::now() < deadline,
                    "probe fixture wait expired: phase={phase}; {}",
                    details()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        #[track_caller]
        fn until_probe(&self, phase: &str, cwd: &Path, condition: impl FnMut() -> bool) {
            Self::until_described(phase, condition, || {
                use std::io::Read;
                let flags = std::fs::File::open(cwd.join("shell-flags")).and_then(|file| {
                    let mut bytes = Vec::new();
                    file.take(64).read_to_end(&mut bytes).map(|_| bytes)
                });
                format!(
                    "{} observed={} shell_flags={flags:?} stop={} done={} trace={:?}",
                    self.label,
                    cwd.join("observed").exists(),
                    self.stop.is_cancelled(),
                    self.done.load(Ordering::Acquire),
                    self.diagnostics,
                )
            });
        }
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.stop.cancel();
            drop(self.runtime.take());
            Self::until_described(
                "harness shutdown",
                || self.done.load(Ordering::Acquire),
                || format!("{} trace={:?}", self.label, self.diagnostics),
            );
        }
    }
    struct Backend(bool);
    impl TerminalSessionBackend for Backend {
        fn read(&mut self, _: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            Ok(TerminalPtyRead {
                bytes_read: 0,
                closed: false,
            })
        }
        fn write(&mut self, _: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            Ok(BackgroundInputReceipt::new(
                0,
                false,
                BackgroundInputStatus::Backpressure,
            ))
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            Ok(if self.0 {
                TerminalPtyStatus::Exited(0)
            } else {
                TerminalPtyStatus::Running
            })
        }
        fn resize(&mut self, _: &TerminalDimensions) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal(&mut self, _: TerminalSignal) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal_may_discard_output(&self) -> bool {
            false
        }
        fn close(
            &mut self,
            _: bool,
            _: &mut dyn FnMut(&[u8]),
        ) -> std::result::Result<TerminalPtyClose, ()> {
            self.0 = true;
            Ok(TerminalPtyClose {
                status: TerminalPtyStatus::Exited(0),
                output_incomplete: false,
            })
        }
    }

    #[test]
    fn worker_preparation_preserves_native_paths_baselines_and_explicit_deadlines() {
        let fixture = Fixture::new();
        let root = fixture.path.join("workspace");
        std::fs::create_dir_all(root.join("real/deep")).unwrap();
        symlink("real/deep", root.join("link")).unwrap();
        std::fs::write(root.join("file"), b"hello").unwrap();
        std::fs::write(root.join("real/file"), b"not-lexical").unwrap();
        let prepared = fixture.prepare(&definition(Condition::PathChanged {
            path: "link/../file".into(),
        }));
        assert_eq!(prepared.activation().path_baseline.unwrap().size, 5);
        for path in [".", "real/.."] {
            let prepared =
                fixture.prepare(&definition(Condition::PathExists { path: path.into() }));
            let grant = TerminalProbeGrant::new(
                owner(),
                session_id(),
                mutation(1, false).monitor_id,
                1,
                prepared.authority.unwrap(),
            )
            .unwrap();
            assert!(matches!(
                grant.target(),
                crate::terminal_monitor::TerminalProbeTarget::Path { .. }
            ));
        }
        let custom = fixture.prepare(&definition(Condition::CustomProbe {
            command: "printf forbidden > forbidden".into(),
            cwd: "link/..".into(),
        }));
        assert_eq!(
            custom.activation().cwd_sha256,
            Some(canonical_fingerprint(&root).unwrap().1)
        );
        assert!(!root.join("real/forbidden").exists());
        for raw in ["../outside/file", "../../outside/file"] {
            assert!(
                fixture
                    .preparer
                    .prepare_on_worker(
                        &definition(Condition::PathExists { path: raw.into() }),
                        root.to_str().unwrap(),
                        None,
                        Instant::now() + Duration::from_secs(1),
                        &CancellationToken::new()
                    )
                    .is_err()
            );
        }
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        for (deadline, token) in [
            (Instant::now(), CancellationToken::new()),
            (Instant::now() + Duration::from_secs(1), cancelled),
        ] {
            assert!(
                fixture
                    .preparer
                    .prepare_on_worker(
                        &definition(Condition::TcpReady {
                            host: "localhost".into(),
                            port: 80
                        }),
                        root.to_str().unwrap(),
                        None,
                        deadline,
                        &token
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn grants_pause_resume_replace_prune_and_shutdown_without_reacquiring_paths() {
        let fixture = Fixture::new();
        let stop = CancellationToken::new();
        let mut scheduler = TerminalHostProbes::new(Arc::clone(&fixture.executor), stop);
        let definition = definition(Condition::PathExists {
            path: "missing".into(),
        });
        scheduler
            .install(
                &owner(),
                &session_id(),
                &mutation(1, false),
                fixture.prepare(&definition),
            )
            .unwrap();
        let original = Arc::clone(&scheduler.grants[0].grant);
        scheduler
            .pause(&owner(), &session_id(), &mutation(2, false))
            .unwrap();
        assert!(original.is_revoked());
        assert!(
            scheduler
                .resume(&owner(), &session_id(), &mutation(2, false))
                .is_err()
        );
        scheduler
            .resume(&owner(), &session_id(), &mutation(3, false))
            .unwrap();
        assert!(!scheduler.grants[0].grant.is_revoked());
        assert!(original.is_revoked());
        assert!(
            scheduler
                .install(
                    &owner(),
                    &session_id(),
                    &mutation(3, false),
                    fixture.prepare(&definition)
                )
                .is_err()
        );
        let resumed = Arc::clone(&scheduler.grants[0].grant);
        scheduler
            .install(
                &owner(),
                &session_id(),
                &mutation(4, false),
                fixture.prepare(&definition),
            )
            .unwrap();
        assert!(resumed.is_revoked());
        scheduler.retain_session_monitors(&owner(), &session_id(), &[]);
        assert!(scheduler.grants.is_empty());
        scheduler
            .install(
                &owner(),
                &session_id(),
                &mutation(5, false),
                fixture.prepare(&definition),
            )
            .unwrap();
        scheduler
            .remove(&owner(), &session_id(), &mutation(5, true))
            .unwrap();
        assert!(scheduler.grants.is_empty());
        scheduler.shutdown();
        assert!(
            scheduler
                .install(
                    &owner(),
                    &session_id(),
                    &mutation(6, false),
                    fixture.prepare(&definition)
                )
                .is_err()
        );
    }

    fn inert_scheduler() -> TerminalHostProbes {
        let captured = Arc::new(
            TerminalCapturedExec::new(
                "/never-executed-probe-helper".into(),
                Vec::new(),
                Duration::from_secs(2),
                16,
            )
            .unwrap(),
        );
        TerminalHostProbes::new(
            Arc::new(NativeTerminalProbeExecutor::new(captured, 16).unwrap()),
            CancellationToken::new(),
        )
    }
    fn prepared_tcp() -> PreparedTerminalMonitor {
        PreparedTerminalMonitor {
            authority: Some(TerminalProbeAuthority::Tcp {
                host: "127.0.0.1".into(),
                port: 80,
                addresses: vec!["127.0.0.1:80".parse().unwrap()],
            }),
            activation: TerminalMonitorActivation::default(),
        }
    }
    fn monitor_context(now_ms: i64) -> crate::terminal_monitor::TerminalMonitorContext {
        crate::terminal_monitor::TerminalMonitorContext {
            now_ms,
            cursor: machine_god_core::TerminalCursor::new(1, 0).unwrap(),
            lifecycle: TerminalLifecycle::Running,
        }
    }
    fn tcp_definition() -> TerminalMonitorDefinition {
        definition(Condition::TcpReady {
            host: "127.0.0.1".into(),
            port: 80,
        })
    }

    fn fill_monitor_grants(
        monitors: &mut crate::terminal_monitor::TerminalMonitorSet,
        scheduler: &mut TerminalHostProbes,
    ) -> Vec<TerminalMonitorMutation> {
        let mut mutations = Vec::new();
        for _ in 0..64 {
            let mutation = monitors
                .apply_with_activation(
                    TerminalMonitorOperation::Add {
                        definition: tcp_definition(),
                    },
                    TerminalMonitorActivation::default(),
                    monitor_context(0),
                )
                .unwrap();
            scheduler
                .install(&owner(), &session_id(), &mutation, prepared_tcp())
                .unwrap();
            mutations.push(mutation);
        }
        mutations
    }

    #[test]
    fn admission_reconciles_until_match_before_add_and_preserves_paused_grants() {
        use crate::terminal_monitor::{
            TerminalMonitorSet, TerminalProbeEvidence, TerminalProbeObservation,
        };
        let mut monitors = TerminalMonitorSet::new(session_id(), monitor_context(0)).unwrap();
        let mut scheduler = inert_scheduler();
        let mutations = fill_monitor_grants(&mut monitors, &mut scheduler);
        let retired = Arc::clone(&scheduler.grants[0].grant);
        let unrelated = Arc::clone(&scheduler.grants[2].grant);
        let paused = monitors
            .apply_with_activation(
                TerminalMonitorOperation::Pause {
                    monitor_id: mutations[1].monitor_id.clone(),
                },
                TerminalMonitorActivation::default(),
                monitor_context(0),
            )
            .unwrap();
        scheduler.pause(&owner(), &session_id(), &paused).unwrap();
        let request = monitors.tick(monitor_context(20)).unwrap().remove(0);
        assert!(
            monitors
                .complete_probe(
                    TerminalProbeEvidence {
                        session_id: request.session_id,
                        monitor_id: request.monitor_id,
                        generation: request.generation,
                        request_sequence: request.request_sequence,
                        completed_at_ms: 20,
                        output_bytes: 0,
                        truncated: false,
                        timed_out: false,
                        result: Ok(TerminalProbeObservation::Tcp { connected: true }),
                    },
                    monitor_context(20)
                )
                .unwrap()
        );
        assert_eq!(monitors.live_generations().len(), 63);
        assert_eq!(scheduler.grants.len(), 64, "housekeeping has not run");
        scheduler.reconcile_live_with(|who, session| {
            assert_eq!((who, session), (&owner(), &session_id()));
            Some(monitors.live_generations())
        });
        let added = monitors
            .apply_with_activation(
                TerminalMonitorOperation::Add {
                    definition: tcp_definition(),
                },
                TerminalMonitorActivation::default(),
                monitor_context(20),
            )
            .unwrap();
        scheduler
            .install(&owner(), &session_id(), &added, prepared_tcp())
            .unwrap();
        assert_eq!(scheduler.grants.len(), 64);
        assert!(retired.is_revoked());
        assert!(!unrelated.is_revoked());
        let paused_entry = &scheduler.grants[scheduler
            .index(&owner(), &session_id(), &paused.monitor_id)
            .unwrap()];
        assert!(paused_entry.paused && paused_entry.generation == paused.generation);
        let resumed = monitors
            .apply_with_activation(
                TerminalMonitorOperation::Resume {
                    monitor_id: paused.monitor_id,
                },
                TerminalMonitorActivation::default(),
                monitor_context(20),
            )
            .unwrap();
        scheduler.resume(&owner(), &session_id(), &resumed).unwrap();
        assert!(
            !scheduler.grants[scheduler
                .index(&owner(), &session_id(), &resumed.monitor_id)
                .unwrap()]
            .grant
            .is_revoked()
        );
        assert!(
            scheduler
                .install(&owner(), &session_id(), &mutation(1, false), prepared_tcp())
                .is_err()
        );
        assert_eq!(scheduler.grants.len(), 64, "live quota is unchanged");
        assert!(!unrelated.is_revoked());
    }

    #[test]
    fn admission_reconciles_closed_namespace_before_global_quota_and_slot_reuse() {
        let mut scheduler = inert_scheduler();
        let sessions: Vec<_> = (0..16)
            .map(|n| TerminalSessionId::new(format!("session-{n}")).unwrap())
            .collect();
        let identities: Vec<_> = (0..64)
            .map(|n| (TerminalMonitorId::new(format!("monitor-{n}")).unwrap(), 1))
            .collect();
        for session in &sessions {
            for (id, generation) in &identities {
                let mutation = TerminalMonitorMutation {
                    monitor_id: id.clone(),
                    generation: *generation,
                    removed: false,
                };
                scheduler
                    .install(&owner(), session, &mutation, prepared_tcp())
                    .unwrap();
            }
        }
        let retired = Arc::clone(&scheduler.grants[0].grant);
        let evicted = Arc::clone(&scheduler.grants[64].grant);
        let unrelated = Arc::clone(&scheduler.grants[128].grant);
        let replacement = TerminalSessionId::new("replacement").unwrap();
        assert_eq!(scheduler.grants.len(), MAX_GRANTS);
        assert!(
            scheduler
                .install(&owner(), &replacement, &mutation(1, false), prepared_tcp())
                .is_err()
        );
        let mut inspected = 0;
        scheduler.reconcile_live_with(|who, session| {
            assert_eq!(who, &owner());
            inspected += 1;
            // A closed live session has no live monitors; an evicted namespace
            // is absent. Both must revoke only its exact retained grants.
            if session == &sessions[0] {
                Some(Vec::new())
            } else if session == &sessions[1] {
                None
            } else {
                Some(identities.clone())
            }
        });
        assert_eq!(inspected, 16);
        assert!(retired.is_revoked() && evicted.is_revoked());
        assert!(!unrelated.is_revoked());
        assert_eq!(scheduler.grants.len(), 14 * 64);
        scheduler
            .install(&owner(), &replacement, &mutation(1, false), prepared_tcp())
            .unwrap();
        assert_eq!(scheduler.namespaces().len(), 15);
        scheduler
            .install(&owner(), &sessions[0], &mutation(2, false), prepared_tcp())
            .unwrap();
        assert_eq!(scheduler.namespaces().len(), 16);
        assert!(
            scheduler
                .install(&owner(), &sessions[1], &mutation(2, false), prepared_tcp())
                .is_err()
        );
        assert!(
            scheduler
                .grants
                .iter()
                .all(|entry| !entry.grant.is_revoked())
        );
        assert!(
            scheduler
                .grants
                .iter()
                .any(|entry| entry.grant.session_id() == &sessions[2])
        );
    }

    #[test]
    fn grant_admission_bounds_namespaces_and_per_session_monitors() {
        let fixture = Fixture::new();
        let mut scheduler =
            TerminalHostProbes::new(Arc::clone(&fixture.executor), CancellationToken::new());
        let prepared = || PreparedTerminalMonitor {
            authority: Some(TerminalProbeAuthority::Tcp {
                host: "127.0.0.1".into(),
                port: 80,
                addresses: vec!["127.0.0.1:80".parse().unwrap()],
            }),
            activation: TerminalMonitorActivation::default(),
        };
        for index in 0..16 {
            scheduler
                .install(
                    &owner(),
                    &TerminalSessionId::new(format!("session-{index}")).unwrap(),
                    &mutation(1, false),
                    prepared(),
                )
                .unwrap();
        }
        assert!(
            scheduler
                .install(&owner(), &session_id(), &mutation(1, false), prepared())
                .is_err()
        );
        assert_eq!(scheduler.grants.len(), 16);
        scheduler.shutdown();
        let mut scheduler =
            TerminalHostProbes::new(Arc::clone(&fixture.executor), CancellationToken::new());
        for index in 0..64 {
            let mut mutation = mutation(1, false);
            mutation.monitor_id = TerminalMonitorId::new(format!("monitor-{index}")).unwrap();
            scheduler
                .install(&owner(), &session_id(), &mutation, prepared())
                .unwrap();
        }
        assert!(
            scheduler
                .install(&owner(), &session_id(), &mutation(1, false), prepared())
                .is_err()
        );
        assert_eq!(scheduler.grants.len(), 64);
    }

    #[test]
    fn housekeeping_retires_automatic_monitor_expiration() {
        let fixture = Fixture::new();
        let mut monitor = definition(Condition::PathExists {
            path: "never-created".into(),
        });
        monitor.lifetime = TerminalMonitorLifetime::Duration { duration_ms: 40 };
        let prepared = fixture.prepare(&monitor);
        let harness = fixture.runtime(monitor, Some(prepared));
        Harness::until(|| harness.counts() == (0, 0, 0));
        let live = futures_executor::block_on(harness.runtime().request_with_context(
            CancellationToken::new(),
            |context| {
                context
                    .registry
                    .live_monitor_generations(&owner(), &session_id())
                    .unwrap()
            },
        ))
        .unwrap();
        assert!(live.is_empty());
    }

    #[test]
    fn missing_nested_path_becomes_present_and_existing_symlink_is_resolved() {
        let fixture = Fixture::new();
        let root = fixture.path.join("workspace");
        let monitor = definition(Condition::PathChanged {
            path: "missing/nested/ready".into(),
        });
        let prepared = fixture.prepare(&monitor);
        assert!(!prepared.activation().path_baseline.unwrap().exists);
        let harness = fixture.runtime(monitor, Some(prepared));
        std::fs::create_dir_all(root.join("missing/nested")).unwrap();
        std::fs::write(root.join("missing/nested/ready"), b"ready").unwrap();
        Harness::until(|| harness.counts() == (0, 0, 0));
        symlink("missing/nested/ready", root.join("leaf-link")).unwrap();
        let prepared = fixture.prepare(&definition(Condition::PathChanged {
            path: "leaf-link".into(),
        }));
        assert_eq!(prepared.activation().path_baseline.unwrap().size, 5);
        let denied = fixture.prepare(&definition(Condition::PathChanged {
            path: "later/ready".into(),
        }));
        symlink(fixture.path.join("outside"), root.join("later")).unwrap();
        let Some(TerminalProbeAuthority::Path { parent, leaf, .. }) = denied.authority else {
            panic!("path grant")
        };
        assert!(
            path_baseline(&parent, &leaf).is_err(),
            "new symlink cannot broaden retained authority"
        );
    }

    #[test]
    fn custom_probe_uses_captured_path_sh_and_session_cwd_independently_of_terminal_profile() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            for profile in [
                machine_god_core::TerminalProfile::Clean,
                machine_god_core::TerminalProfile::User,
            ] {
                let mut fixture = Fixture::new();
                let root = fixture.path.join("workspace");
                let cwd = root.join("session");
                let binary = root.join("bin");
                std::fs::create_dir(&cwd).unwrap();
                std::fs::create_dir(&binary).unwrap();
                std::fs::write(
                    binary.join("sh"),
                    b"#!/bin/sh\nprintf '%s' \"$1\" > shell-flags\nexec /bin/sh \"$@\"\n",
                )
                .unwrap();
                std::fs::set_permissions(binary.join("sh"), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                Arc::get_mut(&mut fixture.preparer).unwrap().environment = Arc::new(
                    ValidatedBackgroundEnvironment::new(vec![(
                        "PATH".into(),
                        binary.into_os_string(),
                    )])
                    .unwrap(),
                );
                std::fs::write(root.join("baseline"), b"root").unwrap();
                std::fs::write(cwd.join("baseline"), b"session").unwrap();
                let baseline = fixture.prepare_at(
                    &definition(Condition::PathChanged {
                        path: "baseline".into(),
                    }),
                    cwd.clone(),
                );
                assert_eq!(baseline.activation().path_baseline.unwrap().size, 7);
                let monitor = definition(Condition::CustomProbe {
                    command: "printf executed > observed".into(),
                    cwd: ".".into(),
                });
                let prepared = fixture.prepare_at(&monitor, cwd.clone());
                let harness =
                    fixture.runtime_at(monitor, Some(prepared), cwd.clone(), shell, profile);
                harness.until_probe("command observed", &cwd, || cwd.join("observed").exists());
                harness.until_probe("grant/publication drain", &cwd, || {
                    harness.counts() == (0, 0, 0)
                });
                assert_eq!(std::fs::read(cwd.join("shell-flags")).unwrap(), b"-lc");
                assert!(!root.join("observed").exists());
            }
        }
    }

    #[test]
    fn last_runtime_handle_stops_native_probe_with_unpolled_request_retained() {
        let fixture = Fixture::new();
        let monitor = definition(Condition::CustomProbe {
            command: "printf '%s' \"$$\" > leader; exec /bin/sleep 30".into(),
            cwd: ".".into(),
        });
        let prepared = fixture.prepare(&monitor);
        let mut harness = fixture.runtime(monitor, Some(prepared));
        let leader = fixture.path.join("workspace/leader");
        Harness::until(|| {
            std::fs::read_to_string(&leader).is_ok_and(|text| text.parse::<i32>().is_ok())
        });
        let pid = rustix::process::Pid::from_raw(
            std::fs::read_to_string(leader).unwrap().parse().unwrap(),
        )
        .unwrap();
        let unpolled = harness
            .runtime()
            .request_with_context(CancellationToken::new(), |_| panic!("unpolled request"));
        drop(harness.runtime.take());
        Harness::until(|| harness.stop.is_cancelled());
        Harness::until(|| rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH));
        drop(unpolled);
    }

    #[test]
    fn real_owner_scheduler_executes_custom_probe_publishes_and_prunes_until_match() {
        let fixture = Fixture::new();
        let definition = definition(Condition::CustomProbe {
            command: "printf executed > observed".into(),
            cwd: ".".into(),
        });
        let prepared = fixture.prepare(&definition);
        let harness = fixture.runtime(definition, Some(prepared));
        Harness::until(|| fixture.path.join("workspace/observed").exists());
        Harness::until(|| harness.counts() == (0, 0, 0));
        let live = futures_executor::block_on(harness.runtime().request_with_context(
            CancellationToken::new(),
            |context| {
                context
                    .registry
                    .live_monitor_generations(&owner(), &session_id())
                    .unwrap()
            },
        ))
        .unwrap();
        assert!(
            live.is_empty(),
            "successful evidence removes UntilMatch under profile transaction"
        );
    }

    #[test]
    fn persisted_probe_description_without_grant_never_connects() {
        let fixture = Fixture::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let harness = fixture.runtime(
            definition(Condition::TcpReady {
                host: "127.0.0.1".into(),
                port: address.port(),
            }),
            None,
        );
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(harness.counts(), (0, 0, 0));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn real_tcp_hostname_probe_runs_and_global_stop_cancels_custom_without_future_polls() {
        let fixture = Fixture::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_definition = definition(Condition::TcpReady {
            host: "localhost".into(),
            port: listener.local_addr().unwrap().port(),
        });
        let prepared = fixture.prepare(&tcp_definition);
        let harness = fixture.runtime(tcp_definition, Some(prepared));
        Harness::until(|| harness.counts() == (0, 0, 0));
        drop(harness);
        // Use a separate profile to keep retained session IDs distinct.
        let fixture = Fixture::new();
        let definition = definition(Condition::CustomProbe {
            command: "printf '%s' \"$$\" > leader; exec /bin/sleep 30".into(),
            cwd: ".".into(),
        });
        let prepared = fixture.prepare(&definition);
        let harness = fixture.runtime(definition, Some(prepared));
        Harness::until(|| {
            std::fs::read_to_string(fixture.path.join("workspace/leader"))
                .is_ok_and(|text| text.parse::<i32>().is_ok())
        });
        let pid = rustix::process::Pid::from_raw(
            std::fs::read_to_string(fixture.path.join("workspace/leader"))
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap();
        harness.stop.cancel();
        Harness::until(|| rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH));
        Harness::until(|| harness.counts() == (0, 0, 0));
    }
}
