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
use crate::{
    NativeOwnedWorkerCompletion, NativeOwnedWorkerScope, NativeSandboxLaunch,
    NativeTerminalPermissionPolicy,
};
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

#[path = "terminal_host_lifecycle.rs"]
mod lifecycle;
use lifecycle::TerminalAccessPrincipals;
pub(crate) use lifecycle::TerminalAccessRoutes;
pub use lifecycle::{
    NativeTerminalHandoffReceipt, NativeTerminalLifecycleRequester, NativeTerminalResetEntry,
    NativeTerminalResetOutcome, NativeTerminalResetReceipt, NativeTerminalTransitionError,
};

struct HostState {
    catalogs: TerminalHostCatalogs,
    probes: TerminalHostProbes,
    access: TerminalAccessRoutes,
}
impl TerminalCatalogState for HostState {
    fn catalogs(&mut self) -> &mut TerminalHostCatalogs {
        &mut self.catalogs
    }
    fn access(&self) -> Option<&dyn terminal_host_dispatch::TerminalAccessView> {
        Some(&self.access)
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
    pub(crate) fn lifecycle_requester(&self) -> NativeTerminalLifecycleRequester {
        NativeTerminalLifecycleRequester {
            requester: self.runtime.requester(),
        }
    }
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
        let captured = TerminalCapturedExec::new(
            inputs.cli_executable.clone(),
            vec![TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
            FOREGROUND_TIMEOUT,
            MAX_EFFECTS,
        )
        .map_err(|_| unavailable())?;
        let captured = captured.with_worker_scope(workers.clone());
        let host = Arc::new(
            TerminalHostAuthority::new(inputs)?
                .capture_on_worker(deadline, &CancellationToken::new())?,
        );
        #[cfg(target_os = "macos")]
        let captured = captured.with_inventory_registration(
            host.launch_config()
                .pty_helper
                .inventory_helper()
                .ok_or_else(unavailable)?
                .clone(),
        );
        let captured = Arc::new(captured);
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
        Self::compose(inputs, state_root, host_identity, None)
    }

    pub(crate) fn compose_with_permission_on_worker(
        inputs: TerminalHostAuthorityInputs,
        state_root: OwnedFd,
        host_identity: SessionIncarnationId,
        permission: Arc<NativeTerminalPermissionPolicy>,
    ) -> Result<(TerminalActionTool, NativeTerminalHostResource), ToolError> {
        Self::compose(inputs, state_root, host_identity, Some(permission))
    }

    fn compose(
        inputs: TerminalHostAuthorityInputs,
        state_root: OwnedFd,
        host_identity: SessionIncarnationId,
        permission: Option<Arc<NativeTerminalPermissionPolicy>>,
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
        let principals = TerminalAccessPrincipals::default();
        let owner_principals = principals.clone();
        let runtime = TerminalRuntime::new(
            move || {
                let store = TerminalProfileStore::prepare(state_root)
                    .map_err(|_| TerminalRuntimeError::Initialization)?;
                let registry = TerminalRegistry::new(workspace.clone())
                    .map_err(|_| TerminalRuntimeError::Initialization)?;
                let budget = TerminalProfileBudget::new(TerminalProfileLimits::default())
                    .map_err(|_| TerminalRuntimeError::Initialization)?;
                let state = HostState {
                    access: TerminalAccessRoutes::new(owner_principals),
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
        let permission_resolver = Arc::new(crate::permission_targets::HostPermissionResolver::new(
            Arc::clone(&host),
            workers.clone(),
            stop.clone(),
        ));
        let executor = NativeTerminalActionExecutor {
            principals,
            access: None,
            permission,
            requester,
            host,
            preparer,
            starter,
            captured,
            stop: stop.clone(),
            active: Arc::new(AtomicUsize::new(0)),
            workers: workers.clone(),
        };
        let tool = TerminalActionTool::new(Arc::new(executor), identity)?
            .with_permission_resolver(permission_resolver);
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
    principals: TerminalAccessPrincipals,
    access: Option<CancellationToken>,
    permission: Option<Arc<NativeTerminalPermissionPolicy>>,
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
        let mut host = self.clone();
        Box::pin(async move {
            check(&cancellation, &host.stop)?;
            let mut authority = resident_authority(context.clone());
            let owner = authority.owner.clone();
            let (access, writer) = host.principals.acquire(owner).map_err(|_| unavailable())?;
            authority.writer = writer;
            host.access = Some(access.clone());
            let effective = CancellationToken::new();
            let _cancel = CancelOnDrop(effective.clone());
            let operation = host.execute_scoped(context, authority, invocation, effective.clone());
            match select(
                operation,
                select(cancellation.cancelled(), access.cancelled()),
            )
            .await
            {
                Either::Left((result, _)) => result,
                Either::Right((_, operation)) => {
                    effective.cancel();
                    operation.await
                }
            }
        })
    }
}

impl NativeTerminalActionExecutor {
    fn execute_scoped(
        self,
        context: ToolContext,
        authority: TerminalResidentAuthority,
        invocation: TerminalActionInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalActionResult, ToolError>> {
        Box::pin(async move {
            let host = self;
            host.check_access()?;
            if invocation.action() == TerminalAction::Exec {
                return host.exec(context, invocation, cancellation).await;
            }
            if matches!(
                invocation.action(),
                TerminalAction::Start | TerminalAction::Monitor
            ) || invocation.has_workspace_filter()
            {
                return host
                    .owned_action(context, authority, invocation, cancellation)
                    .await;
            }
            let request = invocation.resolve_cwd(|_| Err(unavailable()))?;
            let reply = terminal_host_dispatch::dispatch_with_access(
                host.requester,
                authority,
                request,
                TerminalMonitorActivation::default(),
                cancellation,
                host.access,
            )
            .await
            .map_err(dispatch_error)?;
            Ok(reply_result(reply))
        })
    }
}

impl NativeTerminalActionExecutor {
    fn check_access(&self) -> Result<(), ToolError> {
        if self
            .access
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            Err(cancelled())
        } else {
            Ok(())
        }
    }
    fn exec(
        &self,
        context: ToolContext,
        invocation: TerminalActionInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalActionResult, ToolError>> {
        let host = Arc::clone(&self.host);
        let permission = self.permission.clone();
        let access = self.access.clone();
        let future = self.captured.execute_prepared(
            move |deadline, cancellation, stop| {
                check(cancellation, stop).map_err(|_| TerminalCapturedExecError::Cancelled)?;
                if access.as_ref().is_some_and(CancellationToken::is_cancelled) {
                    return Err(TerminalCapturedExecError::Cancelled);
                }
                let resolved = host
                    .resolve_on_worker(invocation, deadline, cancellation)
                    .map_err(|_| TerminalCapturedExecError::Invalid)?;
                let TerminalActionRequest::Exec { request } = resolved.request else {
                    return Err(TerminalCapturedExecError::Invalid);
                };
                let mut shell = host
                    .exec_shell(&request)
                    .map_err(|_| TerminalCapturedExecError::Invalid)?;
                if let Some(permission) = permission {
                    let sandbox = permission
                        .capture_on_worker(&context, deadline, cancellation)
                        .map_err(|error| match error {
                            crate::NativeSandboxError::Cancelled => {
                                TerminalCapturedExecError::Cancelled
                            }
                            crate::NativeSandboxError::Timeout => {
                                TerminalCapturedExecError::Process
                            }
                            _ => TerminalCapturedExecError::Invalid,
                        })?;
                    shell = shell.with_sandbox(sandbox);
                }
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
        context: ToolContext,
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
                            context,
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
        context: ToolContext,
        authority: TerminalResidentAuthority,
        invocation: TerminalActionInvocation,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
        Box::pin(async move {
            check(&cancellation, &self.stop)?;
            self.check_access()?;
            let resolved = self
                .host
                .resolve_on_worker(invocation, deadline, &cancellation)?;
            let needs_launch = match &resolved.request {
                TerminalActionRequest::Start { .. } => true,
                TerminalActionRequest::Monitor {
                    operation:
                        TerminalMonitorOperation::Add { definition }
                        | TerminalMonitorOperation::Update { definition, .. },
                    ..
                } => matches!(
                    definition.condition,
                    machine_god_core::TerminalMonitorCondition::CustomProbe { .. }
                ),
                _ => false,
            };
            let sandbox = if needs_launch {
                self.permission
                    .as_ref()
                    .map(|permission| {
                        permission
                            .capture_on_worker(&context, deadline, &cancellation)
                            .map_err(|error| match error {
                                crate::NativeSandboxError::Cancelled => cancelled(),
                                _ => unavailable(),
                            })
                    })
                    .transpose()?
            } else {
                None
            };
            match resolved.request {
                TerminalActionRequest::Start { request } => {
                    let cwd = resolved.cwd.ok_or_else(unavailable)?;
                    self.start_on_worker(authority, request, cwd, sandbox, deadline, cancellation)
                        .await
                }
                TerminalActionRequest::Monitor {
                    session_id,
                    operation,
                } => {
                    self.monitor_on_worker(
                        authority,
                        session_id,
                        operation,
                        sandbox,
                        deadline,
                        cancellation,
                    )
                    .await
                }
                request @ TerminalActionRequest::List { .. } => {
                    terminal_host_dispatch::dispatch_with_access(
                        self.requester.clone(),
                        authority,
                        request,
                        TerminalMonitorActivation::default(),
                        cancellation,
                        self.access.clone(),
                    )
                    .await
                    .map(reply_result)
                    .map_err(dispatch_error)
                }
                _ => Err(unavailable()),
            }
        })
    }

    async fn admit_start_access(
        &self,
        owner: BackgroundOutputOwner,
        id: TerminalSessionId,
        cancellation: CancellationToken,
    ) -> Result<(), ToolError> {
        let access = self.access.clone();
        self.requester
            .request_with_context(cancellation, move |context| {
                if access.as_ref().is_some_and(CancellationToken::is_cancelled)
                    || !context
                        .state
                        .access
                        .resolve(&owner, &id)
                        .is_ok_and(|storage| storage == owner)
                {
                    Err(unavailable())
                } else {
                    Ok(())
                }
            })
            .await
            .map_err(|_| unavailable())?
    }

    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "ordered startup keeps explicit authority and retained receipts adjacent"
    )]
    async fn start_on_worker(
        &self,
        authority: TerminalResidentAuthority,
        request: TerminalStartRequest,
        cwd: OwnedFd,
        sandbox: Option<Arc<NativeSandboxLaunch>>,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<TerminalActionResult, ToolError> {
        let id = new_session_id()?;
        self.admit_start_access(authority.owner.clone(), id.clone(), cancellation.clone())
            .await?;
        let mut prepared = Vec::with_capacity(request.initial_monitors.len());
        for definition in &request.initial_monitors {
            check(&cancellation, &self.stop)?;
            prepared.push(self.preparer.prepare_on_worker(
                definition,
                &request.cwd,
                sandbox.clone(),
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
                sandbox,
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
                self.access.clone(),
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
        let reply = terminal_host_dispatch::dispatch_with_access(
            self.requester.clone(),
            authority,
            wait,
            TerminalMonitorActivation::default(),
            cancellation.clone(),
            self.access.clone(),
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

    async fn monitor_cwd(
        &self,
        owner: BackgroundOutputOwner,
        session: TerminalSessionId,
        cancellation: CancellationToken,
    ) -> Result<String, ToolError> {
        let access = self.access.clone();
        self.requester
            .request_with_context(cancellation, move |context| {
                if access.as_ref().is_some_and(CancellationToken::is_cancelled) {
                    return None;
                }
                let owner = context.state.access.resolve(&owner, &session).ok()?;
                context
                    .registry
                    .inspect(&owner, &session)
                    .ok()
                    .and_then(|facts| facts.metadata.map(|metadata| metadata.cwd))
            })
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)
    }

    async fn monitor_on_worker(
        &self,
        authority: TerminalResidentAuthority,
        id: TerminalSessionId,
        operation: TerminalMonitorOperation,
        sandbox: Option<Arc<NativeSandboxLaunch>>,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<TerminalActionResult, ToolError> {
        let preparation = match &operation {
            TerminalMonitorOperation::Add { definition }
            | TerminalMonitorOperation::Update { definition, .. } => {
                let cwd = self
                    .monitor_cwd(authority.owner.clone(), id.clone(), cancellation.clone())
                    .await?;
                Some(self.preparer.prepare_on_worker(
                    definition,
                    &cwd,
                    sandbox,
                    deadline,
                    &cancellation,
                )?)
            }
            _ => None,
        };
        check(&cancellation, &self.stop)?;
        let access = self.access.clone();
        self.requester
            .request_with_context(cancellation, move |mut context| {
                if access.as_ref().is_some_and(CancellationToken::is_cancelled) {
                    return Err(cancelled());
                }
                let mut authority = authority;
                authority.owner = context
                    .state
                    .access
                    .resolve(&authority.owner, &id)
                    .map_err(|_| unavailable())?;
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
mod tests;
