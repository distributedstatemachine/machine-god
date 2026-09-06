//! Runtime-facing resident and historical dispatch. Only data and receipt
//! futures leave the owner; profile/catalog/backend authority stays on it.

use machine_god_core::{
    BoxFuture, CancellationToken, TerminalActionRequest, TerminalActionResult, TerminalSessionFacts,
};

use crate::terminal_catalog_view::{self, TerminalCatalogViewError};
use crate::terminal_host_catalog::TerminalHostCatalogs;
use crate::terminal_monitor::TerminalMonitorActivation;
use crate::terminal_owner::TerminalOwnerContext;
use crate::terminal_registry::TerminalRegistryError;
use crate::terminal_resident_dispatch::{
    TerminalResidentAuthority, TerminalResidentDispatch, TerminalResidentError, dispatch_resident,
    resident_facts, resident_wait_result, resident_write_result,
};
use crate::terminal_runtime::{TerminalRuntimeError, TerminalRuntimeRequester};
use crate::terminal_session::TerminalSessionBackend;
use crate::terminal_wait::TerminalWaitReceipt;
use crate::terminal_write_completion::{TerminalWriteError, TerminalWriteReceipt};

/// The host may retain additional launch/probe state beside these catalog leases.
/// This trait does not require that worker-owned state be Send or Sync.
pub(crate) trait TerminalCatalogState {
    fn catalogs(&mut self) -> &mut TerminalHostCatalogs;
}
impl TerminalCatalogState for TerminalHostCatalogs {
    fn catalogs(&mut self) -> &mut TerminalHostCatalogs {
        self
    }
}

#[derive(Debug)]
pub(crate) enum TerminalHostDispatchError {
    Invalid,
    CommandAction,
    Runtime(TerminalRuntimeError),
    Resident(TerminalResidentError),
    Catalog(TerminalCatalogViewError),
    Write(TerminalWriteError),
    /// Native completion survives an unavailable public projection. The caller
    /// must not retry the operation or turn this into successful durability.
    WaitReceipt(TerminalWaitReceipt),
    WriteReceipt(TerminalWriteReceipt),
}
type Result<T> = std::result::Result<T, TerminalHostDispatchError>;

/// Whether session facts were refreshed after the asynchronous receipt. An
/// admission snapshot is explicit, never passed off as final session state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalHostFactsTiming {
    Current,
    Admission,
}
#[derive(Debug)]
pub(crate) struct TerminalHostReply {
    pub(crate) result: TerminalActionResult,
    pub(crate) facts_timing: TerminalHostFactsTiming,
}

/// Route one already authorized non-command action. Monitor activation comes
/// from separate authorized host work, never from stored probe descriptions.
/// No I/O, worker initialization or request submission occurs before polling.
/// A requester deliberately does not keep the host or its native sessions alive.
pub(crate) fn dispatch<B, S>(
    requester: TerminalRuntimeRequester<B, S>,
    authority: TerminalResidentAuthority,
    request: TerminalActionRequest,
    activation: TerminalMonitorActivation,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<TerminalHostReply>>
where
    B: TerminalSessionBackend + Send + 'static,
    S: TerminalCatalogState + 'static,
{
    Box::pin(async move {
        request
            .validate()
            .map_err(|_| TerminalHostDispatchError::Invalid)?;
        if matches!(
            request,
            TerminalActionRequest::Exec { .. } | TerminalActionRequest::Start { .. }
        ) {
            return Err(TerminalHostDispatchError::CommandAction);
        }
        let admitted_authority = authority.clone();
        let reply = requester
            .request_with_context(cancellation, move |context| {
                route(context, &admitted_authority, &request, activation)
            })
            .await
            .map_err(TerminalHostDispatchError::Runtime)??;
        match reply {
            TerminalResidentDispatch::Ready(result) => Ok(TerminalHostReply {
                result,
                facts_timing: TerminalHostFactsTiming::Current,
            }),
            TerminalResidentDispatch::Wait { admitted, future } => {
                let receipt = future.await;
                let (session, facts_timing) = refresh(&requester, authority, admitted).await;
                let result = resident_wait_result(session, receipt)
                    .map_err(TerminalHostDispatchError::WaitReceipt)?;
                Ok(TerminalHostReply {
                    result,
                    facts_timing,
                })
            }
            TerminalResidentDispatch::Write { admitted, future } => {
                let receipt = future.await.map_err(TerminalHostDispatchError::Write)?;
                let (session, facts_timing) = refresh(&requester, authority, admitted).await;
                let result = resident_write_result(session, receipt)
                    .map_err(TerminalHostDispatchError::WriteReceipt)?;
                Ok(TerminalHostReply {
                    result,
                    facts_timing,
                })
            }
        }
    })
}

async fn refresh<B, S>(
    requester: &TerminalRuntimeRequester<B, S>,
    authority: TerminalResidentAuthority,
    admitted: TerminalSessionFacts,
) -> (TerminalSessionFacts, TerminalHostFactsTiming)
where
    B: TerminalSessionBackend + Send + 'static,
    S: 'static,
{
    let id = admitted.session_id.clone();
    // User cancellation cannot erase an already committed receipt. This fresh
    // token does not bypass owner shutdown: the non-owning requester still closes.
    match requester
        .request_with_context(CancellationToken::new(), move |mut context| {
            resident_facts(&mut context, &authority, &id)
        })
        .await
    {
        Ok(Ok(facts)) => (facts, TerminalHostFactsTiming::Current),
        _ => (admitted, TerminalHostFactsTiming::Admission),
    }
}

fn route<B: TerminalSessionBackend, S: TerminalCatalogState>(
    context: TerminalOwnerContext<'_, B, S>,
    authority: &TerminalResidentAuthority,
    request: &TerminalActionRequest,
    activation: TerminalMonitorActivation,
) -> Result<TerminalResidentDispatch> {
    if context.cancellation.is_cancelled() {
        return Err(TerminalHostDispatchError::Resident(
            TerminalResidentError::Cancelled,
        ));
    }
    if let TerminalActionRequest::List { filters } = request {
        let catalog = context
            .state
            .catalogs()
            .catalog(context.store, &authority.owner, context.cancellation)
            .map_err(TerminalHostDispatchError::Catalog)?;
        let result = terminal_catalog_view::list_with(
            context.registry,
            context.store,
            catalog,
            *context.budget,
            &authority.owner,
            authority.actor,
            &authority.controls,
            filters,
            context.now_ms,
        )
        .map_err(TerminalHostDispatchError::Catalog)?;
        result
            .validate_for(request)
            .map_err(|_| TerminalHostDispatchError::Invalid)?;
        return Ok(TerminalResidentDispatch::Ready(result));
    }
    let id = request
        .session_id()
        .ok_or(TerminalHostDispatchError::CommandAction)?;
    match context.registry.authorize_resident(&authority.owner, id) {
        Ok(()) => dispatch_resident(context, authority, request, activation)
            .map_err(TerminalHostDispatchError::Resident),
        Err(TerminalRegistryError::NotFound)
            if matches!(
                request,
                TerminalActionRequest::Read { .. }
                    | TerminalActionRequest::Screen { .. }
                    | TerminalActionRequest::Inspect { .. }
            ) =>
        {
            context
                .state
                .catalogs()
                .dispatch_history(
                    context.store,
                    *context.budget,
                    authority,
                    request,
                    context.now_ms,
                    context.cancellation,
                )
                .map(TerminalResidentDispatch::Ready)
                .map_err(TerminalHostDispatchError::Catalog)
        }
        Err(error) => Err(TerminalHostDispatchError::Resident(error.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_input::{BackgroundInputReceipt, BackgroundInputStatus};
    use crate::terminal_history::TerminalHistory;
    use crate::terminal_input::TerminalWriterId;
    use crate::terminal_journal::TerminalJournalLimits;
    use crate::terminal_profile::{
        TerminalProfileBudget, TerminalProfileLimits, TerminalProfileMutationContext,
    };
    use crate::terminal_profile_store::TerminalProfileStore;
    use crate::terminal_pty::{TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
    use crate::terminal_registry::TerminalRegistry;
    use crate::terminal_runtime::{
        TerminalRuntime, TerminalRuntimeJob, TerminalRuntimeSpawner, TerminalRuntimeWorker,
    };
    use crate::terminal_session::TerminalSession;
    use crate::terminal_session_record::test_metadata;
    use futures_executor::block_on;
    use machine_god_core::{
        BackgroundOutputOwner, SessionId, SessionIncarnationId, TerminalActorRole,
        TerminalAllowedControls, TerminalClosePolicy, TerminalCursor, TerminalDimensions,
        TerminalEventQuery, TerminalLifecycle, TerminalListFilters, TerminalReturnCondition,
        TerminalReturnOutcome, TerminalSessionId, TerminalSignal, TerminalWaitRequest,
        TerminalWriteLeaseIntent, TerminalWritePayload, TerminalWriteRequest,
    };
    use std::num::NonZeroU64;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::task::{Context, Waker};
    use std::thread::JoinHandle;

    #[derive(Default)]
    struct Spawner(Mutex<Vec<JoinHandle<()>>>);
    impl TerminalRuntimeSpawner for Spawner {
        fn spawn(&self, job: TerminalRuntimeJob) -> std::result::Result<(), ()> {
            self.0.lock().unwrap().push(std::thread::spawn(job));
            Ok(())
        }
    }
    impl Spawner {
        fn collect(&self) {
            for handle in std::mem::take(&mut *self.0.lock().unwrap()) {
                handle.join().unwrap();
            }
        }
    }
    #[derive(Default)]
    struct BackendState {
        written: Vec<u8>,
        blocked: bool,
        closed: bool,
    }
    struct Backend(Arc<Mutex<BackendState>>);
    impl TerminalSessionBackend for Backend {
        fn read(&mut self, _: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            Ok(TerminalPtyRead {
                bytes_read: 0,
                closed: false,
            })
        }
        fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            let mut state = self.0.lock().unwrap();
            if state.blocked {
                return Ok(BackgroundInputReceipt::new(
                    0,
                    false,
                    BackgroundInputStatus::Backpressure,
                ));
            }
            state.written.extend_from_slice(bytes);
            Ok(BackgroundInputReceipt::new(
                bytes.len(),
                false,
                BackgroundInputStatus::Written,
            ))
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            Ok(if self.0.lock().unwrap().closed {
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
            self.0.lock().unwrap().closed = true;
            Ok(TerminalPtyClose {
                status: TerminalPtyStatus::Exited(0),
                output_incomplete: false,
            })
        }
    }
    // Deliberately !Send/!Sync: dispatch futures must never own host state.
    struct State {
        catalogs: TerminalHostCatalogs,
        _worker_only: Rc<()>,
    }
    impl TerminalCatalogState for State {
        fn catalogs(&mut self) -> &mut TerminalHostCatalogs {
            &mut self.catalogs
        }
    }
    struct Fixture {
        runtime: Option<TerminalRuntime<Backend, State>>,
        spawner: Arc<Spawner>,
        initialized: Arc<AtomicUsize>,
        backend: Arc<Mutex<BackendState>>,
        path: PathBuf,
    }
    fn private_test_directory() -> PathBuf {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "machine-god-host-dispatch-{:032x}",
            u128::from_le_bytes(random)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        path
    }
    fn prepare_profile(path: &std::path::Path) -> TerminalProfileStore {
        let root = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        TerminalProfileStore::prepare(root).unwrap()
    }
    impl Fixture {
        fn new() -> Self {
            let path = private_test_directory();
            let worker_path = path.clone();
            let initialized = Arc::new(AtomicUsize::new(0));
            let worker_initialized = Arc::clone(&initialized);
            let backend = Arc::new(Mutex::new(BackendState::default()));
            let worker_backend = Arc::clone(&backend);
            let spawner = Arc::new(Spawner::default());
            let runtime = TerminalRuntime::new(
                move || {
                    worker_initialized.fetch_add(1, Ordering::SeqCst);
                    let store = prepare_profile(&worker_path);
                    let budget =
                        TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
                    let mut registry = TerminalRegistry::new("/workspace".into()).unwrap();
                    let mut catalogs = TerminalHostCatalogs::new("/workspace".into()).unwrap();
                    for (name, cold) in [("live", false), ("saved", true)] {
                        let who = authority();
                        let catalog = catalogs
                            .catalog(&store, &who.owner, &CancellationToken::new())
                            .unwrap();
                        let mut transaction = store.transaction().unwrap();
                        let id = id(name);
                        drop(transaction.create_session(catalog, &id).unwrap());
                        let created = budget
                            .create_journal(
                                &mut transaction,
                                catalog.namespace_key(),
                                &id,
                                TerminalJournalLimits::default(),
                            )
                            .unwrap();
                        created.accounting.unwrap();
                        let mut persistence = TerminalProfileMutationContext::new(
                            &mut transaction,
                            budget,
                            catalog.namespace_key(),
                        );
                        let history = TerminalHistory::create_with(
                            &mut persistence,
                            created.operation.unwrap(),
                            &TerminalDimensions::new(3, 20).unwrap(),
                        )
                        .unwrap();
                        let native = if cold {
                            Arc::new(Mutex::new(BackendState::default()))
                        } else {
                            Arc::clone(&worker_backend)
                        };
                        let mut session = TerminalSession::new_with(
                            &mut persistence,
                            Backend(native),
                            history,
                            who.owner.clone(),
                            id.clone(),
                            test_metadata(),
                            0,
                        )
                        .unwrap();
                        session.shell_ready_with(&mut persistence, 0).unwrap();
                        if cold {
                            session
                                .close_with(
                                    &mut persistence,
                                    &who.owner,
                                    TerminalClosePolicy::Force,
                                    0,
                                )
                                .unwrap();
                        } else {
                            registry.start(who.owner, id, || Ok(session)).unwrap();
                        }
                    }
                    Ok(TerminalRuntimeWorker::new_with_state(
                        registry,
                        store,
                        budget,
                        State {
                            catalogs,
                            _worker_only: Rc::new(()),
                        },
                        || 0,
                        |_, _| {},
                    ))
                },
                spawner.clone(),
            );
            Self {
                runtime: Some(runtime),
                spawner,
                initialized,
                backend,
                path,
            }
        }
        fn requester(&self) -> TerminalRuntimeRequester<Backend, State> {
            self.runtime.as_ref().unwrap().requester()
        }
        fn action(&self, request: TerminalActionRequest) -> TerminalHostReply {
            let expected = request.clone();
            let reply = block_on(dispatch(
                self.requester(),
                authority(),
                request,
                TerminalMonitorActivation::default(),
                CancellationToken::new(),
            ))
            .unwrap();
            reply.result.validate_for(&expected).unwrap();
            reply
        }
        fn shutdown(&mut self) {
            drop(self.runtime.take());
            self.spawner.collect();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.shutdown();
            std::fs::remove_dir_all(&self.path).unwrap();
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
    fn id(name: &str) -> TerminalSessionId {
        TerminalSessionId::new(name).unwrap()
    }
    fn list() -> TerminalActionRequest {
        TerminalActionRequest::List {
            filters: TerminalListFilters::default(),
        }
    }
    fn write() -> TerminalActionRequest {
        TerminalActionRequest::Write {
            session_id: id("live"),
            request: TerminalWriteRequest {
                lease: TerminalWriteLeaseIntent::Use,
                payload: Some(TerminalWritePayload::Text { text: "abc".into() }),
            },
        }
    }
    fn acquire() -> TerminalActionRequest {
        TerminalActionRequest::Write {
            session_id: id("live"),
            request: TerminalWriteRequest {
                lease: TerminalWriteLeaseIntent::Acquire,
                payload: None,
            },
        }
    }
    #[test]
    fn unpolled_and_pre_cancelled_dispatch_never_initialize_host() {
        let fixture = Fixture::new();
        drop(dispatch(
            fixture.requester(),
            authority(),
            list(),
            TerminalMonitorActivation::default(),
            CancellationToken::new(),
        ));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(
            block_on(dispatch(
                fixture.requester(),
                authority(),
                list(),
                TerminalMonitorActivation::default(),
                cancel
            ))
            .is_err()
        );
        assert_eq!(fixture.initialized.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 0);
    }
    #[test]
    fn list_and_observations_route_live_and_cold_histories_on_one_worker() {
        let fixture = Fixture::new();
        let reply = fixture.action(list());
        assert_eq!(reply.facts_timing, TerminalHostFactsTiming::Current);
        let TerminalActionResult::List { sessions } = reply.result else {
            panic!("list")
        };
        assert_eq!(
            sessions
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            ["live", "saved"]
        );
        assert_eq!(sessions[0].lifecycle, TerminalLifecycle::Running);
        assert_eq!(sessions[1].lifecycle, TerminalLifecycle::Closed);
        for name in ["live", "saved"] {
            fixture.action(TerminalActionRequest::Read {
                session_id: id(name),
                cursor: TerminalCursor::new(1, 0).unwrap(),
            });
            fixture.action(TerminalActionRequest::Screen {
                session_id: id(name),
            });
            fixture.action(TerminalActionRequest::Inspect {
                session_id: id(name),
                events: TerminalEventQuery {
                    after_event_id: 0,
                    acknowledge_event_id: None,
                    max_events: 256,
                },
            });
        }
        let check = fixture
            .requester()
            .request_with_context(CancellationToken::new(), |context| {
                assert!(context.store.transaction().is_ok());
                context.registry.owner_ids(&authority().owner).len()
            });
        assert_eq!(block_on(check).unwrap(), 1);
        assert_eq!(fixture.initialized.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn foreign_owner_and_cold_mutation_never_gain_native_authority() {
        let fixture = Fixture::new();
        fixture.action(list());
        let mut who = authority();
        who.owner = BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new("foreign").unwrap(),
        );
        let foreign = block_on(dispatch(
            fixture.requester(),
            who,
            list(),
            TerminalMonitorActivation::default(),
            CancellationToken::new(),
        ))
        .unwrap();
        assert!(
            matches!(foreign.result, TerminalActionResult::List { sessions } if sessions.is_empty())
        );
        for (who, target) in [
            (authority(), "saved"),
            (
                {
                    let mut who = authority();
                    who.owner = BackgroundOutputOwner::new(
                        SessionId::new("owner").unwrap(),
                        SessionIncarnationId::new("foreign").unwrap(),
                    );
                    who
                },
                "live",
            ),
        ] {
            let mut request = write();
            if let TerminalActionRequest::Write { session_id, .. } = &mut request {
                *session_id = id(target);
            }
            assert!(matches!(
                block_on(dispatch(
                    fixture.requester(),
                    who,
                    request,
                    TerminalMonitorActivation::default(),
                    CancellationToken::new()
                )),
                Err(TerminalHostDispatchError::Resident(
                    TerminalResidentError::Registry(TerminalRegistryError::NotFound)
                ))
            ));
        }
        assert!(fixture.backend.lock().unwrap().written.is_empty());
    }
    #[test]
    fn wait_and_write_finish_outside_owner_and_refresh_committed_facts() {
        let fixture = Fixture::new();
        fixture.action(acquire());
        let reply = fixture.action(write());
        assert_eq!(reply.facts_timing, TerminalHostFactsTiming::Current);
        assert!(matches!(
            reply.result,
            TerminalActionResult::Write {
                accepted_bytes: 3,
                ..
            }
        ));
        assert_eq!(fixture.backend.lock().unwrap().written, b"abc");
        let reply = fixture.action(TerminalActionRequest::Wait {
            session_id: id("live"),
            request: TerminalWaitRequest {
                condition: TerminalReturnCondition::Started {},
                safety_ceiling_ms: 1000,
            },
        });
        assert_eq!(reply.facts_timing, TerminalHostFactsTiming::Current);
        assert!(matches!(
            reply.result,
            TerminalActionResult::Wait {
                outcome: TerminalReturnOutcome::Started {},
                ..
            }
        ));
    }
    #[test]
    fn committed_write_survives_shutdown_without_extending_host_lifetime() {
        let mut fixture = Fixture::new();
        fixture.action(list());
        fixture.action(acquire());
        fixture.backend.lock().unwrap().blocked = true;
        let requester = fixture.requester();
        let mut pending = dispatch(
            requester.clone(),
            authority(),
            write(),
            TerminalMonitorActivation::default(),
            CancellationToken::new(),
        );
        let mut cx = Context::from_waker(Waker::noop());
        assert!(pending.as_mut().poll(&mut cx).is_pending());
        // FIFO barrier proves write admission ran before shutdown. The future
        // itself need not be polled again for the owner to complete its receipt.
        block_on(requester.request_with_context(CancellationToken::new(), |_| ())).unwrap();
        fixture.shutdown();
        let reply = block_on(pending).unwrap();
        assert_eq!(reply.facts_timing, TerminalHostFactsTiming::Admission);
        assert!(matches!(
            reply.result,
            TerminalActionResult::Write {
                accepted_bytes: 0,
                ..
            }
        ));
        assert!(fixture.backend.lock().unwrap().closed);
        assert!(fixture.backend.lock().unwrap().written.is_empty());
    }

    #[test]
    fn cancelled_wait_does_not_block_other_actions_or_erase_its_receipt() {
        let fixture = Fixture::new();
        fixture.action(list());
        let cancellation = CancellationToken::new();
        let requester = fixture.requester();
        let mut pending = dispatch(
            requester.clone(),
            authority(),
            TerminalActionRequest::Wait {
                session_id: id("live"),
                request: TerminalWaitRequest {
                    condition: TerminalReturnCondition::Exit {},
                    safety_ceiling_ms: 1000,
                },
            },
            TerminalMonitorActivation::default(),
            cancellation.clone(),
        );
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        block_on(requester.request_with_context(CancellationToken::new(), |_| ())).unwrap();
        // This request must finish while the wait remains unsatisfied.
        fixture.action(list());
        cancellation.cancel();
        let reply = block_on(pending).unwrap();
        assert_eq!(reply.facts_timing, TerminalHostFactsTiming::Current);
        assert!(matches!(
            reply.result,
            TerminalActionResult::Wait {
                outcome: TerminalReturnOutcome::Cancelled {},
                ..
            }
        ));
        assert!(!fixture.backend.lock().unwrap().closed);
    }
}
