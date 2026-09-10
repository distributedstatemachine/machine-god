use super::super::{HostState, lifecycle};
use super::*;
use crate::background_input::BackgroundInputReceipt;
use crate::terminal_captured_exec::TerminalCapturedExec;
use crate::terminal_history::TerminalHistory;
use crate::terminal_host_catalog::TerminalHostCatalogs;
use crate::terminal_host_probes::TerminalHostProbes;
use crate::terminal_journal::TerminalJournalLimits;
use crate::terminal_probe_effects::NativeTerminalProbeExecutor;
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
use futures_executor::block_on;
use machine_god_core::{
    SessionId, SessionIncarnationId, TerminalDimensions, TerminalLifecycle, TerminalSignal,
};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

#[derive(Default)]
struct Spawner(Mutex<Vec<std::thread::JoinHandle<()>>>);
impl TerminalRuntimeSpawner for Spawner {
    fn spawn(&self, job: TerminalRuntimeJob) -> std::result::Result<(), ()> {
        self.0.lock().unwrap().push(std::thread::spawn(job));
        Ok(())
    }
}
impl Spawner {
    fn collect(&self) {
        for thread in std::mem::take(&mut *self.0.lock().unwrap()) {
            thread.join().unwrap();
        }
    }
}
#[derive(Default)]
struct BackendState {
    closed: bool,
    closes: usize,
    forced: bool,
    cancel_on_close: Option<CancellationToken>,
    fail_once: bool,
    gate: Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>,
}
struct Backend(Arc<Mutex<BackendState>>);
impl TerminalSessionBackend for Backend {
    fn read(&mut self, _: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
        Ok(TerminalPtyRead {
            bytes_read: 0,
            closed: false,
        })
    }
    fn write(&mut self, _: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
        Err(())
    }
    fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
        Ok(if self.0.lock().unwrap().closed {
            TerminalPtyStatus::Exited(0)
        } else {
            TerminalPtyStatus::Running
        })
    }
    fn resize(&mut self, _: &TerminalDimensions) -> std::result::Result<(), ()> {
        Err(())
    }
    fn signal(&mut self, _: TerminalSignal) -> std::result::Result<(), ()> {
        panic!("background stop must use close, not numeric signal")
    }
    fn signal_may_discard_output(&self) -> bool {
        false
    }
    fn close(
        &mut self,
        force: bool,
        _: &mut dyn FnMut(&[u8]),
    ) -> std::result::Result<TerminalPtyClose, ()> {
        let gate = {
            let mut state = self.0.lock().unwrap();
            state.closes += 1;
            state.forced = force;
            if let Some(cancel) = state.cancel_on_close.take() {
                cancel.cancel();
            }
            if std::mem::take(&mut state.fail_once) {
                return Err(());
            }
            state.gate.take()
        };
        if let Some((entered, release)) = gate {
            entered.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        self.0.lock().unwrap().closed = true;
        Ok(TerminalPtyClose {
            status: TerminalPtyStatus::Exited(0),
            output_incomplete: false,
        })
    }
}
struct Row {
    number: u32,
    created: i64,
    cold: bool,
    bytes: Vec<u8>,
    command: String,
}
impl Row {
    fn new(number: u32, created: i64, cold: bool) -> Self {
        Self {
            number,
            created,
            cold,
            bytes: b"hello\0\xff\x1b[31m".to_vec(),
            command: "SECRET command".into(),
        }
    }
}
struct Fixture {
    runtime: Option<TerminalRuntime<Backend, HostState>>,
    principals: TerminalAccessPrincipals,
    spawner: Arc<Spawner>,
    initialized: Arc<AtomicUsize>,
    backend: Arc<Mutex<BackendState>>,
    path: PathBuf,
}
fn owner(name: &str) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new(name).unwrap(),
        SessionIncarnationId::new(name).unwrap(),
    )
}
fn id(number: u32) -> TerminalSessionId {
    TerminalSessionId::new(format!("terminal-{number:032x}")).unwrap()
}
impl Fixture {
    fn new(rows: Vec<Row>) -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "mg-background-native-{:032x}",
            u128::from_le_bytes(random)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let worker_path = path.clone();
        let principals = TerminalAccessPrincipals::default();
        principals.acquire(owner("a")).unwrap();
        let worker_principals = principals.clone();
        let initialized = Arc::new(AtomicUsize::new(0));
        let worker_initialized = initialized.clone();
        let backend = Arc::new(Mutex::new(BackendState::default()));
        let worker_backend = backend.clone();
        let spawner = Arc::new(Spawner::default());
        let runtime = TerminalRuntime::new(
            move || {
                worker_initialized.fetch_add(1, Ordering::SeqCst);
                let root = rustix::fs::open(
                    &worker_path,
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::DIRECTORY
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .unwrap();
                let store = TerminalProfileStore::prepare(root).unwrap();
                let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
                let mut registry = TerminalRegistry::new("/workspace".into()).unwrap();
                let mut catalogs = TerminalHostCatalogs::new("/workspace".into()).unwrap();
                populate(
                    rows,
                    &mut registry,
                    &store,
                    &mut catalogs,
                    budget,
                    &worker_backend,
                );
                let captured = Arc::new(
                    TerminalCapturedExec::new(
                        PathBuf::from("/bin/true"),
                        vec![],
                        Duration::from_secs(1),
                        1,
                    )
                    .unwrap(),
                );
                let probes = TerminalHostProbes::new(
                    Arc::new(NativeTerminalProbeExecutor::new(captured, 1).unwrap()),
                    CancellationToken::new(),
                );
                Ok(TerminalRuntimeWorker::new_with_state(
                    registry,
                    store,
                    budget,
                    HostState {
                        catalogs,
                        probes,
                        access: lifecycle::TerminalAccessRoutes::new(worker_principals),
                    },
                    || 1000,
                    |_, _| {},
                ))
            },
            spawner.clone(),
        );
        Self {
            runtime: Some(runtime),
            principals,
            spawner,
            initialized,
            backend,
            path,
        }
    }
    fn requester(&self) -> TerminalRuntimeRequester<Backend, HostState> {
        self.runtime.as_ref().unwrap().requester()
    }
    fn snapshot(&self) -> Result<NativeTerminalBackgroundSnapshot> {
        block_on(snapshot_request(
            self.requester(),
            self.principals.clone(),
            owner("a"),
            CancellationToken::new(),
        ))
    }
    fn select(&self, who: &str, number: Option<u32>) -> Result<NativeTerminalBackgroundTarget> {
        block_on(select_request(
            self.requester(),
            self.principals.clone(),
            owner(who),
            number.map(id),
            CancellationToken::new(),
        ))
    }
    fn close(
        &self,
        target: NativeTerminalBackgroundTarget,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<crate::terminal_host_dispatch::TerminalHostReply>> {
        let request = TerminalActionRequest::Close {
            session_id: target.id.clone(),
            policy: TerminalClosePolicy::Graceful,
        };
        action_request(
            self.requester(),
            self.principals.clone(),
            target,
            request,
            cancel,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.runtime.take());
        self.spawner.collect();
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

fn populate(
    rows: Vec<Row>,
    registry: &mut TerminalRegistry<Backend>,
    store: &TerminalProfileStore,
    catalogs: &mut TerminalHostCatalogs,
    budget: TerminalProfileBudget,
    backend: &Arc<Mutex<BackendState>>,
) {
    for row in rows {
        let catalog = catalogs
            .catalog(store, &owner("a"), &CancellationToken::new())
            .unwrap();
        let mut transaction = store.transaction().unwrap();
        let session_id = id(row.number);
        drop(transaction.create_session(catalog, &session_id).unwrap());
        let created = budget
            .create_journal(
                &mut transaction,
                catalog.namespace_key(),
                &session_id,
                TerminalJournalLimits {
                    segment_bytes: 128 * 1024,
                    session_bytes: 16 * 1024 * 1024,
                },
            )
            .unwrap();
        created.accounting.unwrap();
        let mut persistence =
            TerminalProfileMutationContext::new(&mut transaction, budget, catalog.namespace_key());
        let mut history = TerminalHistory::create_with(
            &mut persistence,
            created.operation.unwrap(),
            &TerminalDimensions::new(3, 20).unwrap(),
        )
        .unwrap();
        for chunk in row.bytes.chunks(65536) {
            history.append_with(&mut persistence, chunk).unwrap();
        }
        let mut metadata = crate::terminal_session_record::test_metadata();
        metadata.command = Some(row.command);
        let native = if row.cold {
            Arc::new(Mutex::new(BackendState::default()))
        } else {
            backend.clone()
        };
        let mut session = TerminalSession::new_with(
            &mut persistence,
            Backend(native),
            history,
            owner("a"),
            session_id.clone(),
            metadata,
            row.created,
        )
        .unwrap();
        session
            .shell_ready_with(&mut persistence, row.created)
            .unwrap();
        session
            .command_started_with(&mut persistence, row.created)
            .unwrap();
        if row.cold {
            session
                .close_with(
                    &mut persistence,
                    &owner("a"),
                    TerminalClosePolicy::Force,
                    row.created,
                )
                .unwrap();
        } else {
            registry
                .start(owner("a"), session_id, || Ok(session))
                .unwrap();
        }
    }
}

#[test]
fn inert_unknown_principal_cancelled_and_closed_requesters_never_initialize() {
    let mut fixture = Fixture::new(vec![]);
    drop(snapshot_request(
        fixture.requester(),
        fixture.principals.clone(),
        owner("a"),
        CancellationToken::new(),
    ));
    assert_eq!(
        fixture.select("unknown", None).unwrap_err(),
        NativeTerminalBackgroundError::NotFound
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        block_on(snapshot_request(
            fixture.requester(),
            fixture.principals.clone(),
            owner("a"),
            cancel
        ))
        .unwrap_err(),
        NativeTerminalBackgroundError::Cancelled
    );
    let requester = fixture.requester();
    drop(fixture.runtime.take());
    assert_eq!(
        block_on(snapshot_request(
            requester,
            fixture.principals.clone(),
            owner("a"),
            CancellationToken::new()
        ))
        .unwrap_err(),
        NativeTerminalBackgroundError::Closed
    );
    assert_eq!(fixture.initialized.load(Ordering::SeqCst), 0);
}

#[test]
fn host_shutdown_revokes_selected_observation_without_retaining_backend_authority() {
    let fixture = Fixture::new(vec![Row::new(1, 1, false)]);
    let target = fixture.select("a", None).unwrap();
    let revocation = target.revocation_token();
    fixture.principals.retire_all();
    assert!(revocation.is_cancelled());
    assert_eq!(
        block_on(fixture.close(target, CancellationToken::new())).unwrap_err(),
        NativeTerminalBackgroundError::Revoked
    );
    assert_eq!(fixture.backend.lock().unwrap().closes, 0);
}

#[test]
fn complete_snapshot_last_ties_metadata_history_and_redaction() {
    let fixture = Fixture::new(vec![
        Row::new(1, 10, false),
        Row::new(2, 20, true),
        Row::new(3, 20, true),
    ]);
    let snapshot = fixture.snapshot().unwrap();
    assert_eq!(
        snapshot
            .entries()
            .iter()
            .map(|row| row.id().clone())
            .collect::<Vec<_>>(),
        vec![id(3), id(2), id(1)]
    );
    assert!(snapshot.entries()[0].recovered());
    assert!(!snapshot.entries()[0].owns_backend());
    assert!(snapshot.entries()[2].owns_backend());
    assert_eq!(
        snapshot.entries()[2].facts().lifecycle,
        TerminalLifecycle::Running
    );
    assert_eq!(snapshot.entries()[0].command(), Some("SECRET command"));
    assert!(!format!("{snapshot:?} {:?}", snapshot.entries()[0]).contains("SECRET"));
    assert_eq!(fixture.select("a", None).unwrap().id(), &id(3));
    assert_eq!(fixture.select("a", Some(1)).unwrap().id(), &id(1));
    for number in [1, 3] {
        let target = fixture.select("a", Some(number)).unwrap();
        let reply = block_on(action_request(
            fixture.requester(),
            fixture.principals.clone(),
            target,
            TerminalActionRequest::Inspect {
                session_id: id(number),
                events: TerminalEventQuery {
                    after_event_id: 0,
                    acknowledge_event_id: None,
                    max_events: 1,
                },
            },
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(reply.owned_backend_at_admission, number == 1);
        assert!(matches!(reply.result, TerminalActionResult::Inspect { .. }));
    }
    assert_eq!(
        fixture.select("a", Some(99)).unwrap_err(),
        NativeTerminalBackgroundError::NotFound
    );
}

#[test]
fn complete_empty_and_aggregate_text_overflow_are_not_false_latest() {
    let empty = Fixture::new(vec![]);
    assert!(empty.snapshot().unwrap().entries().is_empty());
    assert_eq!(
        empty.select("a", None).unwrap_err(),
        NativeTerminalBackgroundError::NotFound
    );
    let rows = (0..17)
        .map(|number| {
            let mut row = Row::new(number, 10, true);
            row.command = "x".repeat(64 * 1024);
            row
        })
        .collect();
    let large = Fixture::new(rows);
    assert_eq!(
        large.select("a", None).unwrap_err(),
        NativeTerminalBackgroundError::ResourceLimit
    );
}

#[test]
fn incomplete_catalog_and_row_overflow_fail_the_entire_selection() {
    for count in [1, 129] {
        let fixture = Fixture::new(vec![]);
        block_on(fixture.requester().request_with_context(
            CancellationToken::new(),
            move |context| {
                let catalog = context
                    .state
                    .catalogs
                    .catalog(context.store, &owner("a"), context.cancellation)
                    .unwrap();
                let mut transaction = context.store.transaction().unwrap();
                for number in 0..count {
                    drop(transaction.create_session(catalog, &id(number)).unwrap());
                }
            },
        ))
        .unwrap();
        assert_eq!(
            fixture.select("a", None).unwrap_err(),
            if count == 1 {
                NativeTerminalBackgroundError::Unavailable
            } else {
                NativeTerminalBackgroundError::ResourceLimit
            }
        );
    }
}

#[test]
fn binary_pages_and_tail_cross_segments_without_unbounded_collection() {
    let bytes: Vec<u8> = (0..150_000)
        .map(|n| u8::try_from(n % 256).unwrap())
        .collect();
    let mut row = Row::new(1, 1, true);
    row.bytes = bytes.clone();
    let fixture = Fixture::new(vec![row]);
    let target = fixture.select("a", Some(1)).unwrap();
    let mut cursor = block_on(tail_request(
        fixture.requester(),
        fixture.principals.clone(),
        target.clone(),
        30000,
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(cursor, TerminalCursor::new(1, 120_000).unwrap());
    let mut collected = Vec::new();
    for _ in 0..4 {
        let page = block_on(read_request(
            fixture.requester(),
            fixture.principals.clone(),
            target.clone(),
            cursor,
            30000 - collected.len(),
            CancellationToken::new(),
        ))
        .unwrap();
        assert!(page.gap().is_none());
        collected.extend_from_slice(page.bytes());
        cursor = page.next().clone();
        if page.next() == page.latest() {
            break;
        }
    }
    assert_eq!(collected, bytes[120_000..]);
    assert!(!format!("{target:?}").contains(target.id().as_str()));
    assert_eq!(
        block_on(read_request(
            fixture.requester(),
            fixture.principals.clone(),
            target,
            cursor,
            65537,
            CancellationToken::new()
        ))
        .unwrap_err(),
        NativeTerminalBackgroundError::Invalid
    );
}

#[test]
fn selection_is_sealed_to_host_and_revoked_by_real_handoff_and_reactivation() {
    let fixture = Fixture::new(vec![Row::new(1, 1, false)]);
    let target = fixture.select("a", None).unwrap();
    let other = Fixture::new(vec![]);
    assert_eq!(
        block_on(read_request(
            other.requester(),
            other.principals.clone(),
            target.clone(),
            TerminalCursor::new(1, 0).unwrap(),
            10,
            CancellationToken::new()
        ))
        .unwrap_err(),
        NativeTerminalBackgroundError::NotFound
    );
    block_on(
        fixture
            .requester()
            .request_with_context(CancellationToken::new(), |context| {
                lifecycle::handoff(context, &owner("a"), &owner("b"))
            }),
    )
    .unwrap()
    .unwrap();
    assert!(target.revocation_token().is_cancelled());
    assert_eq!(
        block_on(fixture.close(target.clone(), CancellationToken::new())).unwrap_err(),
        NativeTerminalBackgroundError::Revoked
    );
    assert_eq!(fixture.select("b", None).unwrap().id(), target.id());
    block_on(
        fixture
            .requester()
            .request_with_context(CancellationToken::new(), |context| {
                context.state.access.activate(owner("a"))
            }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        block_on(fixture.close(target, CancellationToken::new())).unwrap_err(),
        NativeTerminalBackgroundError::Revoked
    );
    assert!(fixture.snapshot().unwrap().entries().is_empty());
    assert_eq!(fixture.backend.lock().unwrap().closes, 0);
}

#[test]
fn graceful_stop_keeps_committed_receipt_after_cancellation_and_marks_history_only() {
    let fixture = Fixture::new(vec![Row::new(1, 1, false), Row::new(2, 2, true)]);
    let target = fixture.select("a", Some(1)).unwrap();
    let cancel = CancellationToken::new();
    fixture.backend.lock().unwrap().cancel_on_close = Some(cancel.clone());
    let reply = block_on(fixture.close(target, cancel.clone())).unwrap();
    assert!(cancel.is_cancelled());
    assert!(reply.owned_backend_at_admission);
    assert!(matches!(
        reply.result,
        TerminalActionResult::Close {
            policy: TerminalClosePolicy::Graceful,
            ..
        }
    ));
    assert_eq!(fixture.backend.lock().unwrap().closes, 1);
    assert!(!fixture.backend.lock().unwrap().forced);
    let history = block_on(fixture.close(
        fixture.select("a", Some(2)).unwrap(),
        CancellationToken::new(),
    ))
    .unwrap();
    assert!(!history.owned_backend_at_admission);
}

#[test]
fn dropped_stop_future_does_not_abandon_started_close_and_failure_is_uncertain() {
    let fixture = Fixture::new(vec![Row::new(1, 1, false)]);
    let target = fixture.select("a", None).unwrap();
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    fixture.backend.lock().unwrap().gate = Some((entered, wait));
    let mut operation = fixture.close(target, CancellationToken::new());
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(
        operation.as_mut().poll(&mut context),
        Poll::Pending
    ));
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(operation);
    release.send(()).unwrap();
    fixture.snapshot().unwrap();
    assert_eq!(fixture.backend.lock().unwrap().closes, 1);
    let failing = Fixture::new(vec![Row::new(3, 1, false)]);
    let target = failing.select("a", None).unwrap();
    failing.backend.lock().unwrap().fail_once = true;
    assert_eq!(
        block_on(failing.close(target, CancellationToken::new())).unwrap_err(),
        NativeTerminalBackgroundError::Uncertain
    );
}
