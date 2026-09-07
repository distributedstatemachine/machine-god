//! Complete native action host. Real Engine/Session handles own the resource;
//! tools and submitted operations carry only non-owning runtime requesters.

use crate::terminal_action_tool::{
    TerminalActionExecutor, TerminalActionInvocation, TerminalActionTool,
};
use crate::terminal_captured_exec::{
    TERMINAL_CAPTURED_HELPER_ARGUMENT, TerminalCapturedAuthority, TerminalCapturedExec,
    TerminalCapturedExecError,
};
use crate::terminal_host_authority::{
    CapturedTerminalHostAuthority, TerminalHostAuthority, TerminalHostAuthorityInputs,
};
use crate::terminal_host_catalog::TerminalHostCatalogs;
use crate::terminal_host_dispatch::{self, TerminalCatalogState};
use crate::terminal_host_probes::{
    PreparedTerminalMonitor, TerminalHostProbePreparer, TerminalHostProbes,
};
use crate::terminal_input::TerminalWriterId;
use crate::terminal_monitor::TerminalMonitorActivation;
use crate::terminal_native_backend::TerminalNativeBackend;
use crate::terminal_native_launch::TerminalNativeLaunchError;
use crate::terminal_probe_effects::{NativeTerminalProbeExecutor, TerminalProbeClock};
use crate::terminal_profile::{TerminalProfileBudget, TerminalProfileLimits};
use crate::terminal_profile_store::TerminalProfileStore;
use crate::terminal_registry::TerminalRegistry;
use crate::terminal_resident_dispatch::{TerminalResidentAuthority, resident_facts};
use crate::terminal_runtime::{
    TerminalRuntime, TerminalRuntimeError, TerminalRuntimeRequester, TerminalRuntimeWorker,
};
use crate::terminal_session::TerminalSessionError;
use crate::terminal_staged_start::TerminalStagedStarter;
use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};
use futures_util::future::{Either, select};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, SessionIncarnationId, TerminalAction,
    TerminalActionRequest, TerminalActionResult, TerminalActorRole, TerminalAllowedControls,
    TerminalMonitorOperation, TerminalReturnCondition, TerminalSessionId, TerminalStartRequest,
    TerminalWaitRequest, ToolContext, ToolError, ToolErrorKind,
};
use rustix::fd::OwnedFd;
use std::num::NonZeroU64;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FOREGROUND_TIMEOUT: Duration = Duration::from_secs(120);
const START_TIMEOUT: Duration = crate::terminal_helper::MAX_STARTUP_TIMEOUT;
const MAX_EFFECTS: usize = 16;

struct HostState {
    catalogs: TerminalHostCatalogs,
    probes: TerminalHostProbes,
}
impl TerminalCatalogState for HostState {
    fn catalogs(&mut self) -> &mut TerminalHostCatalogs {
        &mut self.catalogs
    }
}
fn probes(state: &mut HostState) -> &mut TerminalHostProbes {
    &mut state.probes
}
fn reconcile_probes(state: &mut HostState, registry: &TerminalRegistry<TerminalNativeBackend>) {
    state.probes.reconcile_live(registry);
}
type Requester = TerminalRuntimeRequester<TerminalNativeBackend, HostState>;

/// Attach only through `EngineBuilder::host_resource`, never to an engine tool.
pub(crate) struct NativeTerminalHostResource {
    runtime: TerminalRuntime<TerminalNativeBackend, HostState>,
    stop: CancellationToken,
    workers: NativeOwnedWorkerScope,
}
impl NativeTerminalHostResource {
    pub(crate) fn worker_scope(&self) -> NativeOwnedWorkerScope {
        self.workers.clone()
    }

    pub(crate) fn completion(&self) -> NativeOwnedWorkerCompletion {
        self.workers.completion()
    }
}
impl Drop for NativeTerminalHostResource {
    fn drop(&mut self) {
        self.workers.close();
        self.stop.cancel();
        self.runtime.shutdown();
    }
}

pub(crate) struct NativeTerminalHost;

struct PreparedHost {
    host: Arc<CapturedTerminalHostAuthority>,
    preparer: Arc<TerminalHostProbePreparer>,
    captured: Arc<TerminalCapturedExec>,
    probe_executor: Arc<NativeTerminalProbeExecutor>,
}

impl PreparedHost {
    fn capture_on_worker(
        inputs: TerminalHostAuthorityInputs,
        workers: &NativeOwnedWorkerScope,
    ) -> Result<Self, ToolError> {
        let deadline = Instant::now() + START_TIMEOUT;
        let captured = Arc::new(
            TerminalCapturedExec::new(
                inputs.cli_executable.clone(),
                vec![TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
                FOREGROUND_TIMEOUT,
                MAX_EFFECTS,
            )
            .map_err(|_| unavailable())?
            .with_worker_scope(workers.clone()),
        );
        let host = Arc::new(
            TerminalHostAuthority::new(inputs)?
                .capture_on_worker(deadline, &CancellationToken::new())?,
        );
        let preparer = Arc::new(TerminalHostProbePreparer::new_on_worker(
            Arc::clone(&host),
            deadline,
            &CancellationToken::new(),
        )?);
        let probe_executor = Arc::new(
            NativeTerminalProbeExecutor::new(Arc::clone(&captured), MAX_EFFECTS)
                .map_err(|_| unavailable())?
                .with_worker_scope(workers.clone()),
        );
        Ok(Self {
            host,
            preparer,
            captured,
            probe_executor,
        })
    }
}

impl NativeTerminalHost {
    /// The reference host invokes this on its constructor worker. Captured
    /// authority is immutable; profile preparation remains lazy on its owner.
    pub(crate) fn compose_on_worker(
        inputs: TerminalHostAuthorityInputs,
        state_root: OwnedFd,
        host_identity: SessionIncarnationId,
    ) -> Result<(TerminalActionTool, NativeTerminalHostResource), ToolError> {
        let workers = NativeOwnedWorkerScope::new();
        let PreparedHost {
            host,
            preparer,
            captured,
            probe_executor,
        } = PreparedHost::capture_on_worker(inputs, &workers)?;
        let stop = CancellationToken::new();
        let worker_stop = stop.clone();
        let workspace = host.identity().workspace.clone();
        let requester_cell: Arc<OnceLock<Requester>> = Arc::new(OnceLock::new());
        let observer_requester = Arc::clone(&requester_cell);
        let clock = host_clock()?;
        let runtime = TerminalRuntime::new(
            move || {
                let store = TerminalProfileStore::prepare(state_root)
                    .map_err(|_| TerminalRuntimeError::Initialization)?;
                let registry = TerminalRegistry::new(workspace.clone())
                    .map_err(|_| TerminalRuntimeError::Initialization)?;
                let budget = TerminalProfileBudget::new(TerminalProfileLimits::default())
                    .map_err(|_| TerminalRuntimeError::Initialization)?;
                let state = HostState {
                    catalogs: TerminalHostCatalogs::new(workspace)
                        .map_err(|_| TerminalRuntimeError::Initialization)?,
                    probes: TerminalHostProbes::new(probe_executor, worker_stop),
                };
                Ok(TerminalRuntimeWorker::new_with_state(
                    registry,
                    store,
                    budget,
                    state,
                    clock,
                    move |state, steps| {
                        for step in &steps {
                            if step.cleanup_error.is_some() {
                                state.probes.retire_session(&step.owner, &step.session_id);
                            }
                        }
                        if let Some(requester) = observer_requester.get() {
                            state.probes.observe(
                                steps,
                                TerminalProbeClock {
                                    now_ms: clock(),
                                    observed_at: Instant::now(),
                                },
                                requester,
                                probes,
                            );
                        }
                    },
                ))
            },
            Arc::new(workers.clone()),
        );
        let requester = runtime.requester();
        requester_cell
            .set(requester.clone())
            .map_err(|_| unavailable())?;
        let starter = Arc::new(
            TerminalStagedStarter::new(
                requester.clone(),
                host.launch_config(),
                host_identity,
                HostState::catalogs,
            )
            .with_worker_scope(workers.clone()),
        );
        let identity = host.identity().clone();
        let executor = NativeTerminalActionExecutor {
            requester,
            host,
            preparer,
            starter,
            captured,
            stop: stop.clone(),
            active: Arc::new(AtomicUsize::new(0)),
            workers: workers.clone(),
        };
        let tool = TerminalActionTool::new(Arc::new(executor), identity)?;
        Ok((
            tool,
            NativeTerminalHostResource {
                runtime,
                stop,
                workers,
            },
        ))
    }
}

#[derive(Clone)]
struct NativeTerminalActionExecutor {
    requester: Requester,
    host: Arc<CapturedTerminalHostAuthority>,
    preparer: Arc<TerminalHostProbePreparer>,
    starter: Arc<TerminalStagedStarter<HostState>>,
    captured: Arc<TerminalCapturedExec>,
    stop: CancellationToken,
    active: Arc<AtomicUsize>,
    workers: NativeOwnedWorkerScope,
}

impl TerminalActionExecutor for NativeTerminalActionExecutor {
    fn execute(
        &self,
        context: ToolContext,
        invocation: TerminalActionInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
        let host = self.clone();
        Box::pin(async move {
            check(&cancellation, &host.stop)?;
            let authority = resident_authority(context);
            if invocation.action() == TerminalAction::Exec {
                return host.exec(invocation, cancellation).await;
            }
            if matches!(
                invocation.action(),
                TerminalAction::Start | TerminalAction::Monitor
            ) || invocation.has_workspace_filter() {
                return host.owned_action(authority, invocation, cancellation).await;
            }
            let request = invocation.resolve_cwd(|_| Err(unavailable()))?;
            let reply = terminal_host_dispatch::dispatch(
                host.requester,
                authority,
                request,
                TerminalMonitorActivation::default(),
                cancellation,
            )
            .await
            .map_err(dispatch_error)?;
            Ok(reply_result(reply))
        })
    }
}

impl NativeTerminalActionExecutor {
    fn exec(
        &self,
        invocation: TerminalActionInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalActionResult, ToolError>> {
        let host = Arc::clone(&self.host);
        let future = self.captured.execute_prepared(
            move |deadline, cancellation, stop| {
                check(cancellation, stop).map_err(|_| TerminalCapturedExecError::Cancelled)?;
                let resolved = host
                    .resolve_on_worker(invocation, deadline, cancellation)
                    .map_err(|_| TerminalCapturedExecError::Invalid)?;
                let TerminalActionRequest::Exec { request } = resolved.request else {
                    return Err(TerminalCapturedExecError::Invalid);
                };
                let shell = host
                    .exec_shell(&request)
                    .map_err(|_| TerminalCapturedExecError::Invalid)?;
                Ok(TerminalCapturedAuthority {
                    request,
                    shell,
                    environment: host.environment_on_worker(),
                    cwd: resolved.cwd.ok_or(TerminalCapturedExecError::Invalid)?,
                })
            },
            cancellation,
            self.stop.clone(),
        );
        Box::pin(async move {
            future
                .await
                .map(|result| TerminalActionResult::Exec { result })
                .map_err(|error| match error {
                    TerminalCapturedExecError::Cancelled => cancelled(),
                    _ => effect_error(),
                })
        })
    }

    fn owned_action(
        self,
        authority: TerminalResidentAuthority,
        invocation: TerminalActionInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalActionResult, ToolError>> {
        Box::pin(async move {
            let deadline = Instant::now() + START_TIMEOUT;
            check(&cancellation, &self.stop)?;
            self.active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < MAX_EFFECTS).then_some(n + 1)
                })
                .map_err(|_| unavailable())?;
            let permit = EffectPermit(Arc::clone(&self.active));
            let dropped = CancellationToken::new();
            let _cancel_on_drop = CancelOnDrop(dropped.clone());
            let receipt = self
                .workers
                .clone()
                .run(move || {
                    let result = futures_executor::block_on(async {
                        check(&cancellation, &dropped)?;
                        let effective = CancellationToken::new();
                        let operation = self.effect_on_worker(
                            authority,
                            invocation,
                            deadline,
                            effective.clone(),
                        );
                        let stopped = select(
                            cancellation.cancelled(),
                            select(dropped.cancelled(), self.stop.cancelled()),
                        );
                        match select(operation, stopped).await {
                            Either::Left((result, _)) => result,
                            Either::Right((_, operation)) => {
                                effective.cancel();
                                operation.await
                            }
                        }
                    });
                    (result, permit)
                })
                .await
                .map_err(|_| effect_error())?;
            receipt.0
        })
    }

    fn effect_on_worker(
        &self,
        authority: TerminalResidentAuthority,
        invocation: TerminalActionInvocation,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
        Box::pin(async move {
            check(&cancellation, &self.stop)?;
            let resolved = self
                .host
                .resolve_on_worker(invocation, deadline, &cancellation)?;
            match resolved.request {
                TerminalActionRequest::Start { request } => {
                    let cwd = resolved.cwd.ok_or_else(unavailable)?;
                    self.start_on_worker(authority, request, cwd, deadline, cancellation)
                        .await
                }
                TerminalActionRequest::Monitor {
                    session_id,
                    operation,
                } => {
                    self.monitor_on_worker(authority, session_id, operation, deadline, cancellation)
                        .await
                }
                request @ TerminalActionRequest::List { .. } => {
                    terminal_host_dispatch::dispatch(
                        self.requester.clone(),
                        authority,
                        request,
                        TerminalMonitorActivation::default(),
                        cancellation,
                    )
                    .await
                    .map(reply_result)
                    .map_err(dispatch_error)
                }
                _ => Err(unavailable()),
            }
        })
    }

    async fn start_on_worker(
        &self,
        authority: TerminalResidentAuthority,
        request: TerminalStartRequest,
        cwd: OwnedFd,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<TerminalActionResult, ToolError> {
        let id = new_session_id()?;
        let mut prepared = Vec::with_capacity(request.initial_monitors.len());
        for definition in &request.initial_monitors {
            check(&cancellation, &self.stop)?;
            prepared.push(self.preparer.prepare_on_worker(
                definition,
                &request.cwd,
                deadline,
                &cancellation,
            )?);
        }
        let activations = prepared
            .iter()
            .map(PreparedTerminalMonitor::activation)
            .collect();
        let condition = request
            .return_when
            .clone()
            .unwrap_or(TerminalReturnCondition::Started);
        let ceiling = request.wait_ceiling_ms.unwrap_or(300_000);
        let owner = authority.owner.clone();
        let session = id.clone();
        let host = Arc::clone(&self.host);
        let launch_cancel = cancellation.clone();
        let launch_stop = self.stop.clone();
        let receipt = self
            .starter
            .start_until(
                authority.clone(),
                id.clone(),
                request,
                move || {
                    check(&launch_cancel, &launch_stop)
                        .map_err(|_| TerminalNativeLaunchError::Cancelled)?;
                    host.launch_authority_on_worker(cwd, deadline, &launch_cancel)
                        .map_err(|_| TerminalNativeLaunchError::Process)
                },
                activations,
                reconcile_probes,
                move |state, mutations| {
                    if mutations.len() != prepared.len() {
                        return Err(TerminalSessionError::InvalidState);
                    }
                    for (mutation, prepared) in mutations.into_iter().zip(prepared) {
                        if state
                            .probes
                            .install(&owner, &session, &mutation, prepared)
                            .is_err()
                        {
                            state.probes.retire_session(&owner, &session);
                            return Err(TerminalSessionError::InvalidState);
                        }
                    }
                    Ok(())
                },
                deadline,
                cancellation.clone(),
            )
            .await
            .map_err(start_error)?;
        // Keep the admitted session resident until its final start projection,
        // including while the follow-up wait is being admitted.
        let residency = receipt.residency;
        let wait = TerminalActionRequest::Wait {
            session_id: id,
            request: TerminalWaitRequest {
                condition,
                safety_ceiling_ms: ceiling,
            },
        };
        let reply = terminal_host_dispatch::dispatch(
            self.requester.clone(),
            authority,
            wait,
            TerminalMonitorActivation::default(),
            cancellation.clone(),
        )
        .await;
        let reply = match reply {
            Ok(reply) => reply,
            Err(_) if cancellation.is_cancelled() => {
                return Ok(TerminalActionResult::Start {
                    session: receipt.session,
                    outcome: machine_god_core::TerminalReturnOutcome::Cancelled {},
                });
            }
            Err(error) => return Err(dispatch_error(error)),
        };
        let result = match reply_result(reply) {
            TerminalActionResult::Wait { session, outcome }
                if session.session_id == receipt.session.session_id =>
            {
                Ok(TerminalActionResult::Start { session, outcome })
            }
            _ => Err(effect_error()),
        };
        drop(residency);
        result
    }

    async fn monitor_on_worker(
        &self,
        authority: TerminalResidentAuthority,
        id: TerminalSessionId,
        operation: TerminalMonitorOperation,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<TerminalActionResult, ToolError> {
        let preparation = match &operation {
            TerminalMonitorOperation::Add { definition }
            | TerminalMonitorOperation::Update { definition, .. } => {
                let owner = authority.owner.clone();
                let session = id.clone();
                let cwd = self
                    .requester
                    .request_with_context(cancellation.clone(), move |context| {
                        context
                            .registry
                            .inspect(&owner, &session)
                            .ok()
                            .and_then(|facts| facts.metadata.map(|metadata| metadata.cwd))
                    })
                    .await
                    .map_err(|_| unavailable())?
                    .ok_or_else(unavailable)?;
                Some(
                    self.preparer
                        .prepare_on_worker(definition, &cwd, deadline, &cancellation)?,
                )
            }
            _ => None,
        };
        check(&cancellation, &self.stop)?;
        self.requester
            .request_with_context(cancellation, move |mut context| {
                reconcile_probes(context.state, context.registry);
                let activation = preparation.as_ref().map_or_else(
                    TerminalMonitorActivation::default,
                    PreparedTerminalMonitor::activation,
                );
                let mutation = context
                    .registry
                    .mutate_with_profile(
                        context.store,
                        context.budget,
                        &authority.owner,
                        &id,
                        |session, persistence| {
                            session.monitor_with(
                                persistence,
                                &authority.owner,
                                operation.clone(),
                                activation,
                                context.now_ms,
                            )
                        },
                    )
                    .map_err(|_| effect_error())?;
                let result = match operation {
                    TerminalMonitorOperation::Add { .. }
                    | TerminalMonitorOperation::Update { .. } => context.state.probes.install(
                        &authority.owner,
                        &id,
                        &mutation,
                        preparation.ok_or_else(effect_error)?,
                    ),
                    TerminalMonitorOperation::Pause { .. } => {
                        context.state.probes.pause(&authority.owner, &id, &mutation)
                    }
                    TerminalMonitorOperation::Resume { .. } => {
                        context
                            .state
                            .probes
                            .resume(&authority.owner, &id, &mutation)
                    }
                    TerminalMonitorOperation::Remove { .. } => {
                        context
                            .state
                            .probes
                            .remove(&authority.owner, &id, &mutation)
                    }
                };
                if result.is_err() {
                    context.state.probes.retire_session(&authority.owner, &id);
                    return Err(effect_error());
                }
                let session =
                    resident_facts(&mut context, &authority, &id).map_err(|_| effect_error())?;
                Ok(TerminalActionResult::Monitor {
                    session,
                    monitor_id: Some(mutation.monitor_id),
                })
            })
            .await
            .map_err(|_| effect_error())?
    }
}

fn host_clock() -> Result<impl Fn() -> i64 + Copy, ToolError> {
    let epoch = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| unavailable())?
            .as_millis(),
    )
    .map_err(|_| unavailable())?;
    let anchor = Instant::now();
    Ok(move || {
        epoch.saturating_add(i64::try_from(anchor.elapsed().as_millis()).unwrap_or(i64::MAX))
    })
}

fn resident_authority(context: ToolContext) -> TerminalResidentAuthority {
    TerminalResidentAuthority {
        owner: BackgroundOutputOwner::new(context.session_id, context.session_incarnation_id),
        actor: TerminalActorRole::Agent,
        writer: TerminalWriterId::new(NonZeroU64::MIN),
        controls: TerminalAllowedControls {
            read: true,
            screen: true,
            write: true,
            wait: true,
            monitor: true,
            inspect: true,
            list: true,
            resize: true,
            signal: true,
            close: true,
        },
        // The adapter binds close permission separately for lease revocation.
        revoke_authorized: true,
    }
}
fn new_session_id() -> Result<TerminalSessionId, ToolError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| unavailable())?;
    let mut value = String::from("terminal-");
    for byte in bytes {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 15)]));
    }
    TerminalSessionId::new(value).map_err(|_| unavailable())
}
struct EffectPermit(Arc<AtomicUsize>);
impl Drop for EffectPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
fn check(cancellation: &CancellationToken, stop: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() || stop.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}
fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "terminal_cancelled",
        "terminal operation cancelled",
        false,
    )
}
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "terminal_unavailable",
        "terminal host unavailable",
        false,
    )
}
fn effect_error() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "terminal_effect_unavailable",
        "terminal operation failed; an admitted effect may have committed",
        false,
    )
}

fn reply_result(reply: terminal_host_dispatch::TerminalHostReply) -> TerminalActionResult {
    // The wire contract has no independent facts timestamp. A shutdown fallback
    // preserves the explicit admission snapshot, never synthesizes new facts.
    match reply.facts_timing {
        terminal_host_dispatch::TerminalHostFactsTiming::Current
        | terminal_host_dispatch::TerminalHostFactsTiming::Admission => reply.result,
    }
}

fn diagnostic(code: &str, error: impl std::fmt::Debug) -> ToolError {
    // Callers below pass only closed, data-free failure vocabularies or bounded
    // committed receipt metadata, never commands, environment or output bytes.
    ToolError::new(
        ToolErrorKind::Execution,
        code,
        format!("terminal operation failed ({error:?}); an admitted effect may have committed"),
        false,
    )
}

fn dispatch_error(error: terminal_host_dispatch::TerminalHostDispatchError) -> ToolError {
    use crate::terminal_host_dispatch::TerminalHostDispatchError as Error;
    match error {
        Error::Invalid | Error::CommandAction => unavailable(),
        Error::Runtime(error) => diagnostic("terminal_runtime", error),
        Error::Catalog(error) => diagnostic("terminal_catalog", error),
        Error::Write(error) => diagnostic("terminal_write", error),
        Error::WaitReceipt(receipt) => diagnostic("terminal_wait_committed", receipt),
        Error::WriteReceipt(receipt) => diagnostic("terminal_write_committed", receipt),
        Error::Resident(error) => resident_error(error),
    }
}

fn resident_error(error: crate::terminal_resident_dispatch::TerminalResidentError) -> ToolError {
    use crate::terminal_resident_dispatch::{
        TerminalResidentEffect as Effect, TerminalResidentError as Error,
    };
    match error {
        Error::Cancelled => cancelled(),
        Error::Invalid | Error::Unsupported | Error::UnauthorizedRevoke => unavailable(),
        Error::Registry(error) => diagnostic("terminal_registry", error),
        Error::Wait(error) => diagnostic("terminal_wait", error),
        Error::Write(error) => diagnostic("terminal_write", error),
        Error::Committed { effect, error } => match effect {
            Effect::Resize(dimensions) => {
                diagnostic("terminal_resize_committed", (dimensions, error))
            }
            Effect::Signal(signal) => diagnostic("terminal_signal_committed", (signal, error)),
            Effect::Close(policy) => diagnostic("terminal_close_committed", (policy, error)),
            Effect::Monitor(id) => diagnostic("terminal_monitor_committed", (id, error)),
        },
    }
}

fn start_error(error: crate::terminal_staged_start::TerminalStagedStartError) -> ToolError {
    use crate::terminal_staged_start::TerminalStagedStartError as Error;
    match error {
        Error::Cancelled => cancelled(),
        Error::Invalid | Error::Capacity | Error::Worker => unavailable(),
        Error::Runtime(error) => diagnostic("terminal_runtime", error),
        Error::Registry(error) => diagnostic("terminal_registry", error),
        Error::Catalog(error) => diagnostic("terminal_catalog", error),
        Error::Launch(error) => diagnostic("terminal_launch", error),
        Error::Retained { session_id, error } => ToolError::new(
            ToolErrorKind::Execution,
            "terminal_start_retained",
            format!(
                "terminal {} was admitted but startup failed ({error:?}); inspect or close it rather than restarting blindly",
                session_id.as_str()
            ),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_host_authority::TerminalHostAccountShell;
    use machine_god_core::{SessionId, Tool, ToolCall, ToolCallId, ToolName, TurnId};
    use rustix::fs::{Mode, OFlags};
    use serde_json::{Value, json};
    use std::num::NonZeroU32;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    struct Fixture {
        root: PathBuf,
        tool: TerminalActionTool,
        resource: Option<NativeTerminalHostResource>,
        completion: NativeOwnedWorkerCompletion,
    }
    fn open(path: &Path) -> OwnedFd {
        rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap()
    }
    impl Fixture {
        fn new() -> Self {
            let mut nonce = [0_u8; 8];
            getrandom::fill(&mut nonce).unwrap();
            let root =
                std::env::temp_dir().join(format!("mg-h-{:016x}", u64::from_le_bytes(nonce)));
            std::fs::create_dir(&root).unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            for child in ["workspace", "state", "artifacts"] {
                let path = root.join(child);
                std::fs::create_dir(&path).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let cli = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY").map_or_else(|| {
                let script = root.join("helper");
                let executable = std::env::current_exe().unwrap();
                let quoted = executable.to_str().unwrap().replace('\'', "'\\''");
                std::fs::write(&script, format!("#!/bin/sh\nexport MACHINE_GOD_TEST_HOST_HELPER=\"$1\"\nexec '{quoted}' --exact terminal_host::tests::helper_child --ignored --nocapture --quiet\n")).unwrap();
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
                script
            }, PathBuf::from);
            let workspace = root.join("workspace");
            // Exercise the composed host beyond both sockaddr_un and one
            // canonical terminal input line, including shell-sensitive paths.
            let mut artifacts = root.join("artifacts");
            for _ in 0..6 {
                artifacts.push(format!("日本語's-{}", "nested".repeat(15)));
                std::fs::create_dir(&artifacts).unwrap();
                std::fs::set_permissions(&artifacts, std::fs::Permissions::from_mode(0o700))
                    .unwrap();
            }
            let (tool, resource) = NativeTerminalHost::compose_on_worker(
                TerminalHostAuthorityInputs {
                    workspace: open(&workspace),
                    workspace_path: workspace.clone(),
                    default_cwd: workspace,
                    environment: vec![
                        ("PATH".into(), "/usr/bin:/bin".into()),
                        ("HOME".into(), root.as_os_str().to_owned()),
                    ],
                    account_shell: TerminalHostAccountShell::Explicit(Some("/bin/bash".into())),
                    cli_executable: cli,
                    tmux_executable: None,
                    artifacts: open(&artifacts),
                    artifact_path: artifacts,
                },
                open(&root.join("state")),
                SessionIncarnationId::new("host-test").unwrap(),
            )
            .unwrap();
            Self {
                root,
                tool,
                completion: resource.completion(),
                resource: Some(resource),
            }
        }
        fn context() -> ToolContext {
            ToolContext {
                session_id: SessionId::new("owner").unwrap(),
                session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
                turn_id: TurnId::new("turn").unwrap(),
                call_id: ToolCallId::new("call").unwrap(),
            }
        }
        fn future(
            &self,
            arguments: Value,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
            self.future_with_expected_error(arguments, cancellation, false)
        }
        fn future_with_expected_error(
            &self,
            arguments: Value,
            cancellation: CancellationToken,
            expected_error: bool,
        ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
            let call = ToolCall {
                id: ToolCallId::new("call").unwrap(),
                name: ToolName::new("terminal").unwrap(),
                arguments,
            };
            let action = call.arguments["action"].as_str().unwrap().to_owned();
            let prepared = self
                .tool
                .prepare(call)
                .unwrap_or_else(|error| panic!("prepare {action}: {error:?}"));
            let execution =
                self.tool
                    .execute(Self::context(), prepared.arguments().clone(), cancellation);
            Box::pin(async move {
                let output = execution.await?;
                assert_eq!(output.is_error, expected_error, "{action}: tool failure flag");
                Ok(serde_json::from_value(output.content).unwrap())
            })
        }
        fn action(&self, arguments: Value) -> TerminalActionResult {
            let action = arguments["action"].as_str().unwrap().to_owned();
            self.try_action(arguments)
                .unwrap_or_else(|error| panic!("{action}: {error:?}"))
        }
        fn try_action(&self, arguments: Value) -> Result<TerminalActionResult, ToolError> {
            futures_executor::block_on(self.future(arguments, CancellationToken::new()))
        }
        fn close(&self, id: &TerminalSessionId) -> machine_god_core::TerminalSessionFacts {
            // A bounded native inventory may fail under host load. Observe the
            // retained failure before issuing another explicit close for this
            // exact session. Never replay start, write, or arbitrary errors.
            for attempt in 0..4 {
                match self.try_action(json!({"action":"close","session_id":id.as_str(),"close_policy":"force"})) {
                    Ok(TerminalActionResult::Close { session, .. }) => {
                        assert_eq!(&session.session_id, id);
                        assert_eq!(session.lifecycle, machine_god_core::TerminalLifecycle::Closed);
                        return session;
                    }
                    Ok(_) => panic!("close receipt"),
                    Err(error) => {
                        assert_native_cleanup_failure(&error);
                        self.assert_retained_close_failure(id);
                        assert!(attempt < 3, "bounded explicit close recovery exhausted: {error:?}");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
            unreachable!("the last attempt returns or fails")
        }
        fn assert_retained_close_failure(&self, id: &TerminalSessionId) {
            let TerminalActionResult::Inspect { session, .. } =
                self.action(json!({"action":"inspect","session_id":id.as_str()}))
            else {
                panic!("retained inspection receipt");
            };
            assert_eq!(&session.session_id, id);
            assert_eq!(session.lifecycle, machine_god_core::TerminalLifecycle::Lost);
            assert!(matches!(
                session.screen_recovery,
                machine_god_core::TerminalScreenRecovery::Unavailable {
                    reason: machine_god_core::TerminalScreenUnavailableReason::RawGap
                }
            ));
        }
    }
    fn assert_native_cleanup_failure(error: &ToolError) {
        assert_eq!(
            error,
            &diagnostic(
                "terminal_registry",
                crate::terminal_registry::TerminalRegistryError::Session(
                    crate::terminal_session::TerminalSessionError::Native
                )
            ),
            "only the exact retained native-close failure admits fixture recovery"
        );
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            drop(self.resource.take());
            let deadline = Instant::now() + Duration::from_secs(15);
            while !self.completion.is_complete() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if !self.completion.is_complete() && std::thread::panicking() {
                // Preserve the failed fixture while cleanup still owns it;
                // neither destroy active state nor abort on a second panic.
                return;
            }
            assert!(
                self.completion.is_complete(),
                "host workers and transferred child cleanup joined"
            );
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    #[ignore = "private host helper subprocess"]
    fn helper_child() {
        let status = match std::env::var("MACHINE_GOD_TEST_HOST_HELPER").as_deref() {
            Ok(crate::TERMINAL_CAPTURED_HELPER_ARGUMENT) => {
                crate::run_terminal_captured_helper().is_ok()
            }
            Ok(crate::TERMINAL_PTY_HELPER_ARGUMENT) => crate::run_terminal_pty_helper().is_ok(),
            Ok(crate::TERMINAL_STARTUP_MARKER_ARGUMENT) => {
                crate::run_terminal_startup_marker().is_ok()
            }
            _ => false,
        };
        std::process::exit(if status { 0 } else { 125 });
    }

    #[test]
    fn full_host_exec_and_unpolled_start_share_inert_adapter() {
        let fixture = Fixture::new();
        assert!(!fixture.root.join("state/terminal-v1").exists());
        drop(fixture.future(
            json!({"action":"start","command":"printf forbidden > forbidden"}),
            CancellationToken::new(),
        ));
        assert!(!fixture.root.join("workspace/forbidden").exists());
        let TerminalActionResult::Exec { result } = futures_executor::block_on(fixture.future_with_expected_error(
            json!({"action":"exec","profile":"clean","command":"printf foreground; exit 7"}),
            CancellationToken::new(),
            true,
        )).unwrap() else {
            panic!("exec receipt");
        };
        assert_eq!(
            result.status,
            machine_god_core::TerminalExecStatus::Exited { exit_code: 7 }
        );
        assert!(result.stdout.bytes.ends_with(b"foreground"));
    }

    #[test]
    fn full_host_start_read_screen_resize_inspect_and_close() {
        let fixture = Fixture::new();
        let TerminalActionResult::Start { session, .. } = fixture.action(json!({"action":"start","profile":"clean","command":"printf host-ready; exec /bin/sleep 30"})) else { panic!("start receipt"); };
        let id = session.session_id;
        let wait = fixture.action(json!({"action":"wait","session_id":id.as_str(),"return_when":{"kind":"match","pattern":"host-ready"},"wait_ceiling_ms":5000}));
        assert!(matches!(
            wait,
            TerminalActionResult::Wait {
                outcome: machine_god_core::TerminalReturnOutcome::ConditionMet {},
                ..
            }
        ));
        let TerminalActionResult::Read { output, .. } =
            fixture.action(json!({"action":"read","session_id":id.as_str(),"cursor_segment":1}))
        else {
            panic!("read receipt");
        };
        assert!(
            output
                .windows(b"host-ready".len())
                .any(|bytes| bytes == b"host-ready")
        );
        assert!(matches!(
            fixture.action(json!({"action":"screen","session_id":id.as_str()})),
            TerminalActionResult::Screen { .. }
        ));
        assert!(matches!(
            fixture
                .action(json!({"action":"resize","session_id":id.as_str(),"rows":31,"columns":97})),
            TerminalActionResult::Resize { .. }
        ));
        let TerminalActionResult::Inspect { cwd, .. } =
            fixture.action(json!({"action":"inspect","session_id":id.as_str()}))
        else {
            panic!("inspect receipt");
        };
        assert_eq!(cwd, fixture.root.join("workspace").to_str().unwrap());
        fixture.close(&id);
        let TerminalActionResult::List { sessions } = fixture.action(json!({"action":"list"}))
        else {
            panic!("list receipt");
        };
        assert!(sessions.iter().any(|session| session.session_id == id));
    }

    #[test]
    fn full_host_list_resolves_workspace_filters_without_expanding_owner_authority() {
        let fixture = Fixture::new();
        let workspace = fixture.root.join("workspace");
        std::fs::create_dir_all(workspace.join("real/deep")).unwrap();
        std::os::unix::fs::symlink("real/deep", workspace.join("link")).unwrap();
        let TerminalActionResult::Start { session, .. } = fixture.action(json!({
            "action":"start", "profile":"clean", "command":"exec /bin/sleep 30"
        })) else { panic!("start receipt"); };
        fixture.close(&session.session_id);
        let ids = |arguments| {
            let TerminalActionResult::List { sessions } = fixture.action(arguments)
            else { panic!("list receipt"); };
            sessions.into_iter().map(|session| session.session_id).collect::<Vec<_>>()
        };
        let expected = ids(json!({"action":"list"}));
        assert_eq!(expected, vec![session.session_id]);
        for root in [
            ".".to_owned(), workspace.display().to_string(),
            format!("{}/.", workspace.display()), "link/../..".to_owned(),
            " \t.\r\n".to_owned(), "~/workspace".to_owned(), "~//workspace".to_owned(),
        ] {
            assert_eq!(ids(json!({"action":"list","workspace_root":root})), expected, "{root}");
        }
        // Native link/.. is real, not the workspace produced by lexical removal.
        for root in ["link/..".to_owned(), fixture.root.display().to_string(), "..".to_owned()] {
            assert!(ids(json!({"action":"list","workspace_root":root})).is_empty());
        }
        assert!(ids(json!({"action":"list","workspace_root":".","task_id":"other-owner"})).is_empty());
        let mut foreign = Fixture::context();
        foreign.session_id = SessionId::new("foreign-owner").unwrap();
        let prepared = fixture.tool.prepare(ToolCall {
            id: foreign.call_id.clone(), name: ToolName::new("terminal").unwrap(),
            arguments: json!({"action":"list","workspace_root":".","task_id":"owner"}),
        }).unwrap();
        let output = futures_executor::block_on(fixture.tool.execute(
            foreign, prepared.arguments().clone(), CancellationToken::new()
        )).unwrap();
        assert_eq!(output.content["sessions"], json!([]));
        for root in ["missing", "missing/..", " \t ", "~other"] {
            assert!(futures_executor::block_on(fixture.future(
                json!({"action":"list","workspace_root":root}), CancellationToken::new()
            )).is_err(), "{root}");
        }
    }

    #[test]
    fn full_host_filtered_list_is_inert_until_polled_and_cancelled_before_submission() {
        let fixture = Fixture::new();
        let arguments = json!({"action":"list","workspace_root":"."});
        drop(fixture.future(arguments.clone(), CancellationToken::new()));
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(futures_executor::block_on(fixture.future(arguments, cancelled)).is_err());
        assert!(!fixture.root.join("state/terminal-v1").exists());
    }

    #[test]
    fn full_host_shutdown_joins_foreground_work_with_an_unpolled_receipt() {
        use std::task::{Context, Poll};
        let mut fixture = Fixture::new();
        let resource = fixture.resource.take().unwrap();
        let completion = resource.completion();
        let mut execution = fixture.future(
            json!({"action":"exec","profile":"clean","command":"printf ready > exec-ready; exec /bin/sleep 30"}),
            CancellationToken::new(),
        );
        let mut context = Context::from_waker(std::task::Waker::noop());
        assert!(matches!(
            execution.as_mut().poll(&mut context),
            Poll::Pending
        ));
        let deadline = Instant::now() + Duration::from_secs(15);
        while !fixture.root.join("workspace/exec-ready").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(fixture.root.join("workspace/exec-ready").exists());
        assert!(!completion.is_complete());
        drop(resource);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !completion.is_complete() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            completion.is_complete(),
            "native cleanup must not require polling the response"
        );
        assert!(futures_executor::block_on(execution).is_err());
    }

    #[test]
    fn full_host_dormant_shutdown_does_not_need_to_initialize_the_owner() {
        let mut fixture = Fixture::new();
        let resource = fixture.resource.take().unwrap();
        let completion = resource.completion();
        let unpolled = fixture.future(json!({"action":"start"}), CancellationToken::new());
        assert!(!completion.is_complete());
        drop(resource);
        assert!(completion.is_complete());
        assert!(!fixture.root.join("state/terminal-v1").exists());
        assert!(futures_executor::block_on(unpolled).is_err());
    }

    #[test]
    fn full_host_interactive_write_monitor_probe_and_signal() {
        let fixture = Fixture::new();
        let TerminalActionResult::Start { session, .. } =
            fixture.action(json!({"action":"start","profile":"clean"}))
        else {
            panic!("start receipt");
        };
        let id = session.session_id;
        fixture.action(json!({"action":"write","session_id":id.as_str(),"lease":"acquire"}));
        let TerminalActionResult::Write { accepted_bytes, .. } = fixture.action(json!({"action":"write","session_id":id.as_str(),"lease":"use","write":{"kind":"text","text":"printf write-ready\\n\n"}})) else { panic!("write receipt"); };
        assert!(accepted_bytes > 0);
        fixture.action(json!({"action":"wait","session_id":id.as_str(),"return_when":{"kind":"match","pattern":"write-ready"},"wait_ceiling_ms":5000}));
        let definition = json!({"condition":{"kind":"custom_probe","command":"printf probe-ran > probe-ran", "cwd":"."},"check_interval_ms":10,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_session_end"}});
        let TerminalActionResult::Monitor { monitor_id: Some(monitor), .. } = fixture.action(json!({"action":"monitor","session_id":id.as_str(),"monitor":{"kind":"add","definition":definition}})) else { panic!("monitor receipt"); };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.root.join("workspace/probe-ran").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            fixture.root.join("workspace/probe-ran").exists(),
            "ordinary owner pump executes authorized grant"
        );
        for operation in ["pause", "resume", "remove"] {
            fixture.action(json!({"action":"monitor","session_id":id.as_str(),"monitor":{"kind":operation,"monitor_id":monitor.as_str()}}));
        }
        fixture.action(json!({"action":"signal","session_id":id.as_str(),"signal":"terminate"}));
        fixture.close(&id);
    }

    #[test]
    fn full_host_failed_close_retains_history_and_explicit_cleanup_authority() {
        struct InjectedSnapshotFailure(NonZeroU32);
        impl Drop for InjectedSnapshotFailure {
            fn drop(&mut self) {
                crate::background_process::inject_group_snapshot_spawn_failures_for_test(self.0, 0);
            }
        }
        let _guard = crate::background_process::GROUP_SNAPSHOT_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture = Fixture::new();
        let TerminalActionResult::Start { session, .. } = fixture.action(json!({
            "action":"start", "profile":"clean",
            "command":"printf '%s\\n' \"$$\" > close-owner.pid; printf 'once\\n' >> close-count; printf CLOSE_RETAINED; exec /bin/sleep 30"
        })) else {
            panic!("start receipt");
        };
        let id = session.session_id;
        assert!(matches!(
            fixture.action(json!({"action":"wait","session_id":id.as_str(),"return_when":{"kind":"match","pattern":"CLOSE_RETAINED"},"wait_ceiling_ms":5000})),
            TerminalActionResult::Wait {
                outcome: machine_god_core::TerminalReturnOutcome::ConditionMet {},
                ..
            }
        ));
        // The owned command's PID only selects a test fault; production close
        // still obtains all authority through the exact retained session.
        let pid = std::fs::read_to_string(fixture.root.join("workspace/close-owner.pid"))
            .unwrap()
            .trim()
            .parse::<NonZeroU32>()
            .unwrap();
        let injection = InjectedSnapshotFailure(pid);
        crate::background_process::inject_group_snapshot_spawn_failures_for_test(pid, 1);
        let failed = fixture.try_action(json!({"action":"close","session_id":id.as_str(),"close_policy":"force"}));
        drop(injection);
        assert_native_cleanup_failure(&failed.unwrap_err());
        fixture.assert_retained_close_failure(&id);
        let closed = fixture.close(&id);
        assert!(matches!(
            closed.screen_recovery,
            machine_god_core::TerminalScreenRecovery::Unavailable {
                reason: machine_god_core::TerminalScreenUnavailableReason::RawGap
            }
        ));
        let TerminalActionResult::Read { output, .. } =
            fixture.action(json!({"action":"read","session_id":id.as_str(),"cursor_segment":1}))
        else {
            panic!("retained output receipt");
        };
        assert!(output.windows(b"CLOSE_RETAINED".len()).any(|bytes| bytes == b"CLOSE_RETAINED"));
        assert_eq!(std::fs::read_to_string(fixture.root.join("workspace/close-count")).unwrap(), "once\n");
    }
}
