#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::VecDeque;
use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use futures_util::{StreamExt, stream};
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineEvent, ModelEvent, ModelEventStream,
    ModelProvider, ModelRequest, PermissionDecision, PermissionGrantScope, ProviderError, Role,
    SessionId, SessionRecord, SessionRevision, SessionStore, StopReason, Tool, ToolCall,
    ToolCallId, ToolContext, ToolError, ToolName, ToolOutput, ToolSpec, TurnEvent,
};
use machine_god_native::FileSessionStore;
use machine_god_testkit::{PermissionStep, ScriptedPermissionHandler};
use serde_json::{Value, json};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-file-session-engine-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove a temporary directory: {error}"),
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("native session store unexpectedly remained pending"),
    }
}

fn durable_snapshot(store: &FileSessionStore, id: SessionId) -> SessionRecord {
    ready(store.load(id))
        .expect("durable observation succeeds")
        .expect("durable record exists")
}

#[derive(Debug)]
struct ObservingProvider {
    store: Arc<FileSessionStore>,
    responses: Mutex<VecDeque<Vec<ModelEvent>>>,
    observations: Arc<Mutex<Vec<SessionRecord>>>,
}

impl ObservingProvider {
    fn new(
        store: Arc<FileSessionStore>,
        responses: impl IntoIterator<Item = Vec<ModelEvent>>,
    ) -> Self {
        Self {
            store,
            responses: Mutex::new(responses.into_iter().collect()),
            observations: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn observations(&self) -> Arc<Mutex<Vec<SessionRecord>>> {
        Arc::clone(&self.observations)
    }
}

impl ModelProvider for ObservingProvider {
    fn name(&self) -> &'static str {
        "durable-observer"
    }

    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelEventStream, ProviderError>> {
        let persisted = durable_snapshot(&self.store, request.session_id);
        self.observations.lock().unwrap().push(persisted);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("provider response script is not exhausted");
        Box::pin(async move {
            Ok(Box::pin(stream::iter(response.into_iter().map(Ok))) as ModelEventStream)
        })
    }
}

#[derive(Debug)]
struct ObservingTool {
    store: Arc<FileSessionStore>,
    observations: Arc<Mutex<Vec<SessionRecord>>>,
}

impl Tool for ObservingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("durable_tool").unwrap(),
            description: "Observe durable ordering".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"value": {"type": "integer"}},
                "required": ["value"],
                "additionalProperties": false,
            }),
        }
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        self.observations
            .lock()
            .unwrap()
            .push(durable_snapshot(&self.store, context.session_id));
        Box::pin(async move { Ok(ToolOutput::success(json!({"echo": arguments["value"]}))) })
    }
}

fn collect_turn(session: &machine_god_core::Session, prompt: &str) -> (String, Vec<EngineEvent>) {
    futures_executor::block_on(async {
        let turn = session.prompt(prompt).await.unwrap();
        let turn_id = turn.id().to_string();
        let events = turn
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        (turn_id, events)
    })
}

fn completed_response(text: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::TextDelta {
            text: text.to_owned(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ]
}

#[test]
fn engine_persists_prompt_placeholders_results_and_final_text_before_consumers() {
    let temporary = TemporaryDirectory::new();
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let call_id = ToolCallId::new("durable-call").unwrap();
    let provider = ObservingProvider::new(
        Arc::clone(&store),
        [
            vec![
                ModelEvent::TextDelta {
                    text: "working".to_owned(),
                },
                ModelEvent::ToolCall {
                    call: ToolCall {
                        id: call_id.clone(),
                        name: ToolName::new("durable_tool").unwrap(),
                        arguments: json!({"value": 7}),
                    },
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ],
            completed_response("final answer"),
        ],
    );
    let provider_observations = provider.observations();
    let tool_observations = Arc::new(Mutex::new(Vec::new()));
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let shared_store: Arc<dyn SessionStore> = store.clone();
    let engine = Engine::builder()
        .provider(provider)
        .shared_session_store(shared_store)
        .permission_handler(policy)
        .tool(ObservingTool {
            store: Arc::clone(&store),
            observations: Arc::clone(&tool_observations),
        })
        .build()
        .unwrap();
    let session_id = SessionId::new("durable-sequencing").unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            machine_god_core::SessionIncarnationId::new("durable-incarnation").unwrap(),
        )
        .unwrap();

    let (turn_id, events) = collect_turn(&session, "persist me first");
    assert_eq!(turn_id, "turn-1");
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));

    let provider_records = provider_observations.lock().unwrap();
    assert_eq!(provider_records.len(), 2);
    assert_eq!(provider_records[0].revision, SessionRevision(1));
    assert_eq!(provider_records[0].next_turn_sequence, 2);
    assert_eq!(provider_records[0].messages.len(), 1);
    assert_eq!(provider_records[0].messages[0].role, Role::User);
    assert!(matches!(
        &provider_records[0].messages[0].content[..],
        [ContentBlock::Text { text }] if text == "persist me first"
    ));

    let tool_records = tool_observations.lock().unwrap();
    assert_eq!(tool_records.len(), 1);
    assert_eq!(tool_records[0].revision, SessionRevision(2));
    assert_eq!(tool_records[0].messages.len(), 3);
    assert_eq!(tool_records[0].messages[1].role, Role::Assistant);
    assert!(matches!(
        &tool_records[0].messages[1].content[..],
        [ContentBlock::Text { text }, ContentBlock::ToolCall { call }]
            if text == "working" && call.id == call_id
    ));
    let [ContentBlock::ToolResult { output, .. }] = &tool_records[0].messages[2].content[..] else {
        panic!("expected durable result placeholder before tool startup")
    };
    assert_eq!(output.content["code"], "tool_result_unknown");
    assert!(output.is_error);

    assert_eq!(provider_records[1].revision, SessionRevision(3));
    assert_eq!(provider_records[1].messages.len(), 3);
    let [ContentBlock::ToolResult { output, .. }] = &provider_records[1].messages[2].content[..]
    else {
        panic!("expected durable tool result before the next provider round")
    };
    assert_eq!(output, &ToolOutput::success(json!({"echo": 7})));
    drop(tool_records);
    drop(provider_records);

    let final_record = durable_snapshot(&store, session_id);
    assert_eq!(final_record.revision, SessionRevision(4));
    assert_eq!(final_record.messages.len(), 4);
    assert!(matches!(
        &final_record.messages[3].content[..],
        [ContentBlock::Text { text }] if text == "final answer"
    ));
}

#[test]
fn independent_engine_reconstructs_transcript_and_continues_turn_sequence() {
    let temporary = TemporaryDirectory::new();
    let session_id = SessionId::new("durable-reconstruction").unwrap();
    let incarnation = machine_god_core::SessionIncarnationId::new("reconstructed-life").unwrap();

    {
        let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
        let provider =
            ObservingProvider::new(Arc::clone(&store), [completed_response("first answer")]);
        let observations = provider.observations();
        let shared_store: Arc<dyn SessionStore> = store;
        let engine = Engine::builder()
            .provider(provider)
            .shared_session_store(shared_store)
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let session = engine
            .create_session(session_id.clone(), incarnation.clone())
            .unwrap();
        let (turn_id, _) = collect_turn(&session, "first prompt");
        assert_eq!(turn_id, "turn-1");
        let records = observations.lock().unwrap();
        assert_eq!(records[0].messages.len(), 1);
    }

    let reopened = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let provider =
        ObservingProvider::new(Arc::clone(&reopened), [completed_response("second answer")]);
    let observations = provider.observations();
    let shared_store: Arc<dyn SessionStore> = reopened.clone();
    let engine = Engine::builder()
        .provider(provider)
        .shared_session_store(shared_store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let loaded = futures_executor::block_on(engine.load_session(session_id.clone()))
        .unwrap()
        .expect("first engine left a durable session");
    assert_eq!(loaded.incarnation_id(), incarnation);
    assert_eq!(loaded.record().revision, SessionRevision(2));
    assert_eq!(loaded.record().next_turn_sequence, 2);

    let (turn_id, events) = collect_turn(&loaded, "second prompt");
    assert_eq!(turn_id, "turn-2");
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed { .. })
    ));
    let records = observations.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].revision, SessionRevision(3));
    assert_eq!(records[0].next_turn_sequence, 3);
    assert_eq!(records[0].messages.len(), 3);
    assert!(matches!(
        &records[0].messages[0].content[..],
        [ContentBlock::Text { text }] if text == "first prompt"
    ));
    assert!(matches!(
        &records[0].messages[1].content[..],
        [ContentBlock::Text { text }] if text == "first answer"
    ));
    assert!(matches!(
        &records[0].messages[2].content[..],
        [ContentBlock::Text { text }] if text == "second prompt"
    ));
    drop(records);

    let final_record = durable_snapshot(&reopened, session_id);
    assert_eq!(final_record.revision, SessionRevision(4));
    assert_eq!(final_record.next_turn_sequence, 3);
    assert_eq!(final_record.messages.len(), 4);
    assert!(matches!(
        &final_record.messages[3].content[..],
        [ContentBlock::Text { text }] if text == "second answer"
    ));
}
