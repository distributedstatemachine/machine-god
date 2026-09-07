//! Staged native startup: admission and publication borrow the owner briefly;
//! preparation and all rollback waits belong to a collected effect worker.

use crate::terminal_catalog_view::TerminalCatalogViewError;
use crate::terminal_history::{TerminalHistory, TerminalHistoryError};
use crate::terminal_host_catalog::TerminalHostCatalogs;
use crate::terminal_journal::TerminalJournalLimits;
use crate::terminal_monitor::{TerminalMonitorActivation, TerminalMonitorMutation};
use crate::terminal_native_backend::TerminalNativeBackend;
use crate::terminal_native_launch::{
    ResolvedTerminalNativeLaunch, TerminalNativeLaunchAuthority, TerminalNativeLaunchConfig,
    TerminalNativeLaunchError, TerminalNativeLaunchIdentity,
};
use crate::terminal_owner::{TerminalOwnerContext, TerminalOwnerError};
use crate::terminal_profile::{TerminalProfileError, TerminalProfileMutationContext};
use crate::terminal_registry::{
    TerminalRegistry, TerminalRegistryError, TerminalResidentLease, TerminalStartReservation,
};
use crate::terminal_resident_dispatch::TerminalResidentAuthority;
use crate::terminal_runtime::{TerminalRuntimeError, TerminalRuntimeRequester};
use crate::terminal_session::{TerminalSession, TerminalSessionBackend, TerminalSessionError};
use crate::terminal_session_record::TerminalSessionMetadata;
use crate::terminal_startup::TerminalStartupControl;
use crate::{NativeOwnedWorkerScope, NativeOwnedWorkerSpawner};
use machine_god_core::{
    BoxFuture, CancellationToken, SessionIncarnationId, TerminalMonitorOperation,
    TerminalSessionFacts, TerminalSessionId, TerminalStartRequest,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

const MAX_STAGED_STARTS: usize = 16;
const ROLLBACK_BACKOFF: Duration = Duration::from_millis(10);

enum StartDeadline {
    #[cfg(test)]
    Relative(Duration),
    Absolute(Instant),
}
impl StartDeadline {
    fn resolve(self) -> Result<Instant> {
        let now = Instant::now();
        let (deadline, duration) = match self {
            #[cfg(test)]
            Self::Relative(duration) => {
                if duration.is_zero() {
                    return Err(TerminalStagedStartError::Invalid);
                }
                (now.checked_add(duration), duration)
            }
            Self::Absolute(deadline) => (Some(deadline), deadline.saturating_duration_since(now)),
        };
        if duration.is_zero() {
            return Err(TerminalStagedStartError::Launch(
                TerminalNativeLaunchError::Timeout,
            ));
        }
        if duration > crate::terminal_helper::MAX_STARTUP_TIMEOUT {
            return Err(TerminalStagedStartError::Invalid);
        }
        deadline.ok_or(TerminalStagedStartError::Invalid)
    }
}

#[derive(Debug)]
pub(crate) enum TerminalStagedStartError {
    Invalid,
    Capacity,
    Cancelled,
    Worker,
    Runtime(TerminalRuntimeError),
    Registry(TerminalRegistryError),
    Catalog(TerminalCatalogViewError),
    Launch(TerminalNativeLaunchError),
    /// Admission created a registry-owned session but did not complete a valid
    /// start. Its failed publication/native cleanup remains on that owner.
    Retained {
        session_id: TerminalSessionId,
        error: TerminalRegistryError,
    },
}
type Result<T> = std::result::Result<T, TerminalStagedStartError>;

/// Durable initial admission, not proof that the shell/command has started.
/// The enclosing host applies its requested attention wait separately.
#[derive(Debug)]
pub(crate) struct TerminalStagedStartReceipt {
    pub(crate) session: TerminalSessionFacts,
    /// Retain through the enclosing host's attention wait and final projection.
    pub(crate) residency: TerminalResidentLease,
}

/// No host-lifetime vote, host-state value or mutable native backend lives here.
/// The accessor borrows catalogs only while executing on the runtime owner.
pub(crate) struct TerminalStagedStarter<S: 'static> {
    requester: TerminalRuntimeRequester<TerminalNativeBackend, S>,
    config: Arc<TerminalNativeLaunchConfig>,
    host_identity: SessionIncarnationId,
    catalogs: fn(&mut S) -> &mut TerminalHostCatalogs,
    active: Arc<AtomicUsize>,
    worker_scope: Option<NativeOwnedWorkerScope>,
}
impl<S: 'static> TerminalStagedStarter<S> {
    pub(crate) fn new(
        requester: TerminalRuntimeRequester<TerminalNativeBackend, S>,
        config: Arc<TerminalNativeLaunchConfig>,
        host_identity: SessionIncarnationId,
        catalogs: fn(&mut S) -> &mut TerminalHostCatalogs,
    ) -> Self {
        Self {
            requester,
            config,
            host_identity,
            catalogs,
            active: Arc::new(AtomicUsize::new(0)),
            worker_scope: None,
        }
    }
    pub(crate) fn with_worker_scope(mut self, scope: NativeOwnedWorkerScope) -> Self {
        self.worker_scope = Some(scope);
        self
    }

    /// Inert until polled. Authority acquisition runs on the effect worker only
    /// AFTER resident reservation. Initial monitor activation evidence must be
    /// supplied by separately authorized host work; it grants no probe effects.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit native and actor authority"
    )]
    #[cfg(test)]
    pub(crate) fn start(
        &self,
        authority: TerminalResidentAuthority,
        session_id: TerminalSessionId,
        request: TerminalStartRequest,
        prepare_authority: impl FnOnce() -> std::result::Result<
            TerminalNativeLaunchAuthority,
            TerminalNativeLaunchError,
        > + Send
        + 'static,
        activations: Vec<TerminalMonitorActivation>,
        install_monitors: impl FnOnce(
            &mut S,
            Vec<TerminalMonitorMutation>,
        ) -> std::result::Result<(), TerminalSessionError>
        + Send
        + 'static,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalStagedStartReceipt>> {
        self.start_with_deadline(
            authority,
            session_id,
            request,
            prepare_authority,
            activations,
            |_, _| {},
            install_monitors,
            StartDeadline::Relative(timeout),
            cancellation,
        )
    }

    /// Preserve the host's original deadline through prior owned preparation;
    /// polling never reconstructs a later deadline from a remaining duration.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit native and actor authority"
    )]
    pub(crate) fn start_until(
        &self,
        authority: TerminalResidentAuthority,
        session_id: TerminalSessionId,
        request: TerminalStartRequest,
        prepare_authority: impl FnOnce() -> std::result::Result<
            TerminalNativeLaunchAuthority,
            TerminalNativeLaunchError,
        > + Send
        + 'static,
        activations: Vec<TerminalMonitorActivation>,
        reconcile_monitors: fn(&mut S, &TerminalRegistry<TerminalNativeBackend>),
        install_monitors: impl FnOnce(
            &mut S,
            Vec<TerminalMonitorMutation>,
        ) -> std::result::Result<(), TerminalSessionError>
        + Send
        + 'static,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalStagedStartReceipt>> {
        self.start_with_deadline(
            authority,
            session_id,
            request,
            prepare_authority,
            activations,
            reconcile_monitors,
            install_monitors,
            StartDeadline::Absolute(deadline),
            cancellation,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "explicit native and actor authority"
    )]
    fn start_with_deadline(
        &self,
        authority: TerminalResidentAuthority,
        session_id: TerminalSessionId,
        request: TerminalStartRequest,
        prepare_authority: impl FnOnce() -> std::result::Result<
            TerminalNativeLaunchAuthority,
            TerminalNativeLaunchError,
        > + Send
        + 'static,
        activations: Vec<TerminalMonitorActivation>,
        reconcile_monitors: fn(&mut S, &TerminalRegistry<TerminalNativeBackend>),
        install_monitors: impl FnOnce(
            &mut S,
            Vec<TerminalMonitorMutation>,
        ) -> std::result::Result<(), TerminalSessionError>
        + Send
        + 'static,
        deadline: StartDeadline,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalStagedStartReceipt>> {
        let requester = self.requester.clone();
        let config = Arc::clone(&self.config);
        let host_identity = self.host_identity.clone();
        let catalogs = self.catalogs;
        let active = Arc::clone(&self.active);
        let worker_scope = self.worker_scope.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(TerminalStagedStartError::Cancelled);
            }
            if activations.len() != request.initial_monitors.len() {
                return Err(TerminalStagedStartError::Invalid);
            }
            let deadline = deadline.resolve()?;
            let resolved = ResolvedTerminalNativeLaunch::resolve(&config, &request)
                .map_err(TerminalStagedStartError::Launch)?;
            active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < MAX_STAGED_STARTS).then_some(count + 1)
                })
                .map_err(|_| TerminalStagedStartError::Capacity)?;
            let permit = StartPermit(active);
            let stop = CancellationToken::new();
            let cancel_on_drop = CancelOnDrop(stop.clone());
            let worker_stop = stop.clone();
            let operation = move || {
                let request = StartPublication {
                    authority,
                    session_id,
                    request,
                    activations,
                    install_monitors: Box::new(install_monitors),
                    reconcile_monitors,
                    host_identity,
                    deadline,
                };
                let result =
                    run_staged(&requester, request, catalogs, worker_stop, deadline, || {
                        let native_authority = prepare_authority()?;
                        resolved
                            .prepare(&config, native_authority, deadline, &stop)?
                            .commit(&stop)
                    });
                (result, permit)
            };
            let result = match worker_scope {
                Some(scope) => scope.run(operation),
                None => NativeOwnedWorkerSpawner::new().run(operation),
            };
            // Dropping the outer future cancels only preparation. An admitted
            // worker retains rollback and cleanup; successful registration gives
            // the startup controller an independent registry-owned token.
            let receipt = match futures_util::future::select(
                result,
                Box::pin(cancellation.cancelled()),
            )
            .await
            {
                futures_util::future::Either::Left((result, _)) => result,
                futures_util::future::Either::Right(((), result)) => {
                    cancel_on_drop.0.cancel();
                    result.await
                }
            }
            .map_err(|_| TerminalStagedStartError::Worker)?;
            receipt.0
        })
    }
}

struct StartPermit(Arc<AtomicUsize>);
impl Drop for StartPermit {
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

type InstallMonitors<S> = Box<
    dyn FnOnce(
            &mut S,
            Vec<TerminalMonitorMutation>,
        ) -> std::result::Result<(), TerminalSessionError>
        + Send,
>;

struct StartPublication<B: TerminalSessionBackend, S> {
    authority: TerminalResidentAuthority,
    session_id: TerminalSessionId,
    request: TerminalStartRequest,
    activations: Vec<TerminalMonitorActivation>,
    install_monitors: InstallMonitors<S>,
    reconcile_monitors: fn(&mut S, &TerminalRegistry<B>),
    host_identity: SessionIncarnationId,
    deadline: Instant,
}

/// Constructed, polled and dropped exclusively on the effect worker. In
/// particular no closure carrying native backend ownership crosses the outer
/// tool future's poll/Drop path, including owner rejection before queueing.
fn run_staged<B: TerminalSessionBackend + Send + 'static, S: 'static>(
    requester: &TerminalRuntimeRequester<B, S>,
    publication: StartPublication<B, S>,
    catalogs: fn(&mut S) -> &mut TerminalHostCatalogs,
    cancellation: CancellationToken,
    deadline: Instant,
    prepare: impl FnOnce() -> std::result::Result<
        (B, TerminalStartupControl, TerminalNativeLaunchIdentity),
        TerminalNativeLaunchError,
    >,
) -> Result<TerminalStagedStartReceipt> {
    let owner = publication.authority.owner.clone();
    let id = publication.session_id.clone();
    let preparation_stop = cancellation.clone();
    let reservation = futures_executor::block_on(requester.request_with_context(
        cancellation.clone(),
        move |context| {
            if Instant::now() >= deadline {
                return Err(TerminalStagedStartError::Launch(
                    TerminalNativeLaunchError::Timeout,
                ));
            }
            context
                .registry
                .reserve_start_with_cancellation(owner, id, preparation_stop)
                .map_err(TerminalStagedStartError::Registry)
        },
    ))
    .map_err(TerminalStagedStartError::Runtime)??;
    let reservation = Arc::new(reservation);
    let mut rollback = StartRollback {
        requester: requester.clone(),
        reservation: Arc::clone(&reservation),
        armed: true,
    };
    if cancellation.is_cancelled() {
        return Err(TerminalStagedStartError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(TerminalStagedStartError::Launch(
            TerminalNativeLaunchError::Timeout,
        ));
    }
    let prepared = prepare().map_err(TerminalStagedStartError::Launch)?;
    // The callback's cancellation check happens on the owner. A fresh request
    // token ensures cancellation cannot discard a queued committed receipt.
    let result = futures_executor::block_on(requester.request_with_context(
        CancellationToken::new(),
        move |mut context| {
            publish_start(
                &mut context,
                &reservation,
                publication,
                catalogs,
                prepared,
                &cancellation,
            )
        },
    ))
    .map_err(TerminalStagedStartError::Runtime)?;
    if matches!(
        result,
        Ok(_) | Err(TerminalStagedStartError::Retained { .. })
    ) {
        rollback.armed = false;
    }
    result
}

struct StartRollback<B: TerminalSessionBackend + Send + 'static, S: 'static> {
    requester: TerminalRuntimeRequester<B, S>,
    reservation: Arc<TerminalStartReservation>,
    armed: bool,
}
impl<B: TerminalSessionBackend + Send + 'static, S: 'static> Drop for StartRollback<B, S> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            loop {
                let reservation = Arc::clone(&self.reservation);
                let result = futures_executor::block_on(
                    self.requester
                        .request_with_context(CancellationToken::new(), move |context| {
                            context.registry.withdraw_reserved_start(&reservation)
                        }),
                );
                if matches!(
                    result,
                    Err(TerminalRuntimeError::Owner(TerminalOwnerError::Busy))
                ) {
                    // A full user reply queue cannot abandon a reserved slot. The
                    // non-owning requester does not prevent host shutdown, which
                    // invalidates all pending reservations and ends this retry.
                    std::thread::sleep(ROLLBACK_BACKOFF);
                } else {
                    break;
                }
            }
        }));
        if let Err(payload) = result {
            std::mem::forget(payload);
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "ordered durable admission and retained failures remain one owner transaction"
)]
fn publish_start<B: TerminalSessionBackend + Send + 'static, S>(
    context: &mut TerminalOwnerContext<'_, B, S>,
    reservation: &TerminalStartReservation,
    publication: StartPublication<B, S>,
    catalogs: fn(&mut S) -> &mut TerminalHostCatalogs,
    prepared: (B, TerminalStartupControl, TerminalNativeLaunchIdentity),
    cancellation: &CancellationToken,
) -> Result<TerminalStagedStartReceipt> {
    if cancellation.is_cancelled() {
        return Err(TerminalStagedStartError::Cancelled);
    }
    let (backend, control, identity) = prepared;
    if Instant::now() >= publication.deadline {
        return Err(TerminalStagedStartError::Launch(
            TerminalNativeLaunchError::Timeout,
        ));
    }
    let owner = &publication.authority.owner;
    let id = &publication.session_id;
    let metadata = TerminalSessionMetadata {
        host_identity: publication.host_identity,
        backend_identity: identity.backend_identity,
        shell: identity.shell,
        workspace: context.registry.workspace().into(),
        cwd: identity.cwd,
        command: identity.command,
        backend: identity.backend,
        profile: identity.profile,
    };
    let catalog = catalogs(context.state)
        .catalog(context.store, owner, cancellation)
        .map_err(TerminalStagedStartError::Catalog)?;
    let mut transaction = context.store.transaction().map_err(|error| {
        TerminalStagedStartError::Catalog(TerminalCatalogViewError::Profile(error))
    })?;
    drop(transaction.create_session(catalog, id).map_err(|error| {
        TerminalStagedStartError::Catalog(TerminalCatalogViewError::Profile(error))
    })?);
    let journal = context
        .budget
        .create_journal(
            &mut transaction,
            catalog.namespace_key(),
            id,
            TerminalJournalLimits::default(),
        )
        .map_err(profile_error)?;
    // The journal remains charged even if its independent accounting failed.
    journal.accounting.map_err(profile_error)?;
    let journal = journal.operation.map_err(|error| {
        TerminalStagedStartError::Catalog(TerminalCatalogViewError::Journal(error))
    })?;
    let namespace = catalog.namespace_key().to_owned();
    let mut persistence =
        TerminalProfileMutationContext::new(&mut transaction, *context.budget, &namespace);
    let history = TerminalHistory::create_with(&mut persistence, journal, &identity.dimensions)
        .map_err(|error| {
            TerminalStagedStartError::Catalog(TerminalCatalogViewError::History(error))
        })?;
    if cancellation.is_cancelled() {
        return Err(TerminalStagedStartError::Cancelled);
    }
    if Instant::now() >= publication.deadline {
        return Err(TerminalStagedStartError::Launch(
            TerminalNativeLaunchError::Timeout,
        ));
    }
    // Only live metadata may prune retained grants. This runs in the same owner
    // callback as installation, after asynchronous preparation and before the
    // registry borrow used to construct the new session. No pump can intervene.
    (publication.reconcile_monitors)(context.state, context.registry);
    let mut admission_error = None;
    context
        .registry
        .commit_reserved_start(reservation, || {
            let (mut session, initial_error) = TerminalSession::new_retained_with(
                &mut persistence,
                backend,
                history,
                owner.clone(),
                id.clone(),
                metadata,
                context.now_ms,
            )?;
            admission_error = initial_error;
            if initial_error.is_none() {
                if let Err(error) =
                    session.attach_startup_control(control, CancellationToken::new())
                {
                    let _ = session.teardown_without_persistence(true, context.now_ms, error);
                    admission_error = Some(error);
                } else {
                    let mut mutations = Vec::with_capacity(publication.activations.len());
                    for (definition, activation) in publication
                        .request
                        .initial_monitors
                        .into_iter()
                        .zip(publication.activations)
                    {
                        let mutation = session.monitor_with(
                            &mut persistence,
                            owner,
                            TerminalMonitorOperation::Add { definition },
                            activation,
                            context.now_ms,
                        );
                        match mutation {
                            Ok(mutation) => mutations.push(mutation),
                            Err(error) => {
                                session.fail_pending_startup_with(
                                    &mut persistence,
                                    context.now_ms,
                                    error,
                                );
                                admission_error = Some(error);
                                break;
                            }
                        }
                    }
                    if admission_error.is_none()
                        && let Err(error) = (publication.install_monitors)(context.state, mutations)
                    {
                        session.fail_pending_startup_with(&mut persistence, context.now_ms, error);
                        admission_error = Some(error);
                    }
                }
            }
            Ok(session)
        })
        .map_err(TerminalStagedStartError::Registry)?;
    if let Some(error) = admission_error {
        return Err(TerminalStagedStartError::Retained {
            session_id: id.clone(),
            error: TerminalRegistryError::Session(error),
        });
    }
    let session = context.registry.project_facts_with(
        &mut persistence,
        owner,
        id,
        publication.authority.actor,
        &publication.authority.controls,
    );
    let session = match session {
        Ok(session) => session,
        Err(error) => {
            if let TerminalRegistryError::Session(session_error) = error
                && let Ok(session) = context.registry.live_mut(owner, id)
            {
                session.fail_pending_startup_with(&mut persistence, context.now_ms, session_error);
            }
            return Err(TerminalStagedStartError::Retained {
                session_id: id.clone(),
                error,
            });
        }
    };
    let residency = context
        .registry
        .lease(owner, id)
        .map_err(TerminalStagedStartError::Registry)?;
    Ok(TerminalStagedStartReceipt { session, residency })
}

fn profile_error(error: TerminalProfileError) -> TerminalStagedStartError {
    TerminalStagedStartError::Registry(TerminalRegistryError::Session(
        TerminalSessionError::History(TerminalHistoryError::Profile(error)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_input::TerminalWriterId;
    use crate::terminal_profile::{TerminalProfileBudget, TerminalProfileLimits};
    use crate::terminal_profile_store::TerminalProfileStore;
    use crate::terminal_pty::TerminalPtyHelper;
    use crate::terminal_registry::TerminalRegistry;
    use crate::terminal_runtime::{
        TerminalRuntime, TerminalRuntimeJob, TerminalRuntimeSpawner, TerminalRuntimeWorker,
    };
    use machine_god_core::{
        BackgroundOutputOwner, SessionId, TerminalActorRole, TerminalAllowedControls,
        TerminalMonitorCondition, TerminalMonitorDefinition, TerminalMonitorLifetime,
        TerminalNotifySchedule, TerminalProfile, TerminalSchedule,
    };
    use rustix::fd::OwnedFd;
    use rustix::fs::{Mode, OFlags};
    use std::future::Future;
    use std::num::NonZeroU64;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::{Mutex, mpsc};
    use std::task::{Context, Poll, Waker};

    #[derive(Default)]
    struct Spawner(Mutex<Vec<std::thread::JoinHandle<()>>>);
    impl TerminalRuntimeSpawner for Spawner {
        fn spawn(&self, job: TerminalRuntimeJob) -> std::result::Result<(), ()> {
            self.0.lock().unwrap().push(std::thread::spawn(job));
            Ok(())
        }
    }
    impl Spawner {
        fn join(&self) {
            for thread in self.0.lock().unwrap().drain(..) {
                thread.join().unwrap();
            }
        }
    }
    fn fd(path: &std::path::Path) -> OwnedFd {
        rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap()
    }
    struct FixtureState {
        catalogs: TerminalHostCatalogs,
        reconciled: bool,
    }
    struct Fixture {
        runtime: Option<TerminalRuntime<TerminalNativeBackend, FixtureState>>,
        starter: TerminalStagedStarter<FixtureState>,
        spawner: Arc<Spawner>,
        path: PathBuf,
        initialized: Arc<AtomicUsize>,
        output: Arc<Mutex<Vec<u8>>>,
        monitor_installations: Arc<AtomicUsize>,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = PathBuf::from("/tmp")
                .join(format!("mg-stage-{:032x}", u128::from_le_bytes(random)));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            for name in ["profile", "artifacts"] {
                std::fs::create_dir(path.join(name)).unwrap();
                std::fs::set_permissions(path.join(name), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
            }
            let path = std::fs::canonicalize(path).unwrap();
            let store_fd = fd(&path.join("profile"));
            let workspace = path.to_str().unwrap().to_owned();
            let spawner = Arc::new(Spawner::default());
            let initialized = Arc::new(AtomicUsize::new(0));
            let output = Arc::new(Mutex::new(Vec::new()));
            let monitor_installations = Arc::new(AtomicUsize::new(0));
            let installed = Arc::clone(&monitor_installations);
            let init = Arc::clone(&initialized);
            let observed = Arc::clone(&output);
            let runtime = TerminalRuntime::new(
                move || {
                    init.fetch_add(1, Ordering::AcqRel);
                    let started = Instant::now();
                    Ok(TerminalRuntimeWorker::new_with_state(
                        TerminalRegistry::new(workspace.clone()).unwrap(),
                        TerminalProfileStore::prepare(store_fd).unwrap(),
                        TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap(),
                        FixtureState {
                            catalogs: TerminalHostCatalogs::new(workspace).unwrap(),
                            reconciled: false,
                        },
                        move || i64::try_from(started.elapsed().as_millis()).unwrap(),
                        move |_, steps| {
                            for step in steps {
                                if let Ok(step) = step.result {
                                    if !step.probes.is_empty() {
                                        assert!(installed.load(Ordering::Acquire) > 0);
                                    }
                                    let mut output = observed.lock().unwrap();
                                    output.extend(step.output);
                                    assert!(output.len() < 1024 * 1024);
                                }
                            }
                        },
                    ))
                },
                spawner.clone(),
            );
            let starter = TerminalStagedStarter::new(
                runtime.requester(),
                Arc::new(config()),
                SessionIncarnationId::new("native-host").unwrap(),
                |state| &mut state.catalogs,
            );
            Self {
                runtime: Some(runtime),
                starter,
                spawner,
                path,
                initialized,
                output,
                monitor_installations,
            }
        }
        fn runtime(&self) -> &TerminalRuntime<TerminalNativeBackend, FixtureState> {
            self.runtime.as_ref().unwrap()
        }
        fn request(&self, command: Option<&str>) -> TerminalStartRequest {
            let mut request =
                TerminalStartRequest::interactive(self.path.to_str().unwrap()).unwrap();
            request.profile = Some(TerminalProfile::Clean);
            request.return_when =
                command.map(|_| machine_god_core::TerminalReturnCondition::Started);
            request.command = command.map(str::to_owned);
            request
        }
        fn native_authority(&self) -> TerminalNativeLaunchAuthority {
            TerminalNativeLaunchAuthority {
                environment: vec![
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    ("TERM".into(), "xterm-256color".into()),
                ],
                cwd: fd(&self.path),
                artifacts: fd(&self.path.join("artifacts")),
                artifact_path: self.path.join("artifacts"),
            }
        }
        fn assert_slot_released(&self) {
            futures_executor::block_on(self.runtime().request_with_context(
                CancellationToken::new(),
                |context| {
                    let token = context
                        .registry
                        .reserve_start(authority().owner, id())
                        .unwrap();
                    context.registry.withdraw_reserved_start(&token).unwrap();
                },
            ))
            .unwrap();
        }
        fn wait_idle(&self) {
            until(|| self.starter.active.load(Ordering::Acquire) == 0);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.runtime.take();
            self.spawner.join();
            self.wait_idle();
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn helper(mode: &str, entry: &str) -> TerminalPtyHelper {
        if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            TerminalPtyHelper::new(program.into(), vec![mode.into()])
                .unwrap()
                .with_test_inventory_helper()
        } else {
            TerminalPtyHelper::new(
                std::env::current_exe().unwrap(),
                vec![
                    "--exact".into(),
                    entry.into(),
                    "--test-threads=1".into(),
                    "--quiet".into(),
                ],
            )
            .unwrap()
            .with_test_inventory_helper()
        }
    }
    fn config() -> TerminalNativeLaunchConfig {
        TerminalNativeLaunchConfig {
            account_shell: Some("/bin/bash".into()),
            pty_helper: helper(
                crate::terminal_helper::TERMINAL_PTY_HELPER_ARGUMENT,
                "terminal_pty::tests::helper_entry",
            ),
            marker_helper: helper(
                crate::terminal_helper::TERMINAL_STARTUP_MARKER_ARGUMENT,
                "terminal_startup::tests::marker_helper_entry",
            ),
            tmux: None,
        }
    }
    fn authority() -> TerminalResidentAuthority {
        TerminalResidentAuthority {
            owner: BackgroundOutputOwner::new(
                SessionId::new("owner").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            ),
            actor: TerminalActorRole::Agent,
            writer: TerminalWriterId::new(NonZeroU64::new(1).unwrap()),
            controls: TerminalAllowedControls::default(),
            revoke_authorized: false,
        }
    }
    fn id() -> TerminalSessionId {
        TerminalSessionId::new("staged").unwrap()
    }
    fn until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(Instant::now() < deadline, "bounded observation timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn poll<T>(future: &mut (impl Future<Output = T> + Unpin)) -> Poll<T> {
        Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn scoped_staging_is_inert_and_closed_scope_never_reserves_or_prepares() {
        let fixture = Fixture::new();
        let scope = NativeOwnedWorkerScope::new();
        let starter = TerminalStagedStarter::new(
            fixture.runtime().requester(),
            Arc::clone(&fixture.starter.config),
            fixture.starter.host_identity.clone(),
            fixture.starter.catalogs,
        )
        .with_worker_scope(scope.clone());
        drop(starter.start(
            authority(),
            id(),
            fixture.request(None),
            || panic!("unpolled authority"),
            Vec::new(),
            |_, _| Ok(()),
            Duration::from_secs(5),
            CancellationToken::new(),
        ));
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert!(matches!(
            futures_executor::block_on(starter.start(
                authority(),
                id(),
                fixture.request(None),
                || panic!("closed scope authority"),
                Vec::new(),
                |_, _| Ok(()),
                Duration::from_secs(5),
                CancellationToken::new(),
            )),
            Err(TerminalStagedStartError::Worker)
        ));
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
        assert_eq!(starter.active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn staged_start_is_inert_and_precancellation_or_capacity_has_no_authority_effect() {
        let fixture = Fixture::new();
        drop(fixture.starter.start(
            authority(),
            id(),
            fixture.request(None),
            || panic!("unpolled authority"),
            Vec::new(),
            |_, _| Ok(()),
            Duration::from_secs(5),
            CancellationToken::new(),
        ));
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            futures_executor::block_on(fixture.starter.start(
                authority(),
                id(),
                fixture.request(None),
                || panic!("cancelled authority"),
                Vec::new(),
                |_, _| Ok(()),
                Duration::from_secs(5),
                cancellation
            )),
            Err(TerminalStagedStartError::Cancelled)
        ));
        fixture
            .starter
            .active
            .store(MAX_STAGED_STARTS, Ordering::Release);
        assert!(matches!(
            futures_executor::block_on(fixture.starter.start(
                authority(),
                id(),
                fixture.request(None),
                || panic!("capacity authority"),
                Vec::new(),
                |_, _| Ok(()),
                Duration::from_secs(5),
                CancellationToken::new()
            )),
            Err(TerminalStagedStartError::Capacity)
        ));
        fixture.starter.active.store(0, Ordering::Release);
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
    }

    #[test]
    fn reservation_precedes_authority_and_owner_remains_responsive_during_preparation() {
        let mut fixture = Fixture::new();
        let scope = NativeOwnedWorkerScope::new();
        fixture.starter.worker_scope = Some(scope.clone());
        let poll_thread = std::thread::current().id();
        let (entered, ready) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let mut start = fixture.starter.start(
            authority(),
            id(),
            fixture.request(None),
            move || {
                assert_ne!(std::thread::current().id(), poll_thread);
                entered.send(()).unwrap();
                gate.recv().unwrap();
                Err(TerminalNativeLaunchError::Process)
            },
            Vec::new(),
            |_, _| Ok(()),
            Duration::from_secs(5),
            CancellationToken::new(),
        );
        assert!(poll(&mut start).is_pending());
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        futures_executor::block_on(fixture.runtime().request_with_context(
            CancellationToken::new(),
            |context| {
                assert!(context.registry.owner_ids(&authority().owner).is_empty());
                assert!(matches!(
                    context.registry.reserve_start(authority().owner, id()),
                    Err(TerminalRegistryError::Conflict)
                ));
            },
        ))
        .unwrap();
        scope.close();
        assert!(!scope.completion().is_complete());
        release.send(()).unwrap();
        scope.completion().wait_on_worker().unwrap();
        assert_eq!(fixture.starter.active.load(Ordering::Acquire), 1);
        assert!(matches!(
            futures_executor::block_on(start),
            Err(TerminalStagedStartError::Launch(
                TerminalNativeLaunchError::Process
            ))
        ));
        fixture.assert_slot_released();
    }

    #[test]
    fn absolute_start_deadline_never_restarts_at_first_poll() {
        let fixture = Fixture::new();
        let deadline = Instant::now() + Duration::from_millis(10);
        let future = fixture.starter.start_until(
            authority(),
            id(),
            fixture.request(None),
            || panic!("expired authority"),
            Vec::new(),
            |_, _| {},
            |_, _| Ok(()),
            deadline,
            CancellationToken::new(),
        );
        std::thread::sleep(Duration::from_millis(20));
        assert!(matches!(
            futures_executor::block_on(future),
            Err(TerminalStagedStartError::Launch(
                TerminalNativeLaunchError::Timeout
            ))
        ));
        assert_eq!(fixture.initialized.load(Ordering::Acquire), 0);
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            StartDeadline::Absolute(deadline).resolve().unwrap(),
            deadline
        );
    }

    #[test]
    fn delayed_owner_admission_consumes_original_deadline_before_authority() {
        let fixture = Fixture::new();
        let (entered, ready) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let mut blocking =
            fixture
                .runtime()
                .request_with_context(CancellationToken::new(), move |_| {
                    entered.send(()).unwrap();
                    gate.recv().unwrap();
                });
        assert!(poll(&mut blocking).is_pending());
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut start = fixture.starter.start(
            authority(),
            id(),
            fixture.request(None),
            || panic!("expired staging must not acquire native authority"),
            Vec::new(),
            |_, _| Ok(()),
            Duration::from_millis(20),
            CancellationToken::new(),
        );
        assert!(poll(&mut start).is_pending());
        std::thread::sleep(Duration::from_millis(40));
        release.send(()).unwrap();
        futures_executor::block_on(blocking).unwrap();
        assert!(matches!(
            futures_executor::block_on(start),
            Err(TerminalStagedStartError::Launch(
                TerminalNativeLaunchError::Timeout
            ))
        ));
        fixture.assert_slot_released();
    }

    #[test]
    fn abandoned_preparation_keeps_worker_rollback_and_releases_reserved_slot() {
        let fixture = Fixture::new();
        let (entered, ready) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let mut start = fixture.starter.start(
            authority(),
            id(),
            fixture.request(None),
            move || {
                entered.send(()).unwrap();
                gate.recv().unwrap();
                Err(TerminalNativeLaunchError::Cancelled)
            },
            Vec::new(),
            |_, _| Ok(()),
            Duration::from_secs(5),
            CancellationToken::new(),
        );
        assert!(poll(&mut start).is_pending());
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(start);
        release.send(()).unwrap();
        fixture.wait_idle();
        fixture.assert_slot_released();
    }

    #[test]
    fn host_shutdown_does_not_wait_for_or_get_kept_alive_by_preparation() {
        let mut fixture = Fixture::new();
        let (entered, ready) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let native_authority = fixture.native_authority();
        let mut start = fixture.starter.start(
            authority(),
            id(),
            fixture.request(None),
            move || {
                entered.send(()).unwrap();
                gate.recv().unwrap();
                Ok(native_authority)
            },
            Vec::new(),
            |_, _| Ok(()),
            Duration::from_secs(5),
            CancellationToken::new(),
        );
        assert!(poll(&mut start).is_pending());
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        fixture.runtime.take();
        fixture.spawner.join();
        release.send(()).unwrap();
        assert!(matches!(
            futures_executor::block_on(start),
            Err(TerminalStagedStartError::Launch(
                TerminalNativeLaunchError::Cancelled
            ))
        ));
        assert!(
            std::fs::read_dir(fixture.path.join("artifacts"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn committed_start_runs_after_outer_receipt_drop_and_exposes_durable_facts() {
        let fixture = Fixture::new();
        let native_authority = fixture.native_authority();
        let receipt = futures_executor::block_on(fixture.starter.start(
            authority(),
            id(),
            fixture.request(Some("printf STAGED_OK; exit 0")),
            move || Ok(native_authority),
            Vec::new(),
            |_, _| Ok(()),
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(receipt.session.session_id, id());
        receipt.session.validate().unwrap();
        let residency = receipt.residency.clone();
        drop(receipt);
        // A short command can exit before the next ordinary read. Its final
        // drain is committed to history but is not replayed in step.output;
        // observe that authoritative history after dropping the outer receipt.
        until(|| {
            futures_executor::block_on(fixture.runtime().request_with_context(
                CancellationToken::new(),
                |context| {
                    context.registry.read(
                        &authority().owner,
                        &id(),
                        &machine_god_core::TerminalCursor::new(1, 0).unwrap(),
                        16384,
                    )
                },
            ))
            .unwrap()
            .unwrap()
            .bytes
            .windows(b"STAGED_OK".len())
            .any(|bytes| bytes == b"STAGED_OK")
        });
        let facts = futures_executor::block_on(
            fixture
                .runtime()
                .request_with_context(CancellationToken::new(), |context| {
                    context.registry.inspect(&authority().owner, &id())
                }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            facts.metadata.unwrap().command.as_deref(),
            Some("printf STAGED_OK; exit 0")
        );
        until(|| {
            futures_executor::block_on(fixture.runtime().request_with_context(
                CancellationToken::new(),
                |context| {
                    !context
                        .registry
                        .live_mut(&authority().owner, &id())
                        .unwrap()
                        .owns_backend()
                },
            ))
            .unwrap()
        });
        futures_executor::block_on(fixture.runtime().request_with_context(
            CancellationToken::new(),
            |context| {
                for number in 0..15 {
                    context
                        .registry
                        .reserve_start(
                            authority().owner,
                            TerminalSessionId::new(format!("pending-{number}")).unwrap(),
                        )
                        .unwrap();
                }
                assert!(matches!(
                    context.registry.reserve_start(
                        authority().owner,
                        TerminalSessionId::new("pressure").unwrap()
                    ),
                    Err(TerminalRegistryError::Capacity)
                ));
            },
        ))
        .unwrap();
        drop(residency);
        futures_executor::block_on(fixture.runtime().request_with_context(
            CancellationToken::new(),
            |context| {
                context
                    .registry
                    .reserve_start(
                        authority().owner,
                        TerminalSessionId::new("pressure").unwrap(),
                    )
                    .unwrap();
            },
        ))
        .unwrap();
    }

    #[test]
    fn initial_monitor_installation_precedes_effects_and_failure_retains_quiescent_session() {
        for reject in [false, true] {
            let fixture = Fixture::new();
            let native_authority = fixture.native_authority();
            let mut request =
                fixture.request(Some("printf ran > callback-command; sleep 0.1; exit 0"));
            request.initial_monitors.push(TerminalMonitorDefinition {
                condition: TerminalMonitorCondition::PathExists {
                    path: fixture.path.to_str().unwrap().into(),
                },
                check_schedule: Some(TerminalSchedule { interval_ms: 10 }),
                notify: TerminalNotifySchedule::OnMatch,
                lifetime: TerminalMonitorLifetime::UntilMatch,
            });
            let path = fixture.path.clone();
            let installed = Arc::clone(&fixture.monitor_installations);
            let poll_thread = std::thread::current().id();
            let result = futures_executor::block_on(fixture.starter.start_until(
                authority(),
                id(),
                request,
                move || Ok(native_authority),
                vec![TerminalMonitorActivation::default()],
                |state, registry| {
                    assert!(!state.reconciled);
                    assert!(registry.inspect(&authority().owner, &id()).is_err());
                    state.reconciled = true;
                },
                move |state, mutations| {
                    assert!(state.reconciled, "reconcile before monitor installation");
                    assert_ne!(std::thread::current().id(), poll_thread);
                    assert!(!path.join("callback-command").exists());
                    assert_eq!(mutations.len(), 1);
                    assert!(!mutations[0].removed);
                    assert_eq!(installed.fetch_add(1, Ordering::AcqRel), 0);
                    if reject {
                        Err(TerminalSessionError::InvalidState)
                    } else {
                        Ok(())
                    }
                },
                Instant::now() + Duration::from_secs(5),
                CancellationToken::new(),
            ));
            assert_eq!(fixture.monitor_installations.load(Ordering::Acquire), 1);
            if reject {
                assert!(matches!(
                    result,
                    Err(TerminalStagedStartError::Retained { .. })
                ));
                assert!(!fixture.path.join("callback-command").exists());
                let facts = futures_executor::block_on(
                    fixture
                        .runtime()
                        .request_with_context(CancellationToken::new(), |context| {
                            context.registry.inspect(&authority().owner, &id())
                        }),
                )
                .unwrap()
                .unwrap();
                assert_eq!(
                    facts.context.lifecycle,
                    machine_god_core::TerminalLifecycle::Lost
                );
            } else {
                result.unwrap();
                until(|| fixture.path.join("callback-command").exists());
            }
        }
    }

    #[test]
    fn failed_initial_monitor_admission_never_releases_command_and_retains_session() {
        let fixture = Fixture::new();
        let native_authority = fixture.native_authority();
        let mut request = fixture.request(Some("printf ran > should-not-run; exit 0"));
        request.initial_monitors.push(TerminalMonitorDefinition {
            condition: TerminalMonitorCondition::PathChanged {
                path: fixture.path.to_str().unwrap().into(),
            },
            check_schedule: Some(TerminalSchedule { interval_ms: 10 }),
            notify: TerminalNotifySchedule::OnMatch,
            lifetime: TerminalMonitorLifetime::UntilMatch,
        });
        let result = futures_executor::block_on(fixture.starter.start(
            authority(),
            id(),
            request,
            move || Ok(native_authority),
            vec![TerminalMonitorActivation::default()],
            |_, _| panic!("failed monitor admission must not install grants"),
            Duration::from_secs(5),
            CancellationToken::new(),
        ));
        assert!(matches!(
            result,
            Err(TerminalStagedStartError::Retained { .. })
        ));
        assert!(!fixture.path.join("should-not-run").exists());
        let facts = futures_executor::block_on(
            fixture
                .runtime()
                .request_with_context(CancellationToken::new(), |context| {
                    context.registry.inspect(&authority().owner, &id())
                }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            facts.context.lifecycle,
            machine_god_core::TerminalLifecycle::Lost
        );
    }
}
