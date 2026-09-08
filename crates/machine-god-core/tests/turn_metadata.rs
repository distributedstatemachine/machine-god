use std::collections::{BTreeMap, VecDeque};
use std::future::poll_fn;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use futures_core::Stream;
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::*;
use serde_json::{Value, json};

#[derive(Clone, Copy, Default)]
enum Save {
    #[default]
    Commit,
    Wait,
    CommitThenWait,
    CommitThenError,
    Fail,
}

#[derive(Default)]
struct Store {
    record: Mutex<Option<SessionRecord>>,
    saves: Mutex<VecDeque<Save>>,
    loads: Mutex<VecDeque<Option<SessionRecord>>>,
    calls: AtomicUsize,
    load_calls: AtomicUsize,
    drops: AtomicUsize,
    release: AtomicBool,
    conflicts: AtomicUsize,
    cancel_after_commit: Mutex<Option<TurnHandle>>,
}

struct Dropped<'a>(&'a AtomicUsize);
impl Drop for Dropped<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn store_error(kind: SessionStoreErrorKind) -> SessionStoreError {
    SessionStoreError::new(kind, "secret-code", "secret-message", true)
}

impl Store {
    fn behavior(&self, value: Save) {
        self.saves.lock().unwrap().push_back(value);
    }
    fn record(&self) -> SessionRecord {
        self.record.lock().unwrap().clone().unwrap()
    }
    async fn wait(&self) {
        poll_fn(|_| {
            if self.release.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    }
}

impl SessionStore for Store {
    fn load(
        &self,
        _: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(self
                .loads
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| self.record.lock().unwrap().clone()))
        })
    }
    fn save(
        &self,
        mut record: SessionRecord,
        expected: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let _dropped = Dropped(&self.drops);
            let behavior = self.saves.lock().unwrap().pop_front().unwrap_or_default();
            if self
                .conflicts
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(store_error(SessionStoreErrorKind::Conflict));
            }
            if matches!(behavior, Save::Fail) {
                return Err(store_error(SessionStoreErrorKind::Unavailable));
            }
            if matches!(behavior, Save::Wait) {
                self.wait().await;
            }
            let revision = {
                let mut current = self.record.lock().unwrap();
                if current.as_ref().map(|value| value.revision) != expected {
                    return Err(store_error(SessionStoreErrorKind::Conflict));
                }
                record.revision = SessionRevision(record.revision.0 + 1);
                let revision = record.revision;
                *current = Some(record);
                revision
            };
            if matches!(behavior, Save::CommitThenWait) {
                self.wait().await;
            }
            if matches!(behavior, Save::CommitThenError) {
                return Err(store_error(SessionStoreErrorKind::Unavailable));
            }
            if let Some(handle) = self.cancel_after_commit.lock().unwrap().take() {
                let _ = handle.cancel();
            }
            Ok(revision)
        })
    }
}

#[derive(Default)]
struct Provider {
    release: AtomicBool,
    calls: AtomicUsize,
    tool: bool,
}
impl ModelProvider for Provider {
    fn name(&self) -> &'static str {
        "metadata-test"
    }
    fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            poll_fn(|_| {
                if self.release.load(Ordering::Acquire) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await;
            let events = if self.tool && call == 0 {
                vec![
                    ModelEvent::TextDelta {
                        text: "before call".into(),
                    },
                    ModelEvent::ToolCall {
                        call: ToolCall {
                            id: ToolCallId::new("call").unwrap(),
                            name: ToolName::new("echo").unwrap(),
                            arguments: json!({}),
                        },
                    },
                    ModelEvent::Stop {
                        reason: StopReason::ToolCalls,
                    },
                ]
            } else {
                vec![
                    ModelEvent::TextDelta {
                        text: "answer".into(),
                    },
                    ModelEvent::Stop {
                        reason: StopReason::Completed,
                    },
                ]
            };
            Ok(
                Box::pin(futures_util::stream::iter(events.into_iter().map(Ok)))
                    as ModelEventStream,
            )
        })
    }
}

#[derive(Default)]
struct Policy {
    editor: Mutex<Option<TurnMetadataEditor>>,
    calls: AtomicUsize,
}
impl PermissionHandler for Policy {
    fn authorize(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let editor = self.editor.lock().unwrap().clone();
            if let Some(editor) = editor {
                let before = editor.read_entry().await.unwrap();
                editor
                    .compare_exchange(before, Some(json!({"allow": true})))
                    .await
                    .unwrap();
            }
            Ok(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        })
    }
}

struct Echo;
impl Tool for Echo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("echo").unwrap(),
            description: "test".into(),
            input_schema: json!({}),
        }
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        Ok(PreparedToolCall::new(
            Capability::Tool {
                name: call.name,
                call_id: call.id,
                arguments: call.arguments.clone(),
            },
            call.arguments,
        ))
    }
    fn execute(
        &self,
        _: ToolContext,
        _: Value,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { Ok(ToolOutput::success(json!({"executed": true}))) })
    }
}

struct Fixture {
    engine: Engine,
    session: Session,
    store: Arc<Store>,
    provider: Arc<Provider>,
    policy: Arc<Policy>,
}
impl Fixture {
    fn new(tool: bool, limits: EngineLimits) -> Self {
        let store = Arc::new(Store::default());
        let provider = Arc::new(Provider {
            tool,
            ..Provider::default()
        });
        let policy = Arc::new(Policy::default());
        let engine = Engine::builder()
            .host_resource(())
            .shared_session_store(store.clone())
            .shared_provider(provider.clone())
            .shared_permission_handler(policy.clone())
            .tool(Echo)
            .limits(limits)
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("session").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        Self {
            engine,
            session,
            store,
            provider,
            policy,
        }
    }
    fn turn(&self) -> Turn {
        block_on(self.session.prompt("prompt")).unwrap()
    }
}
fn fixture() -> Fixture {
    Fixture::new(false, EngineLimits::default())
}
fn pending<T>(future: &mut BoxFuture<'_, T>) {
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}
fn drive(turn: &mut Turn, condition: impl Fn() -> bool) {
    for _ in 0..64 {
        if condition() {
            return;
        }
        let result = Pin::new(&mut *turn).poll_next(&mut Context::from_waker(Waker::noop()));
        assert!(!matches!(result, Poll::Ready(None | Some(Err(_)))));
    }
    assert!(condition(), "turn did not reach bounded test checkpoint");
}
fn finish(turn: Turn) {
    for event in block_on(turn.collect::<Vec<_>>()) {
        event.unwrap();
    }
}
fn conflict<T: std::fmt::Debug>(result: Result<T, EngineError>) {
    assert!(
        matches!(result, Err(EngineError::Store(error)) if error.kind == SessionStoreErrorKind::Conflict)
    );
}

#[test]
fn inert_editor_futures_and_snapshot_identity() {
    let f = fixture();
    let turn = f.turn();
    let editor = turn.metadata_editor("private-key").unwrap();
    let calls = f.store.load_calls.load(Ordering::SeqCst);
    drop(editor.read_entry());
    assert_eq!(f.store.load_calls.load(Ordering::SeqCst), calls);
    let snapshot = block_on(editor.read_entry()).unwrap();
    assert_eq!(snapshot.entry(), None);
    let saves = f.store.calls.load(Ordering::SeqCst);
    drop(editor.compare_exchange(snapshot, Some(json!("private-value"))));
    assert_eq!(f.store.calls.load(Ordering::SeqCst), saves);
    let snapshot = block_on(editor.read_entry()).unwrap();
    assert!(!format!("{editor:?} {snapshot:?}").contains("private"));
    let foreign = turn.metadata_editor("private-key").unwrap();
    assert!(block_on(foreign.compare_exchange(snapshot, None)).is_err());
    let snapshot = block_on(editor.read_entry()).unwrap();
    block_on(editor.clone().compare_exchange(snapshot, Some(json!(1)))).unwrap();
    assert_eq!(f.session.record().metadata["private-key"], json!(1));
    assert!(matches!(
        block_on(
            f.session
                .update_metadata(f.session.record().revision, BTreeMap::default())
        ),
        Err(EngineError::SessionBusy)
    ));
}

#[test]
fn permission_handler_can_await_publication_inside_turn_poll() {
    let f = Fixture::new(true, EngineLimits::default());
    let mut turn = f.turn();
    *f.policy.editor.lock().unwrap() = Some(turn.metadata_editor("rules").unwrap());
    f.store.behavior(Save::Commit); // Assistant call and result placeholder.
    f.store.behavior(Save::Wait); // Inline permission editor publication.
    f.provider.release.store(true, Ordering::Release);
    drive(&mut turn, || f.policy.calls.load(Ordering::SeqCst) == 1);
    assert_eq!(f.store.calls.load(Ordering::SeqCst), 3);
    assert!(!f.store.record().metadata.contains_key("rules"));
    f.store.release.store(true, Ordering::Release);
    finish(turn);
    assert_eq!(f.policy.calls.load(Ordering::SeqCst), 1);
    let record = f.store.record();
    assert_eq!(record.metadata["rules"], json!({"allow": true}));
    assert!(record.messages.iter().flat_map(|m| &m.content).any(|block| matches!(block, ContentBlock::ToolResult { output, .. } if output.content == json!({"executed":true}))));
}

#[test]
fn provider_wait_and_subsequent_transcript_preserve_new_metadata() {
    let f = fixture();
    let mut turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    drive(&mut turn, || f.provider.calls.load(Ordering::SeqCst) == 1);
    let snapshot = block_on(editor.read_entry()).unwrap();
    block_on(editor.compare_exchange(snapshot, Some(json!("saved")))).unwrap();
    f.provider.release.store(true, Ordering::Release);
    finish(turn);
    assert_eq!(f.store.record().metadata["rules"], json!("saved"));
    assert_eq!(
        f.store.record().messages.last().unwrap().role,
        Role::Assistant
    );
    assert!(block_on(editor.read_entry()).is_err());
}

#[test]
fn unrelated_entry_updates_survive_but_target_changes_conflict() {
    let f = fixture();
    let turn = f.turn();
    let a = turn.metadata_editor("a").unwrap();
    let b = turn.metadata_editor("b").unwrap();
    let before = block_on(a.read_entry()).unwrap();
    let stale = block_on(a.read_entry()).unwrap();
    block_on(b.compare_exchange(block_on(b.read_entry()).unwrap(), Some(json!(2)))).unwrap();
    block_on(a.compare_exchange(before, Some(json!(1)))).unwrap();
    conflict(block_on(a.compare_exchange(stale, Some(json!(3)))));
    block_on(a.compare_exchange(block_on(a.read_entry()).unwrap(), None)).unwrap();
    assert_eq!(f.store.record().metadata.len(), 1);
    assert_eq!(f.store.record().metadata["b"], json!(2));
}

#[test]
fn pending_editor_does_not_hold_turn_lease_or_edit_new_turn() {
    let f = fixture();
    let turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    f.store.behavior(Save::Wait);
    let mut edit = editor.compare_exchange(snapshot, Some(json!(1)));
    pending(&mut edit);
    assert!(matches!(
        block_on(turn.metadata_editor("other").unwrap().read_entry()),
        Err(EngineError::SessionBusy)
    ));
    drop(turn);
    assert!(!f.session.has_active_turn());
    let next = f.turn();
    assert!(block_on(edit).is_err());
    assert!(block_on(editor.read_entry()).is_err());
    assert!(!f.store.record().metadata.contains_key("rules"));
    drop(next);
}

#[test]
fn uncertain_saves_reload_and_do_not_assume_rollback() {
    for behavior in [
        Save::Fail,
        Save::CommitThenError,
        Save::CommitThenWait,
        Save::Wait,
    ] {
        let f = fixture();
        let turn = f.turn();
        let editor = turn.metadata_editor("rules").unwrap();
        let before = block_on(editor.read_entry()).unwrap();
        f.store.behavior(behavior);
        let mut edit = editor.compare_exchange(before, Some(json!("published")));
        if matches!(behavior, Save::Wait | Save::CommitThenWait) {
            pending(&mut edit);
            drop(edit);
        } else {
            assert!(!format!("{:?}", block_on(edit).unwrap_err()).contains("secret"));
        }
        let snapshot = block_on(editor.read_entry()).unwrap();
        let published = matches!(behavior, Save::CommitThenError | Save::CommitThenWait);
        assert_eq!(snapshot.entry(), published.then_some(&json!("published")));
        block_on(editor.compare_exchange(snapshot, Some(json!("confirmed")))).unwrap();
        assert_eq!(f.store.record().metadata["rules"], json!("confirmed"));
        drop(turn);
        let loads = f.store.load_calls.load(Ordering::SeqCst);
        drop(f.turn());
        assert!(f.store.load_calls.load(Ordering::SeqCst) > loads);
    }
}

#[test]
fn core_cas_retries_after_metadata_publication() {
    let f = fixture();
    let mut turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    f.provider.release.store(true, Ordering::Release);
    f.store.behavior(Save::Wait);
    drive(&mut turn, || f.store.calls.load(Ordering::SeqCst) == 2);
    block_on(editor.compare_exchange(snapshot, Some(json!(1)))).unwrap();
    f.store.release.store(true, Ordering::Release);
    finish(turn);
    assert_eq!(f.store.record().metadata["rules"], json!(1));
    assert_eq!(f.store.record().messages.len(), 2);
}

#[test]
fn editor_cas_retries_preserving_concurrent_transcript_progress() {
    let f = Fixture::new(true, EngineLimits::default());
    let mut turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    f.store.behavior(Save::Wait);
    let mut edit = editor.compare_exchange(snapshot, Some(json!(1)));
    pending(&mut edit);
    f.provider.release.store(true, Ordering::Release);
    drive(&mut turn, || f.store.record().messages.len() >= 3);
    f.store.release.store(true, Ordering::Release);
    block_on(edit).unwrap();
    finish(turn);
    assert_eq!(f.store.record().metadata["rules"], json!(1));
    assert!(f.store.record().messages.len() >= 4);
}

#[test]
fn cancellation_and_host_closure_reject_owned_operations() {
    let f = fixture();
    let turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    let _ = turn.handle().cancel();
    assert!(block_on(editor.compare_exchange(snapshot, Some(json!(1)))).is_err());
    assert!(turn.metadata_editor("new").is_err());
    drop(turn);
    let turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let before = block_on(editor.read_entry()).unwrap();
    drop(f.session);
    drop(f.engine);
    assert!(matches!(
        block_on(editor.compare_exchange(before, Some(json!(2)))),
        Err(EngineError::HostClosed)
    ));
    drop(turn);
}

#[test]
fn malformed_load_and_bounded_deep_inputs_fail_without_writes() {
    let limits = EngineLimits {
        max_session_metadata_bytes: NonZeroUsize::new(128).unwrap(),
        ..EngineLimits::default()
    };
    let f = Fixture::new(false, limits);
    let turn = f.turn();
    assert!(turn.metadata_editor(&"x".repeat(129)).is_err());
    let editor = turn.metadata_editor("rules").unwrap();
    for depth in [0, 10_000] {
        let before = block_on(editor.read_entry()).unwrap();
        let mut value = json!("x".repeat(129));
        for _ in 0..depth {
            value = Value::Array(vec![value]);
        }
        let saves = f.store.calls.load(Ordering::SeqCst);
        assert!(block_on(editor.compare_exchange(before, Some(value))).is_err());
        assert_eq!(f.store.calls.load(Ordering::SeqCst), saves);
    }
    let before = block_on(editor.read_entry()).unwrap();
    let mut deep = Value::Null;
    for _ in 0..10_000 {
        deep = Value::Array(vec![deep]);
    }
    drop(editor.compare_exchange(before, Some(deep)));
    let mut corrupt = f.store.record();
    corrupt.next_turn_sequence += 1;
    f.store.loads.lock().unwrap().push_back(Some(corrupt));
    assert!(block_on(editor.read_entry()).is_err());
    f.store.loads.lock().unwrap().push_back(None);
    assert!(block_on(editor.read_entry()).is_err());
}

#[derive(Default)]
struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn pending_editor_gets_its_own_cancellation_and_completion_wakeups() {
    for cancel in [true, false] {
        let f = fixture();
        let mut turn = f.turn();
        let editor = turn.metadata_editor("rules").unwrap();
        let snapshot = block_on(editor.read_entry()).unwrap();
        f.store.behavior(Save::Wait);
        let mut edit = editor.compare_exchange(snapshot, Some(json!(1)));
        let notifications = Arc::new(WakeCount::default());
        let waker = Waker::from(notifications.clone());
        assert!(
            edit.as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        if cancel {
            let _ = turn.handle().cancel();
        } else {
            f.provider.release.store(true, Ordering::Release);
            block_on(async {
                while let Some(event) = turn.next().await {
                    event.unwrap();
                }
            });
        }
        assert!(notifications.0.load(Ordering::SeqCst) > 0);
        assert!(block_on(edit).is_err());
    }
}

#[test]
fn confirmed_save_receipt_wins_same_poll_cancellation() {
    let f = fixture();
    let turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    *f.store.cancel_after_commit.lock().unwrap() = Some(turn.handle());
    let revision = block_on(editor.compare_exchange(snapshot, Some(json!(1)))).unwrap();
    assert_eq!(revision, f.store.record().revision);
    assert_eq!(f.session.record().metadata["rules"], json!(1));
    assert!(block_on(editor.read_entry()).is_err());
}

#[test]
fn dropped_old_save_rearms_debt_after_newer_turn_reconciliation() {
    let f = fixture();
    let turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    f.store.behavior(Save::CommitThenWait);
    let mut edit = editor.compare_exchange(snapshot, Some(json!(1)));
    pending(&mut edit);
    drop(turn);
    let next = f.turn(); // Reloads the publication, then clears reservation debt.
    assert_eq!(f.session.record().metadata["rules"], json!(1));
    let drops = f.store.drops.load(Ordering::SeqCst);
    drop(edit);
    assert_eq!(f.store.drops.load(Ordering::SeqCst), drops + 1);
    drop(next);
    let loads = f.store.load_calls.load(Ordering::SeqCst);
    drop(f.turn());
    assert!(f.store.load_calls.load(Ordering::SeqCst) > loads);
}

#[test]
fn conflict_retries_are_bounded_and_nonincreasing_or_foreign_loads_reject() {
    let f = fixture();
    let turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    f.store.conflicts.store(100, Ordering::SeqCst);
    let calls = f.store.calls.load(Ordering::SeqCst);
    conflict(block_on(editor.compare_exchange(snapshot, Some(json!(1)))));
    assert_eq!(f.store.calls.load(Ordering::SeqCst), calls + 32);
    for mutation in 0..3 {
        let mut record = f.store.record();
        match mutation {
            0 => record.incarnation_id = SessionIncarnationId::new("foreign").unwrap(),
            1 => record.revision = SessionRevision(0),
            _ => {
                record.metadata.insert("unexpected".into(), json!(1));
            }
        }
        f.store.loads.lock().unwrap().push_back(Some(record));
        assert!(block_on(editor.read_entry()).is_err());
    }
}

#[test]
fn loaded_deep_json_and_aggregate_nodes_are_bounded_before_cloning() {
    let limits = EngineLimits {
        max_json_nodes: NonZeroUsize::new(8).unwrap(),
        ..EngineLimits::default()
    };
    let f = Fixture::new(false, limits);
    let turn = f.turn();
    let editor = turn.metadata_editor("rules").unwrap();
    let snapshot = block_on(editor.read_entry()).unwrap();
    assert!(
        block_on(editor.compare_exchange(snapshot, Some(json!([0, 0, 0, 0, 0, 0, 0, 0])))).is_err()
    );
    let mut loaded = f.store.record();
    let mut value = Value::Null;
    for _ in 0..10_000 {
        value = Value::Array(vec![value]);
    }
    loaded.metadata.insert("deep".into(), value);
    f.store.loads.lock().unwrap().push_back(Some(loaded));
    assert!(block_on(editor.read_entry()).is_err());
    assert!(f.session.record().metadata.is_empty());
}

#[test]
fn metadata_byte_boundary_is_inclusive_for_key_admission_and_replacement() {
    let limits = EngineLimits {
        max_session_metadata_bytes: NonZeroUsize::new(7).unwrap(),
        ..EngineLimits::default()
    };
    let f = Fixture::new(false, limits);
    let turn = f.turn();
    let editor = turn.metadata_editor("a").unwrap();
    block_on(editor.compare_exchange(block_on(editor.read_entry()).unwrap(), Some(json!(0))))
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&f.store.record().metadata)
            .unwrap()
            .len(),
        7
    );
    assert!(
        block_on(editor.compare_exchange(block_on(editor.read_entry()).unwrap(), Some(json!(10))))
            .is_err()
    );
    assert!(turn.metadata_editor("ab").is_err());
}
