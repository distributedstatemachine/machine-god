//! Composed evidence producers: native tools, core orchestration, native ownership.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_core::Stream;
use futures_executor::block_on;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineEvent, EventSink, EventSinkError,
    InferenceOptions, ModelEvent, PermissionDecision, PermissionGrantScope, PreparedToolCall, Role,
    SessionId, SessionIncarnationId, SessionRecord, SessionRevision, SessionStore,
    SessionStoreError, SessionStoreErrorKind, StopReason, Tool, ToolCall, ToolCallId, ToolContext,
    ToolError, ToolName, ToolOutput, ToolSpec, TurnEvent,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::{Value, json};

use crate::file_history_tool::{NativeFileHistoryKind, NativeFileHistoryTool};
use crate::{
    NATIVE_SESSION_METADATA_KEY, NativeConversation, NativeConversationError,
    NativeConversationHistory, NativeConversationObservations, NativeConversationRuntime,
    NativeConversationTurn, NativeHistoryFileAction as Action, NativeHistoryFileEvidence,
    NativeHistoryFileStatus as Status, NativeHistoryState, NativeModelPreferences,
    NativeSessionMetadata, NativeSessionOrigin, ReadFileTool, WriteFileTool,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        for _ in 0..1000 {
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mg-observation-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("cannot create observation workspace: {error}"),
            }
        }
        panic!("observation workspace capacity exhausted");
    }
    fn file(&self) -> PathBuf {
        self.0.join("note.txt")
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.0);
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

#[derive(Clone, Copy)]
enum FinalizerFault {
    Error,
    Pending,
    PublishThenError,
    ReadResultError,
}

#[derive(Clone)]
struct FaultStore {
    inner: InMemorySessionStore,
    fault: Arc<Mutex<Option<FinalizerFault>>>,
    hits: Arc<AtomicUsize>,
    saves: Arc<AtomicUsize>,
}
impl SessionStore for FaultStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.inner.load(id)
    }
    fn save(
        &self,
        record: SessionRecord,
        expected: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async move {
            self.saves.fetch_add(1, Ordering::SeqCst);
            let finalizing = NativeConversationHistory::from_record(&record)
                .unwrap()
                .groups()
                .last()
                .is_some_and(|group| group.state() != NativeHistoryState::Running);
            let read_result = record.messages.iter().flat_map(|message| &message.content)
                .any(|block| matches!(block, ContentBlock::ToolResult { call_id, output }
                    if call_id.as_str() == "read" && !output.is_error && output.content.get("content").is_some()));
            let fault = {
                let mut fault = self.fault.lock().unwrap();
                let targeted = match *fault {
                    Some(FinalizerFault::ReadResultError) => read_result,
                    Some(_) => finalizing,
                    None => false,
                };
                if targeted { fault.take() } else { None }
            };
            if fault.is_some() {
                self.hits.fetch_add(1, Ordering::SeqCst);
            }
            match fault {
                Some(FinalizerFault::Error | FinalizerFault::ReadResultError) => Err(store_error()),
                Some(FinalizerFault::Pending) => std::future::pending().await,
                Some(FinalizerFault::PublishThenError) => {
                    self.inner.save(record, expected).await?;
                    Err(store_error())
                }
                None => self.inner.save(record, expected).await,
            }
        })
    }
}
fn store_error() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "observation_fixture",
        "injected metadata publication failure",
        false,
    )
}

/// Performs the real native mutation before optionally hiding its result forever.
/// The observation adapter surrounds this tool, so dropping leaves Unknown.
struct CountedWrite {
    inner: WriteFileTool,
    calls: Arc<AtomicUsize>,
    pause: bool,
}
impl Tool for CountedWrite {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        self.inner.prepare(call)
    }
    fn execute(
        &self,
        context: ToolContext,
        args: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let result = self.inner.execute(context, args, cancellation).await;
            if self.pause {
                std::future::pending::<()>().await;
            }
            result
        })
    }
}

struct RejectToolFinished(bool);
impl EventSink for RejectToolFinished {
    fn emit(&self, event: EngineEvent) -> BoxFuture<'_, Result<(), EventSinkError>> {
        Box::pin(async move {
            if self.0 && matches!(event.payload, TurnEvent::ToolFinished { .. }) {
                Err(EventSinkError::new(
                    "observation_sink",
                    "hide saved tool result",
                ))
            } else {
                Ok(())
            }
        })
    }
}

struct Harness {
    conversation: NativeConversation,
    store: FaultStore,
    provider: ScriptedModelProvider,
    permissions: ScriptedPermissionHandler,
    writes: Arc<AtomicUsize>,
}

fn setup(
    root: &Path,
    steps: Vec<ModelProviderStep>,
    deny: bool,
    pause_write: bool,
    fault: Option<FinalizerFault>,
    reject_finished: bool,
) -> Harness {
    let mut record = SessionRecord::empty(
        SessionId::new("observed").unwrap(),
        SessionIncarnationId::new("observed-life").unwrap(),
    );
    record.revision = SessionRevision(1);
    record.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        NativeSessionMetadata::new(root, 100, NativeSessionOrigin::Cli)
            .unwrap()
            .to_value(),
    );
    let store = FaultStore {
        inner: InMemorySessionStore::from_records(BTreeMap::from([(
            record.id.clone(),
            record.clone(),
        )])),
        fault: Arc::new(Mutex::new(fault)),
        hits: Arc::new(AtomicUsize::new(0)),
        saves: Arc::new(AtomicUsize::new(0)),
    };
    let provider = ScriptedModelProvider::new("observed", steps);
    let permissions = ScriptedPermissionHandler::new((0..16).map(|_| {
        PermissionStep::Decision(if deny {
            PermissionDecision::Deny {
                reason: "fixture denies".to_owned(),
            }
        } else {
            PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            }
        })
    }));
    let observations = Arc::new(NativeConversationObservations::new());
    let writes = Arc::new(AtomicUsize::new(0));
    let engine = Engine::builder()
        .session_store(store.clone())
        .provider(provider.clone())
        .permission_handler(permissions.clone())
        .event_sink(RejectToolFinished(reject_finished))
        .tool(NativeFileHistoryTool::new(
            ReadFileTool::open(root).unwrap(),
            NativeFileHistoryKind::Read,
            observations.clone(),
        ))
        .tool(NativeFileHistoryTool::new(
            CountedWrite {
                inner: WriteFileTool::open(root).unwrap(),
                calls: writes.clone(),
                pause: pause_write,
            },
            NativeFileHistoryKind::Write,
            observations.clone(),
        ))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(record.id)).unwrap().unwrap();
    let conversation = NativeConversation::from_session(session)
        .unwrap()
        .with_observations(&observations)
        .unwrap();
    Harness {
        conversation,
        store,
        provider,
        permissions,
        writes,
    }
}

fn call(id: &str, name: &str, arguments: Value) -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::ToolCall {
            call: ToolCall {
                id: ToolCallId::new(id).unwrap(),
                name: ToolName::new(name).unwrap(),
                arguments,
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolCalls,
        },
    ])
}
fn read(id: &str) -> ModelProviderStep {
    call(id, "read_file", json!({"path":"./note.txt"}))
}
fn write(id: &str) -> ModelProviderStep {
    call(
        id,
        "write_file",
        json!({"path":"./note.txt", "content":"after"}),
    )
}
fn finished() -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: "done".to_owned(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}
fn start(harness: &Harness) -> NativeConversationTurn {
    block_on(harness.conversation.prompt("observe files".into(), 200)).unwrap()
}
fn complete(turn: NativeConversationTurn) {
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    let events: Vec<_> = events.into_iter().map(Result::unwrap).collect();
    assert!(matches!(
        events.last().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
}
fn poll_until_pending(turn: &mut NativeConversationTurn) -> Vec<EngineEvent> {
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);
    let mut events = Vec::new();
    for _ in 0..128 {
        match Pin::new(&mut *turn).poll_next(&mut context) {
            Poll::Pending => return events,
            Poll::Ready(Some(Ok(event))) => events.push(event),
            other @ Poll::Ready(_) => {
                panic!("expected an admitted pending boundary, got {other:?}")
            }
        }
    }
    panic!("pending boundary was not reached");
}
fn assert_source(record: &SessionRecord, file: &NativeHistoryFileEvidence, id: &str, name: &str) {
    let source = file
        .source()
        .expect("real execution must name its canonical call");
    assert_eq!(source.call_id().as_str(), id);
    assert_eq!(source.tool_name().as_str(), name);
    let message = &record.messages[source.assistant_message()];
    assert_eq!(message.role, Role::Assistant);
    let ContentBlock::ToolCall { call } = &message.content[source.content_block()] else {
        panic!("source must name tool call")
    };
    assert_eq!(call.id, *source.call_id());
    assert_eq!(call.name, *source.tool_name());
}
fn assert_no_more_flushes(harness: &Harness) {
    let before = harness.conversation.record();
    let saves = harness.store.saves.load(Ordering::SeqCst);
    assert_eq!(
        block_on(harness.conversation.flush_history_observations(400)).unwrap(),
        None
    );
    assert_eq!(harness.conversation.record(), before);
    assert_eq!(harness.store.saves.load(Ordering::SeqCst), saves);
}

#[test]
fn real_read_write_read_records_canonical_sources_staleness_status_and_full_view() {
    let root = Workspace::new();
    fs::write(root.file(), "before").unwrap();
    let harness = setup(
        &root.0,
        vec![
            read("first"),
            write("write"),
            read("second"),
            call("missing", "read_file", json!({"path":"missing.txt"})),
            finished(),
        ],
        false,
        false,
        None,
        false,
    );
    complete(start(&harness));
    assert_eq!(fs::read_to_string(root.file()).unwrap(), "after");
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
    assert_eq!(harness.permissions.requests().len(), 4);
    let history = harness.conversation.history().unwrap();
    let group = history.group(0).unwrap();
    assert_eq!(group.state(), NativeHistoryState::Completed);
    assert_eq!(group.turn_sequence(), 1);
    let files = group.files();
    assert_eq!(files.len(), 4);
    let record = harness.conversation.record();
    for (index, (id, name, action, status, stale, full)) in [
        (
            "first",
            "read_file",
            Action::Read,
            Status::Success,
            true,
            true,
        ),
        (
            "write",
            "write_file",
            Action::Write,
            Status::Success,
            false,
            false,
        ),
        (
            "second",
            "read_file",
            Action::Read,
            Status::Success,
            false,
            true,
        ),
        (
            "missing",
            "read_file",
            Action::Read,
            Status::Failure,
            false,
            false,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_source(&record, &files[index], id, name);
        assert_eq!(
            files[index].path(),
            if index == 3 {
                "missing.txt"
            } else {
                "note.txt"
            }
        );
        assert_eq!(files[index].action(), action);
        assert_eq!(files[index].status(), status);
        assert_eq!(files[index].stale(), stale);
        assert_eq!(files[index].model_view_covers_full_file(), full);
        assert_eq!(files[index].new_path(), None);
    }
    assert_ne!(files[0].source(), files[2].source());
    assert_eq!(
        NativeConversationHistory::from_record(
            &harness
                .store
                .inner
                .record(&harness.conversation.id())
                .unwrap()
        )
        .unwrap(),
        history
    );
    assert_no_more_flushes(&harness);
}

#[test]
fn denied_real_file_calls_never_produce_execution_facts() {
    let root = Workspace::new();
    fs::write(root.file(), "before").unwrap();
    let harness = setup(
        &root.0,
        vec![read("denied-read"), write("denied-write"), finished()],
        true,
        false,
        None,
        false,
    );
    complete(start(&harness));
    assert_eq!(harness.permissions.requests().len(), 2);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read_to_string(root.file()).unwrap(), "before");
    assert!(
        harness
            .conversation
            .history()
            .unwrap()
            .group(0)
            .unwrap()
            .files()
            .is_empty()
    );
    assert_no_more_flushes(&harness);
}

#[test]
fn text_and_multiple_calls_use_canonical_result_ordinals_for_full_read_evidence() {
    let root = Workspace::new();
    fs::write(root.file(), "before").unwrap();
    let calls = ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: "Reading twice.".to_owned(),
        },
        ModelEvent::ToolCall {
            call: ToolCall {
                id: ToolCallId::new("first").unwrap(),
                name: ToolName::new("read_file").unwrap(),
                arguments: json!({"path":"./note.txt"}),
            },
        },
        ModelEvent::ToolCall {
            call: ToolCall {
                id: ToolCallId::new("second").unwrap(),
                name: ToolName::new("read_file").unwrap(),
                arguments: json!({"path":"./note.txt"}),
            },
        },
        ModelEvent::Stop {
            reason: StopReason::ToolCalls,
        },
    ]);
    let harness = setup(&root.0, vec![calls, finished()], false, false, None, false);
    complete(start(&harness));
    let history = harness.conversation.history().unwrap();
    let files = history.group(0).unwrap().files();
    assert_eq!(files.len(), 2);
    let record = harness.conversation.record();
    for (index, id) in ["first", "second"].into_iter().enumerate() {
        assert_source(&record, &files[index], id, "read_file");
        assert_eq!(files[index].source().unwrap().content_block(), index + 1);
        assert_eq!(files[index].status(), Status::Success);
        assert!(files[index].model_view_covers_full_file());
        assert!(!files[index].stale());
    }
    assert_eq!(
        files[0].source().unwrap().assistant_message(),
        files[1].source().unwrap().assistant_message()
    );
    assert_no_more_flushes(&harness);
}

#[test]
fn successful_read_with_failed_result_publication_does_not_claim_a_full_model_view() {
    let root = Workspace::new();
    fs::write(root.file(), "before").unwrap();
    let harness = setup(
        &root.0,
        vec![read("read"), finished()],
        false,
        false,
        Some(FinalizerFault::ReadResultError),
        false,
    );
    let events = block_on(start(&harness).collect::<Vec<_>>());
    assert!(events.iter().any(|event| event.is_err()
        || matches!(
            event,
            Ok(EngineEvent {
                payload: TurnEvent::Failed { .. },
                ..
            })
        )));
    assert_eq!(harness.store.hits.load(Ordering::SeqCst), 1);
    let history = harness.conversation.history().unwrap();
    let files = history.group(0).unwrap().files();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status(), Status::Success);
    assert!(!files[0].model_view_covers_full_file());
    let record = harness
        .store
        .inner
        .record(&harness.conversation.id())
        .unwrap();
    assert_source(&record, &files[0], "read", "read_file");
    assert!(record.messages.iter().flat_map(|message| &message.content).any(|block|
        matches!(block, ContentBlock::ToolResult { call_id, output }
            if call_id.as_str() == "read" && output.is_error
                && output.content.get("code").and_then(Value::as_str) == Some("tool_result_unknown"))));
    assert_eq!(harness.provider.requests().len(), 1);
    assert_no_more_flushes(&harness);
}

#[test]
fn dropped_execution_flushes_unknown_committed_write_without_reexecution() {
    let root = Workspace::new();
    let harness = setup(&root.0, vec![write("uncertain")], false, true, None, false);
    let mut turn = start(&harness);
    let events = poll_until_pending(&mut turn);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
    );
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_to_string(root.file()).unwrap(), "after");
    drop(turn);
    assert!(!harness.conversation.is_busy());
    assert!(
        harness
            .conversation
            .paused_turn()
            .unwrap()
            .unwrap()
            .has_uncertain_tool_results
    );
    assert!(
        harness
            .conversation
            .history()
            .unwrap()
            .group(0)
            .unwrap()
            .files()
            .is_empty()
    );
    let saves = harness.store.saves.load(Ordering::SeqCst);
    let flush = harness.conversation.flush_history_observations(300);
    assert_eq!(harness.store.saves.load(Ordering::SeqCst), saves);
    assert!(block_on(flush).unwrap().is_some());
    let history = harness.conversation.history().unwrap();
    let files = history.group(0).unwrap().files();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status(), Status::Unknown);
    assert!(!files[0].model_view_covers_full_file());
    assert_source(
        &harness.conversation.record(),
        &files[0],
        "uncertain",
        "write_file",
    );
    assert_no_more_flushes(&harness);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
    assert_eq!(harness.provider.requests().len(), 1);
}

#[test]
fn tool_finished_sink_failure_does_not_hide_saved_execution_evidence() {
    let root = Workspace::new();
    let harness = setup(
        &root.0,
        vec![write("saved"), finished()],
        false,
        false,
        None,
        true,
    );
    let events = block_on(start(&harness).collect::<Vec<_>>());
    assert!(!events.iter().any(|event| matches!(
        event,
        Ok(EngineEvent {
            payload: TurnEvent::ToolFinished { .. },
            ..
        })
    )));
    assert!(events.iter().any(|event| event.is_err()
        || matches!(
            event,
            Ok(EngineEvent {
                payload: TurnEvent::Failed { .. },
                ..
            })
        )));
    let history = harness.conversation.history().unwrap();
    let group = history.group(0).unwrap();
    assert_eq!(group.state(), NativeHistoryState::Failed);
    assert_eq!(group.files().len(), 1);
    assert_eq!(group.files()[0].status(), Status::Success);
    let record = harness
        .store
        .inner
        .record(&harness.conversation.id())
        .unwrap();
    assert_source(&record, &group.files()[0], "saved", "write_file");
    assert!(record.messages.iter().flat_map(|message| &message.content).any(|block| matches!(block, ContentBlock::ToolResult { call_id, output } if call_id.as_str() == "saved" && !output.is_error)));
    assert_eq!(fs::read_to_string(root.file()).unwrap(), "after");
    assert_eq!(harness.provider.requests().len(), 1);
    assert_no_more_flushes(&harness);
}

#[test]
fn saved_read_remains_fully_visible_when_tool_finished_sink_fails() {
    let root = Workspace::new();
    fs::write(root.file(), "before").unwrap();
    let harness = setup(
        &root.0,
        vec![read("read"), finished()],
        false,
        false,
        None,
        true,
    );
    let events = block_on(start(&harness).collect::<Vec<_>>());
    assert!(!events.iter().any(|event| matches!(
        event,
        Ok(EngineEvent {
            payload: TurnEvent::ToolFinished { .. },
            ..
        })
    )));
    let history = harness.conversation.history().unwrap();
    let group = history.group(0).unwrap();
    assert_eq!(group.state(), NativeHistoryState::Failed);
    assert_eq!(group.files().len(), 1);
    assert_eq!(group.files()[0].status(), Status::Success);
    assert!(group.files()[0].model_view_covers_full_file());
    assert_no_more_flushes(&harness);
}

#[test]
fn dropping_after_tool_started_before_acknowledgement_does_not_record_execution() {
    let root = Workspace::new();
    let harness = setup(
        &root.0,
        vec![write("never-started")],
        false,
        false,
        None,
        false,
    );
    let mut turn = start(&harness);
    block_on(async {
        loop {
            let event = turn
                .next()
                .await
                .expect("tool start must be emitted")
                .unwrap();
            if matches!(event.payload, TurnEvent::ToolStarted { .. }) {
                break;
            }
        }
    });
    assert_eq!(harness.writes.load(Ordering::SeqCst), 0);
    drop(turn);
    assert!(!root.file().exists());
    assert!(
        harness
            .conversation
            .history()
            .unwrap()
            .group(0)
            .unwrap()
            .files()
            .is_empty()
    );
    assert_no_more_flushes(&harness);
}

#[test]
fn failed_native_finalizer_retains_completed_observations_until_explicit_flush() {
    let root = Workspace::new();
    let harness = setup(
        &root.0,
        vec![write("saved"), finished()],
        false,
        false,
        Some(FinalizerFault::Error),
        false,
    );
    let events = block_on(start(&harness).collect::<Vec<_>>());
    assert_eq!(
        events.last().unwrap().as_ref().unwrap_err(),
        &NativeConversationError::Persistence
    );
    assert_eq!(harness.store.hits.load(Ordering::SeqCst), 1);
    assert!(
        harness
            .conversation
            .history()
            .unwrap()
            .group(0)
            .unwrap()
            .files()
            .is_empty()
    );
    assert!(
        block_on(harness.conversation.flush_history_observations(300))
            .unwrap()
            .is_some()
    );
    let history = harness.conversation.history().unwrap();
    assert_eq!(history.group(0).unwrap().files().len(), 1);
    assert_eq!(
        history.group(0).unwrap().files()[0].status(),
        Status::Success
    );
    assert_no_more_flushes(&harness);
    assert_eq!(harness.provider.requests().len(), 2);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn dropped_pending_native_finalizer_does_not_acknowledge_its_batch() {
    let root = Workspace::new();
    let harness = setup(
        &root.0,
        vec![write("saved"), finished()],
        false,
        false,
        Some(FinalizerFault::Pending),
        false,
    );
    let mut turn = start(&harness);
    let events = poll_until_pending(&mut turn);
    assert_eq!(harness.store.hits.load(Ordering::SeqCst), 1);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.payload, TurnEvent::Completed { .. }))
    );
    assert!(harness.conversation.is_busy());
    assert_eq!(
        block_on(harness.conversation.flush_history_observations(300)),
        Err(NativeConversationError::Busy)
    );
    drop(turn);
    assert!(!harness.conversation.is_busy());
    assert!(
        block_on(harness.conversation.flush_history_observations(300))
            .unwrap()
            .is_some()
    );
    let history = harness.conversation.history().unwrap();
    assert_eq!(history.group(0).unwrap().files().len(), 1);
    assert_eq!(
        history.group(0).unwrap().files()[0].status(),
        Status::Success
    );
    assert_no_more_flushes(&harness);
    assert_eq!(harness.provider.requests().len(), 2);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn published_then_failed_finalizer_reconciles_without_duplicate_or_downgraded_facts() {
    let root = Workspace::new();
    let harness = setup(
        &root.0,
        vec![read("read"), write("write"), finished()],
        false,
        false,
        Some(FinalizerFault::PublishThenError),
        false,
    );
    fs::write(root.file(), "before").unwrap();
    let events = block_on(start(&harness).collect::<Vec<_>>());
    assert_eq!(
        events.last().unwrap().as_ref().unwrap_err(),
        &NativeConversationError::Persistence
    );
    let published = harness
        .store
        .inner
        .record(&harness.conversation.id())
        .unwrap();
    let saved_history = NativeConversationHistory::from_record(&published).unwrap();
    assert!(saved_history.group(0).unwrap().files()[0].stale());
    // Core refreshes uncertain metadata but rejects the caller's old revision.
    assert_eq!(
        block_on(harness.conversation.flush_history_observations(300)),
        Err(NativeConversationError::Conflict)
    );
    assert!(
        block_on(harness.conversation.flush_history_observations(300))
            .unwrap()
            .is_some()
    );
    assert_eq!(harness.conversation.history().unwrap(), saved_history);
    assert_no_more_flushes(&harness);
    assert_eq!(harness.provider.requests().len(), 3);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn continuation_merges_old_attempt_observations_before_advancing_its_sequence() {
    let root = Workspace::new();
    let harness = setup(
        &root.0,
        vec![write("uncertain"), finished()],
        false,
        true,
        None,
        false,
    );
    let mut turn = start(&harness);
    poll_until_pending(&mut turn);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
    drop(turn);
    let continuation = block_on(
        harness
            .conversation
            .continue_turn(InferenceOptions::default(), 300),
    )
    .unwrap();
    assert_eq!(harness.provider.requests().len(), 1);
    let reserved = harness
        .store
        .inner
        .record(&harness.conversation.id())
        .unwrap();
    let history = NativeConversationHistory::from_record(&reserved).unwrap();
    let group = history.group(0).unwrap();
    assert_eq!(group.turn_sequence(), 2);
    assert_eq!(group.state(), NativeHistoryState::Running);
    assert_eq!(group.files().len(), 1);
    assert_eq!(group.files()[0].status(), Status::Unknown);
    assert_source(&reserved, &group.files()[0], "uncertain", "write_file");
    complete(continuation);
    assert_eq!(
        harness
            .conversation
            .history()
            .unwrap()
            .group(0)
            .unwrap()
            .files(),
        group.files()
    );
    assert_eq!(
        harness
            .conversation
            .record()
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .count(),
        1
    );
    assert_eq!(harness.provider.requests().len(), 2);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
    assert_no_more_flushes(&harness);
}

#[test]
fn runtime_flush_preserves_queued_input_and_dirty_model_generation() {
    let root = Workspace::new();
    let harness = setup(&root.0, vec![write("uncertain")], false, true, None, false);
    let mut turn = start(&harness);
    poll_until_pending(&mut turn);
    drop(turn);
    let runtime = NativeConversationRuntime::new(
        harness.conversation,
        NativeModelPreferences::default(),
        None,
    )
    .unwrap();
    let queued = runtime.enqueue("still pending".into()).unwrap();
    let mut preferences = runtime.model_preferences();
    preferences.set_model("private/next-model").unwrap();
    runtime.set_model_preferences(preferences.clone()).unwrap();
    let status = runtime.status();
    assert!(status.model_preferences_pending);
    assert_eq!(status.queued_jobs, 1);
    let saves = harness.store.saves.load(Ordering::SeqCst);
    let flush = runtime.flush_history_observations(300);
    assert_eq!(harness.store.saves.load(Ordering::SeqCst), saves);
    assert!(block_on(flush).unwrap().is_some());
    assert_eq!(runtime.status(), status);
    assert_eq!(runtime.model_preferences(), preferences);
    assert_eq!(
        runtime.history().unwrap().group(0).unwrap().files()[0].status(),
        Status::Unknown
    );
    assert_eq!(
        block_on(runtime.flush_history_observations(400)).unwrap(),
        None
    );
    assert!(runtime.cancel_queued(queued));
    assert_eq!(harness.provider.requests().len(), 1);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 1);
}
