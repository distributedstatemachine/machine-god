#![cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]

use std::fs;
use std::future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, Engine, EngineEvent, Message,
    ModelEvent, NetworkTarget, PermissionDecision, PermissionGrantScope, Role, SessionId,
    SessionIncarnationId, StopReason, ToolCall, ToolCallId, ToolName, ToolOutput, TurnEvent,
};
use machine_god_native::{
    MAX_VISION_BATCH_BYTES, MAX_VISION_BATCH_IMAGES, VISION_TOOL_NAME, VisionBatchRequest,
    VisionBatchResponse, VisionDeadline, VisionImageOutcome, VisionImageResult, VisionMediaType,
    VisionTool, VisionTransport, VisionTransportError,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, ScriptedModelProvider,
    ScriptedPermissionHandler,
};
use serde_json::{Value, json};

const IMAGE_PATH: &str = "screenshots/login.png";
const FOCUS: &str = "Read the visible status and describe the badge.";
const IMAGE_BYTE_SENTINEL: &str = "PRIVATE_IMAGE_BYTES_SENTINEL";
const CANCELLED_EVIDENCE_SENTINEL: &str = "PRIVATE_PARTIAL_VISUAL_EVIDENCE";
const PNG_BASE64_PREFIX: &str = "iVBORw0KGgo";

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-vision-engine-{}-{identifier}",
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

struct NeverDeadline;

impl VisionDeadline for NeverDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>> {
        Box::pin(future::pending())
    }
}

#[derive(Clone, Copy)]
enum TransportMode {
    Success,
    CancelThenSuccess,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestRecord {
    session_id: String,
    focus: String,
    images: Vec<(u64, VisionMediaType, Vec<u8>)>,
}

#[derive(Default)]
struct TransportState {
    calls: AtomicUsize,
    requests: Mutex<Vec<RequestRecord>>,
}

#[derive(Clone)]
struct FakeTransport {
    mode: TransportMode,
    state: Arc<TransportState>,
}

impl FakeTransport {
    fn new(mode: TransportMode) -> Self {
        Self {
            mode,
            state: Arc::new(TransportState::default()),
        }
    }

    fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<RequestRecord> {
        self.state.requests.lock().unwrap().clone()
    }
}

impl VisionTransport for FakeTransport {
    fn analyze(
        &self,
        request: VisionBatchRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<VisionBatchResponse, VisionTransportError>> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        self.state.requests.lock().unwrap().push(RequestRecord {
            session_id: request.session_id().as_str().to_owned(),
            focus: request.focus().to_owned(),
            images: request
                .images()
                .iter()
                .map(|image| (image.image_id(), image.media_type(), image.bytes().to_vec()))
                .collect(),
        });
        let cancelled = matches!(self.mode, TransportMode::CancelThenSuccess);
        let summary = if cancelled {
            CANCELLED_EVIDENCE_SENTINEL
        } else {
            "A green status badge is visible."
        };
        let results = request
            .images()
            .iter()
            .map(|image| {
                VisionImageResult::new(
                    image.image_id(),
                    VisionImageOutcome::Ok {
                        summary: summary.to_owned(),
                        visible_text: vec!["Ready".to_owned()],
                        details: vec!["The badge is green.".to_owned()],
                    },
                )
                .unwrap()
            })
            .collect();
        let response = VisionBatchResponse::new(results);
        Box::pin(async move {
            if cancelled {
                assert!(cancellation.cancel());
            }
            response
        })
    }
}

fn target() -> NetworkTarget {
    NetworkTarget {
        scheme: "https".to_owned(),
        host: "ai-gateway.vercel.sh".to_owned(),
        port: None,
    }
}

fn png_fixture() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(IMAGE_BYTE_SENTINEL.as_bytes());
    bytes
}

fn provider(arguments: Value, name: &str) -> ScriptedModelProvider {
    let call = ToolCall {
        id: ToolCallId::new("vision-call").unwrap(),
        name: ToolName::new(VISION_TOOL_NAME).unwrap(),
        arguments,
    };
    ScriptedModelProvider::new(
        name,
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall { call },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]),
        ],
    )
}

fn tool(root: &TemporaryDirectory, transport: FakeTransport) -> VisionTool {
    VisionTool::with_transport(
        root.path(),
        target(),
        Arc::new(transport),
        Arc::new(NeverDeadline),
    )
    .unwrap()
}

fn collect(engine: &Engine, name: &str) -> (SessionId, Vec<EngineEvent>) {
    let session_id = SessionId::new(name).unwrap();
    let session = engine
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new(format!("incarnation-{name}")).unwrap(),
        )
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("inspect the requested image")
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    (session_id, events)
}

fn second_request_tool_output(provider: &ScriptedModelProvider) -> (Message, ToolOutput) {
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.tools.len(), 1);
    assert_eq!(requests[0].request.tools[0].name.as_str(), VISION_TOOL_NAME);
    let message = requests[1].request.messages[2].clone();
    assert_eq!(message.role, Role::Tool);
    let ContentBlock::ToolResult { output, .. } = &message.content[0] else {
        panic!("expected a durable tool result")
    };
    (message.clone(), output.clone())
}

fn assert_completed(events: &[EngineEvent], reason: &StopReason) {
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: actual,
            ..
        }) if actual == reason
    ));
}

fn assert_exact_capability(policy: &ScriptedPermissionHandler) {
    let requests = policy.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].capability,
        Capability::Vision {
            paths: vec![IMAGE_PATH.to_owned()],
            target: target(),
        }
    );
}

#[test]
fn path_denial_performs_no_tool_or_transport_effect_and_leaks_no_image_content() {
    let workspace = TemporaryDirectory::new();
    fs::create_dir(workspace.path().join("screenshots")).unwrap();
    let image = png_fixture();
    fs::write(workspace.path().join(IMAGE_PATH), &image).unwrap();
    let provider = provider(
        json!({"focus": FOCUS, "paths": [IMAGE_PATH]}),
        "vision-engine-denied",
    );
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Deny {
            reason: "denied by test policy".to_owned(),
        })]);
    let transport = FakeTransport::new(TransportMode::Success);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(tool(&workspace, transport.clone()))
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "vision-engine-denied");

    assert_completed(&events, &StopReason::Completed);
    assert_exact_capability(&policy);
    assert_eq!(transport.calls(), 0);
    assert!(transport.requests().is_empty());
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::ToolStarted { .. } | TurnEvent::ToolFinished { .. }
    )));
    let (message, output) = second_request_tool_output(&provider);
    assert_eq!(
        output,
        ToolOutput {
            content: json!({
                "code": "permission_denied",
                "message": "tool execution was denied by policy",
            }),
            is_error: true,
        }
    );
    let outer_request = serde_json::to_string(&provider.requests()[1].request).unwrap();
    assert!(!outer_request.contains(IMAGE_BYTE_SENTINEL));
    assert!(!outer_request.contains(PNG_BASE64_PREFIX));
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
    assert_eq!(fs::read(workspace.path().join(IMAGE_PATH)).unwrap(), image);
}

#[test]
fn approval_precedes_one_bounded_request_and_persists_only_structured_text_evidence() {
    let workspace = TemporaryDirectory::new();
    fs::create_dir(workspace.path().join("screenshots")).unwrap();
    let image = png_fixture();
    fs::write(workspace.path().join(IMAGE_PATH), &image).unwrap();
    let provider = provider(
        json!({"focus": FOCUS, "paths": [IMAGE_PATH]}),
        "vision-engine-allowed",
    );
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let transport = FakeTransport::new(TransportMode::Success);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(tool(&workspace, transport.clone()))
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "vision-engine-allowed");

    assert_completed(&events, &StopReason::Completed);
    assert_exact_capability(&policy);
    let resolved = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::PermissionResolved { .. }))
        .unwrap();
    let started = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
        .unwrap();
    let finished = events
        .iter()
        .position(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
        .unwrap();
    assert!(resolved < started && started < finished);
    assert_eq!(transport.calls(), 1);
    assert_eq!(
        transport.requests(),
        [RequestRecord {
            session_id: "vision-engine-allowed".to_owned(),
            focus: FOCUS.to_owned(),
            images: vec![(1, VisionMediaType::Png, image)],
        }]
    );
    let request = &transport.requests()[0];
    assert!(request.images.len() <= MAX_VISION_BATCH_IMAGES);
    assert!(
        request
            .images
            .iter()
            .map(|(_, _, bytes)| bytes.len())
            .sum::<usize>()
            <= MAX_VISION_BATCH_BYTES
    );

    let (message, output) = second_request_tool_output(&provider);
    let expected = ToolOutput::success(json!({
        "images": [{
            "image_id": 1,
            "status": "ok",
            "summary": "A green status badge is visible.",
            "visible_text": ["Ready"],
            "details": ["The badge is green."]
        }]
    }));
    assert_eq!(output, expected);
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);

    let tool_result = serde_json::to_string(&output).unwrap();
    assert!(tool_result.contains("A green status badge is visible."));
    assert!(!tool_result.contains(IMAGE_PATH));
    assert!(!tool_result.contains(IMAGE_BYTE_SENTINEL));
    assert!(!tool_result.contains(PNG_BASE64_PREFIX));
    let outer_request = serde_json::to_string(&provider.requests()[1].request).unwrap();
    assert!(!outer_request.contains(IMAGE_BYTE_SENTINEL));
    assert!(!outer_request.contains(PNG_BASE64_PREFIX));
}

#[test]
fn attachment_ids_need_no_permission_or_transport_and_return_total_failure() {
    let workspace = TemporaryDirectory::new();
    let provider = provider(
        json!({"focus": "Compare the attachments.", "image_ids": [9, 3]}),
        "vision-engine-attachments",
    );
    let store = InMemorySessionStore::new();
    let policy = ScriptedPermissionHandler::new([]);
    let transport = FakeTransport::new(TransportMode::Success);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(tool(&workspace, transport.clone()))
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "vision-engine-attachments");

    assert_completed(&events, &StopReason::Completed);
    assert!(policy.requests().is_empty());
    assert_eq!(transport.calls(), 0);
    assert!(events.iter().all(|event| !matches!(
        event.payload,
        TurnEvent::PermissionRequested { .. } | TurnEvent::PermissionResolved { .. }
    )));
    let (message, output) = second_request_tool_output(&provider);
    assert!(output.is_error);
    assert_eq!(output.content["images"][0]["image_id"], 9);
    assert_eq!(output.content["images"][1]["image_id"], 3);
    assert_eq!(
        output.content["images"][0]["error"]["code"],
        "image_unavailable"
    );
    assert_eq!(
        output.content["images"][1]["error"]["code"],
        "image_unavailable"
    );
    assert_eq!(store.record(&session_id).unwrap().messages[2], message);
}

#[test]
fn same_poll_cancellation_keeps_unknown_placeholder_without_partial_visual_evidence() {
    let workspace = TemporaryDirectory::new();
    fs::create_dir(workspace.path().join("screenshots")).unwrap();
    fs::write(workspace.path().join(IMAGE_PATH), png_fixture()).unwrap();
    let provider = provider(
        json!({"focus": FOCUS, "paths": [IMAGE_PATH]}),
        "vision-engine-cancelled",
    );
    let store = InMemorySessionStore::new();
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Once,
        })]);
    let transport = FakeTransport::new(TransportMode::CancelThenSuccess);
    let engine = Engine::builder()
        .provider(provider.clone())
        .session_store(store.clone())
        .permission_handler(policy.clone())
        .tool(tool(&workspace, transport.clone()))
        .build()
        .unwrap();

    let (session_id, events) = collect(&engine, "vision-engine-cancelled");

    assert_completed(&events, &StopReason::Cancelled);
    assert_exact_capability(&policy);
    assert_eq!(transport.calls(), 1);
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, TurnEvent::ToolStarted { .. }))
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.payload, TurnEvent::ToolFinished { .. }))
    );
    assert_eq!(provider.requests().len(), 1);

    let record = store.record(&session_id).unwrap();
    assert_eq!(record.messages.len(), 3);
    assert_eq!(record.messages[2].role, Role::Tool);
    let ContentBlock::ToolResult { call_id, output } = &record.messages[2].content[0] else {
        panic!("expected durable unknown tool result placeholder")
    };
    assert_eq!(call_id.as_str(), "vision-call");
    assert_eq!(
        output,
        &ToolOutput {
            content: json!({
                "code": "tool_result_unknown",
                "message": "tool result status is unknown",
            }),
            is_error: true,
        }
    );
    let durable_record = serde_json::to_string(&record).unwrap();
    assert!(!durable_record.contains(CANCELLED_EVIDENCE_SENTINEL));
    assert!(!durable_record.contains(IMAGE_BYTE_SENTINEL));
    assert!(!durable_record.contains(PNG_BASE64_PREFIX));
}
