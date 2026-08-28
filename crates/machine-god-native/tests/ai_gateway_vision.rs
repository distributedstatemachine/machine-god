#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures_util::{Stream, stream};
use machine_god_core::{BoxFuture, CancellationToken, ProviderError, ProviderErrorKind, SessionId};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::{
    NetworkTarget, SessionIncarnationId, Tool, ToolCallId, ToolContext, TurnId,
};
use machine_god_native::{
    AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION, AI_GATEWAY_PROTOCOL_VERSION,
    AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest,
    AiGatewayVisionConfigErrorKind, AiGatewayVisionTransport, MAX_VISION_ATTEMPT_EVIDENCE_BYTES,
    MAX_VISION_BATCH_RAW_BYTES, MAX_VISION_EVIDENCE_LIST_ITEMS, MAX_VISION_EVIDENCE_STRING_BYTES,
    MAX_VISION_FOCUS_BYTES, MAX_VISION_REQUEST_BYTES, MAX_VISION_RESPONSE_BYTES,
    MAX_VISION_RESPONSE_JSON_NODES, VisionBatchRequest, VisionImage, VisionImageOutcome,
    VisionMediaType, VisionTransport, VisionTransportErrorKind,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_native::{VisionDeadline, VisionTool, VisionTransportError};
use serde_json::{Value, json};
use std::future::Future;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::{Path, PathBuf};
use std::pin::Pin;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::Instant;

const MODEL: &str = "test/configured-vision-model";
const PRIVATE_FOCUS: &str = "inspect PRIVATE_FOCUS_SENTINEL";
#[cfg(any(target_os = "linux", target_os = "macos"))]
static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
struct CapturedRequest {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    body_capacity: usize,
}

#[derive(Clone)]
struct ScriptedTransport {
    scripts: Arc<Vec<Vec<Vec<u8>>>>,
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

impl ScriptedTransport {
    fn new(scripts: Vec<Vec<Vec<u8>>>) -> Self {
        Self {
            scripts: Arc::new(scripts),
            calls: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn one(response: Vec<u8>) -> Self {
        Self::new(vec![vec![response]])
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl AiGatewayTransport for ScriptedTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        let (headers, body) = request.into_parts();
        let body_capacity = body.capacity();
        self.requests.lock().unwrap().push(CapturedRequest {
            headers: headers
                .into_iter()
                .map(machine_god_native::AiGatewayHeader::into_parts)
                .collect(),
            body,
            body_capacity,
        });
        let chunks = self.scripts[index].clone();
        Box::pin(async move {
            Ok(Box::pin(stream::iter(chunks.into_iter().map(Ok))) as AiGatewayByteStream)
        })
    }
}

#[derive(Clone)]
struct ErrorTransport {
    kind: ProviderErrorKind,
    calls: Arc<AtomicUsize>,
}

impl AiGatewayTransport for ErrorTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let kind = self.kind;
        Box::pin(async move {
            Err(ProviderError::new(
                kind,
                "PRIVATE_PROVIDER_CODE",
                "PRIVATE_PROVIDER_MESSAGE",
                false,
            ))
        })
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

struct CountingWake {
    wakes: Arc<AtomicUsize>,
}

impl Wake for CountingWake {
    fn wake(self: Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(&Waker::from(Arc::new(NoopWake))))
}

fn request(images: Vec<VisionImage>) -> VisionBatchRequest {
    request_with_focus(PRIVATE_FOCUS.to_owned(), images)
}

fn request_with_focus(focus: String, images: Vec<VisionImage>) -> VisionBatchRequest {
    VisionBatchRequest::new(
        SessionId::new("vision-gateway-session").unwrap(),
        focus,
        images,
    )
    .unwrap()
}

fn png(id: u64, bytes: &[u8]) -> VisionImage {
    VisionImage::new(id, VisionMediaType::Png, bytes.to_vec()).unwrap()
}

fn valid_result() -> Value {
    json!({
        "images": [{
            "image_id": 1,
            "status": "ok",
            "summary": "terminal screenshot",
            "visible_text": ["READY"],
            "details": ["green indicator"]
        }]
    })
}

fn sse_text(text: &str) -> Vec<u8> {
    sse_events(&[
        json!({"type": "stream-start", "warnings": []}),
        json!({"type": "text-start", "id": "vision-text"}),
        json!({"type": "text-delta", "id": "vision-text", "delta": text}),
        json!({"type": "text-end", "id": "vision-text"}),
        json!({"type": "finish", "finishReason": {"unified": "stop"}}),
    ])
}

fn sse_unframed_text(text: &str) -> Vec<u8> {
    sse_events(&[
        json!({"type": "text-delta", "id": "vision-text", "delta": text}),
        json!({"type": "finish", "finishReason": {"unified": "stop"}}),
    ])
}

fn pinned_usage() -> Value {
    json!({
        "inputTokens": {
            "total": 10,
            "noCache": 8,
            "cacheRead": 2,
            "cacheWrite": 0
        },
        "outputTokens": {
            "total": 5,
            "text": 5,
            "reasoning": 0
        },
        "raw": {"provider": {"promptTokens": 10}}
    })
}

fn sse_empty_text() -> Vec<u8> {
    sse_events(&[json!({
        "type": "finish",
        "finishReason": {"unified": "stop"}
    })])
}

fn sse_events(events: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend_from_slice(b"data: ");
        bytes.extend_from_slice(serde_json::to_string(event).unwrap().as_bytes());
        bytes.extend_from_slice(b"\n\n");
    }
    bytes.extend_from_slice(b"data: [DONE]\n\n");
    bytes
}

fn json_node_count(value: &Value) -> usize {
    let mut count = 0_usize;
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        count += 1;
        match value {
            Value::Array(values) => stack.extend(values),
            Value::Object(values) => stack.extend(values.values()),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    count
}

fn execute(
    transport: Arc<dyn AiGatewayTransport>,
    request: VisionBatchRequest,
) -> Result<machine_god_native::VisionBatchResponse, machine_god_native::VisionTransportError> {
    let worker = AiGatewayVisionTransport::new(MODEL, transport).unwrap();
    futures_executor::block_on(worker.analyze(request, CancellationToken::new()))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct TemporaryDirectory(PathBuf);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "machine-god-ai-gateway-vision-{}-{identifier}",
                std::process::id()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory")
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if std::thread::panicking() => {
                eprintln!("failed to remove temporary directory while panicking: {error}");
            }
            Err(error) => panic!("failed to remove temporary directory: {error}"),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NeverDeadline;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl VisionDeadline for NeverDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>> {
        Box::pin(std::future::pending())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct TriggeredDeadline {
    ready: Arc<AtomicBool>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl VisionDeadline for TriggeredDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>> {
        let ready = Arc::clone(&self.ready);
        Box::pin(std::future::poll_fn(move |_context| {
            if ready.load(Ordering::SeqCst) {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        }))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn tool_context() -> ToolContext {
    ToolContext {
        session_id: SessionId::new("vision-gateway-session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("vision-gateway-incarnation").unwrap(),
        turn_id: TurnId::new("vision-gateway-turn").unwrap(),
        call_id: ToolCallId::new("vision-gateway-call").unwrap(),
    }
}

fn header<'a>(request: &'a CapturedRequest, name: &str) -> &'a str {
    request
        .headers
        .iter()
        .find_map(|(candidate, value)| (candidate == name).then_some(value.as_str()))
        .unwrap_or_else(|| panic!("missing header {name}"))
}

#[test]
fn exact_request_uses_configured_model_file_parts_and_strict_response_format() {
    let transport = ScriptedTransport::one(sse_text(&valid_result().to_string()));
    let response = execute(
        Arc::new(transport.clone()),
        request(vec![
            png(1, &[0x00]),
            VisionImage::new(2, VisionMediaType::Jpeg, vec![0x00, 0x01]).unwrap(),
            VisionImage::new(3, VisionMediaType::Gif, vec![0x00, 0x01, 0x02]).unwrap(),
            VisionImage::new(4, VisionMediaType::Webp, vec![0xff, 0xee, 0xdd, 0xcc]).unwrap(),
        ]),
    )
    .unwrap();
    assert_eq!(response.images().len(), 1);

    let captured = transport.requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(header(&captured[0], "ai-language-model-id"), MODEL);
    assert_eq!(
        header(&captured[0], "ai-gateway-protocol-version"),
        AI_GATEWAY_PROTOCOL_VERSION
    );
    assert_eq!(
        header(&captured[0], "ai-language-model-specification-version"),
        AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION
    );
    assert_eq!(
        header(&captured[0], "x-session-id"),
        "vision-gateway-session"
    );
    let body: Value = serde_json::from_slice(&captured[0].body).unwrap();
    assert_eq!(body["tools"], json!([]));
    assert_eq!(body["toolChoice"], json!({"type": "none"}));
    assert_eq!(body["prompt"][0]["role"], "system");
    assert!(
        body["prompt"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Never include filesystem paths")
    );
    let parts = body["prompt"][1]["content"].as_array().unwrap();
    assert_eq!(parts.len(), 5);
    assert_eq!(parts[0]["type"], "text");
    assert!(parts[0]["text"].as_str().unwrap().contains(PRIVATE_FOCUS));
    for (part, expected_media, expected_bytes) in [
        (&parts[1], "image/png", vec![0x00]),
        (&parts[2], "image/jpeg", vec![0x00, 0x01]),
        (&parts[3], "image/gif", vec![0x00, 0x01, 0x02]),
        (&parts[4], "image/webp", vec![0xff, 0xee, 0xdd, 0xcc]),
    ] {
        assert_eq!(part["type"], "file");
        assert_eq!(part["mediaType"], expected_media);
        assert_eq!(
            BASE64_STANDARD
                .decode(part["data"].as_str().unwrap())
                .unwrap(),
            expected_bytes
        );
    }
    assert_eq!(body["responseFormat"]["type"], "json");
    assert_eq!(body["responseFormat"]["name"], "fx_vision_evidence");
    assert_eq!(
        body["responseFormat"]["schema"]["properties"]["images"]["minItems"],
        4
    );
    assert_eq!(
        body["responseFormat"]["schema"]["properties"]["images"]["maxItems"],
        4
    );
    assert_eq!(
        body["responseFormat"]["schema"]["properties"]["images"]["items"]["anyOf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(body.get("maxOutputTokens").is_none());
}

#[test]
fn response_decoding_survives_every_byte_fragmentation() {
    let response = sse_text(&valid_result().to_string());
    let chunks = response.into_iter().map(|byte| vec![byte]).collect();
    let transport = ScriptedTransport::new(vec![chunks]);
    let result = execute(Arc::new(transport), request(vec![png(1, &[1])])).unwrap();

    let VisionImageOutcome::Ok {
        summary,
        visible_text,
        details,
    } = result.images()[0].outcome()
    else {
        panic!("expected successful evidence")
    };
    assert_eq!(summary, "terminal screenshot");
    assert_eq!(visible_text, &["READY"]);
    assert_eq!(details, &["green indicator"]);
}

#[test]
fn pinned_raw_v4_unframed_text_delta_finish_and_done_sequence_is_accepted() {
    let transport = ScriptedTransport::one(sse_unframed_text(&valid_result().to_string()));
    let result = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap();
    assert_eq!(result.images().len(), 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn pinned_raw_v4_usage_totals_and_exact_optional_counters_are_accepted() {
    let transport = ScriptedTransport::one(sse_events(&[
        json!({
            "type": "text-delta",
            "id": "vision-text",
            "delta": valid_result().to_string()
        }),
        json!({
            "type": "finish",
            "finishReason": {"unified": "stop", "raw": "provider-stop"},
            "usage": pinned_usage(),
            "providerMetadata": {"gateway": {"route": "direct"}}
        }),
    ]));
    let result = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap();
    assert_eq!(result.images().len(), 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn exact_raw_batch_and_hostile_focus_use_one_exact_body_allocation_below_the_ceiling() {
    let transport = ScriptedTransport::one(sse_text(&valid_result().to_string()));
    let bytes = vec![0xa5; MAX_VISION_BATCH_RAW_BYTES];
    let focus = "\u{0001}".repeat(MAX_VISION_FOCUS_BYTES);
    execute(
        Arc::new(transport.clone()),
        request_with_focus(
            focus.clone(),
            vec![VisionImage::new(1, VisionMediaType::Png, bytes).unwrap()],
        ),
    )
    .unwrap();
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body.len() <= MAX_VISION_REQUEST_BYTES);
    assert_eq!(requests[0].body_capacity, requests[0].body.len());
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(
        body["prompt"][1]["content"].as_array().unwrap()[0]["text"]
            .as_str()
            .unwrap()
            .contains(&focus)
    );
}

#[test]
fn base64_segment_boundaries_preserve_the_canonical_wire_body() {
    const SEGMENT_BYTES: usize = 48 * 1024;

    for length in [
        SEGMENT_BYTES - 1,
        SEGMENT_BYTES,
        SEGMENT_BYTES + 1,
        2 * SEGMENT_BYTES - 1,
        2 * SEGMENT_BYTES,
        2 * SEGMENT_BYTES + 1,
    ] {
        let bytes = (0..length)
            .map(|index| u8::try_from(index % 251).unwrap())
            .collect::<Vec<_>>();
        let transport = ScriptedTransport::one(sse_text(&valid_result().to_string()));
        execute(Arc::new(transport.clone()), request(vec![png(1, &bytes)])).unwrap();

        let captured = transport.requests();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].body_capacity, captured[0].body.len());
        let body: Value = serde_json::from_slice(&captured[0].body).unwrap();
        assert_eq!(
            body["prompt"][1]["content"][1]["data"],
            BASE64_STANDARD.encode(&bytes)
        );
    }
}

#[test]
fn maximal_request_encoding_yields_before_dispatch_and_drop_is_inert() {
    let transport = ScriptedTransport::one(sse_text(&valid_result().to_string()));
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let mut future = worker.analyze(
        request(vec![
            VisionImage::new(
                1,
                VisionMediaType::Png,
                vec![0xa5; MAX_VISION_BATCH_RAW_BYTES],
            )
            .unwrap(),
        ]),
        CancellationToken::new(),
    );
    let notification_count = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(CountingWake {
        wakes: Arc::clone(&notification_count),
    }));

    for expected_notifications in 1..=2 {
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert_eq!(
            notification_count.load(Ordering::SeqCst),
            expected_notifications
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }
    drop(future);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_at_completed_body_boundary_prevents_dispatch() {
    let transport = ScriptedTransport::one(sse_text(&valid_result().to_string()));
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let cancellation = CancellationToken::new();
    let mut future = worker.analyze(
        request(vec![png(1, b"PRIVATE_PREDISPATCH_IMAGE_SENTINEL")]),
        cancellation.clone(),
    );

    // The first boundary follows the bounded base64 segment. The second owns
    // the complete canonical body immediately before transport dispatch.
    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    cancellation.cancel();
    let Poll::Ready(Err(error)) = poll_once(future.as_mut()) else {
        panic!("pre-dispatch cancellation must complete")
    };
    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_PREDISPATCH_IMAGE_SENTINEL"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn outer_deadline_at_encoding_boundary_prevents_gateway_dispatch() {
    let workspace = TemporaryDirectory::new();
    std::fs::write(workspace.path().join("one.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    let transport = ScriptedTransport::one(sse_text(&valid_result().to_string()));
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let deadline_ready = Arc::new(AtomicBool::new(false));
    let tool = VisionTool::with_transport(
        workspace.path(),
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "ai-gateway.vercel.sh".to_owned(),
            port: None,
        },
        Arc::new(worker),
        Arc::new(TriggeredDeadline {
            ready: Arc::clone(&deadline_ready),
        }),
    )
    .unwrap();
    let mut future = tool.execute(
        tool_context(),
        json!({"focus": "Inspect", "paths": ["one.png"]}),
        CancellationToken::new(),
    );

    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    deadline_ready.store(true, Ordering::SeqCst);
    let Poll::Ready(Err(error)) = poll_once(future.as_mut()) else {
        panic!("outer deadline must complete before Gateway dispatch")
    };
    assert_eq!(error.code, "vision_timeout");
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn response_json_node_budget_is_aggregate_across_events_and_structured_evidence() {
    let result = valid_result();
    let result_text = result.to_string();
    let base_events = vec![
        json!({"type": "stream-start", "warnings": []}),
        json!({"type": "text-start", "id": "vision-text"}),
        json!({"type": "text-delta", "id": "vision-text", "delta": result_text}),
        json!({"type": "text-end", "id": "vision-text"}),
        json!({"type": "finish", "finishReason": {"unified": "stop"}}),
    ];
    let required_nodes =
        base_events.iter().map(json_node_count).sum::<usize>() + json_node_count(&result);
    let exact_padding = MAX_VISION_RESPONSE_JSON_NODES - required_nodes;

    let mut exact_events = base_events.clone();
    exact_events[0]["warnings"] = Value::Array(vec![json!(0); exact_padding]);
    let exact_transport = ScriptedTransport::one(sse_events(&exact_events));
    execute(
        Arc::new(exact_transport.clone()),
        request(vec![png(1, &[1])]),
    )
    .unwrap();
    assert_eq!(exact_transport.calls.load(Ordering::SeqCst), 1);

    let mut oversized_events = base_events;
    oversized_events[0]["warnings"] = Value::Array(vec![json!(0); exact_padding + 1]);
    let oversized = sse_events(&oversized_events);
    let oversized_transport = ScriptedTransport::one(oversized);
    let error = execute(
        Arc::new(oversized_transport.clone()),
        request(vec![png(1, &[1])]),
    )
    .unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::ResponseTooLarge);
    assert_eq!(oversized_transport.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn event_envelope_node_exhaustion_is_a_non_retryable_resource_error() {
    let oversized_event = json!({
        "type": "stream-start",
        "warnings": vec![json!(0); MAX_VISION_RESPONSE_JSON_NODES],
    });
    let transport = ScriptedTransport::one(sse_events(&[oversized_event]));
    let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::ResponseTooLarge);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);

    let malformed = ScriptedTransport::one(b"data: {\"type\":\n\n".to_vec());
    let error = execute(Arc::new(malformed.clone()), request(vec![png(1, &[1])])).unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::Protocol);
    assert_eq!(malformed.calls.load(Ordering::SeqCst), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn event_envelope_node_exhaustion_projects_to_one_output_limit_failure() {
    let workspace = TemporaryDirectory::new();
    std::fs::write(workspace.path().join("one.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    let oversized_event = json!({
        "type": "stream-start",
        "warnings": vec![json!(0); MAX_VISION_RESPONSE_JSON_NODES],
    });
    let transport = ScriptedTransport::one(sse_events(&[oversized_event]));
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let tool = VisionTool::with_transport(
        workspace.path(),
        NetworkTarget {
            scheme: "https".to_owned(),
            host: "ai-gateway.vercel.sh".to_owned(),
            port: None,
        },
        Arc::new(worker),
        Arc::new(NeverDeadline),
    )
    .unwrap();

    let output = futures_executor::block_on(tool.execute(
        tool_context(),
        json!({"focus": "Inspect", "paths": ["one.png"]}),
        CancellationToken::new(),
    ))
    .unwrap();

    assert!(output.is_error);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(output.content["images"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        output.content["images"][0]["error"]["code"],
        "output_limit_exceeded"
    );
}

#[test]
fn semantic_invalidity_retries_once_with_byte_identical_verified_images() {
    let invalid = sse_text(r#"{"images":[]}"#);
    let valid = sse_text(&valid_result().to_string());
    let transport = ScriptedTransport::new(vec![vec![invalid], vec![valid]]);

    execute(
        Arc::new(transport.clone()),
        request(vec![png(1, b"PRIVATE_IMAGE_BYTES_SENTINEL")]),
    )
    .unwrap();

    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    let requests = transport.requests();
    assert_eq!(requests[0].body, requests[1].body);
}

#[derive(Clone)]
struct RetryOwnershipTransport {
    calls: Arc<AtomicUsize>,
    live_attempts: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct RetryOwnershipStream {
    response: Option<Vec<u8>>,
    _request_body: Vec<u8>,
    live_attempts: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl Stream for RetryOwnershipStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.response
            .take()
            .map_or(Poll::Pending, |response| Poll::Ready(Some(Ok(response))))
    }
}

impl Drop for RetryOwnershipStream {
    fn drop(&mut self) {
        assert_eq!(self.live_attempts.fetch_sub(1, Ordering::SeqCst), 1);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for RetryOwnershipTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            self.live_attempts.fetch_add(1, Ordering::SeqCst),
            0,
            "prior attempt-owned request body and stream must be gone"
        );
        let response = if call_index == 0 {
            sse_text(r#"{"images":[]}"#)
        } else {
            sse_text(&valid_result().to_string())
        };
        let stream = RetryOwnershipStream {
            response: Some(response),
            _request_body: request.into_body(),
            live_attempts: Arc::clone(&self.live_attempts),
            drops: Arc::clone(&self.drops),
        };
        Box::pin(async move { Ok(Box::pin(stream) as AiGatewayByteStream) })
    }
}

#[test]
fn retry_drops_first_stream_and_its_owned_body_before_request_two() {
    let transport = RetryOwnershipTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        live_attempts: Arc::new(AtomicUsize::new(0)),
        drops: Arc::new(AtomicUsize::new(0)),
    };

    let result = execute(
        Arc::new(transport.clone()),
        request(vec![png(1, b"PRIVATE_RETRY_OWNERSHIP_IMAGE_SENTINEL")]),
    )
    .unwrap();
    assert_eq!(result.images().len(), 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transport.live_attempts.load(Ordering::SeqCst), 0);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 2);
}

#[test]
fn semantic_retry_returns_pending_before_request_two_and_drop_stays_inert() {
    let invalid = sse_text(r#"{"images":[]}"#);
    let valid = sse_text(&valid_result().to_string());
    let transport = ScriptedTransport::new(vec![vec![invalid], vec![valid]]);
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let mut future = worker.analyze(
        request(vec![png(1, b"PRIVATE_RETRY_BOUNDARY_IMAGE_SENTINEL")]),
        CancellationToken::new(),
    );
    let notification_count = Arc::new(AtomicUsize::new(0));
    let retry_waker = Waker::from(Arc::new(CountingWake {
        wakes: Arc::clone(&notification_count),
    }));

    for expected_notifications in 1..=3 {
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&retry_waker))
                .is_pending()
        );
        assert_eq!(
            notification_count.load(Ordering::SeqCst),
            expected_notifications
        );
    }
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    drop(future);
    assert_eq!(notification_count.load(Ordering::SeqCst), 3);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_wins_at_semantic_retry_boundary_without_request_two() {
    let invalid = sse_text(r#"{"images":[]}"#);
    let valid = sse_text(&valid_result().to_string());
    let transport = ScriptedTransport::new(vec![vec![invalid], vec![valid]]);
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let cancellation = CancellationToken::new();
    let mut future = worker.analyze(
        request(vec![png(1, b"PRIVATE_RETRY_CANCELLATION_IMAGE_SENTINEL")]),
        cancellation.clone(),
    );

    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    cancellation.cancel();
    let Poll::Ready(Err(error)) = poll_once(future.as_mut()) else {
        panic!("retry-boundary cancellation must complete")
    };
    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_RETRY_CANCELLATION_IMAGE_SENTINEL"));
}

#[test]
fn second_semantic_invalidity_returns_stable_error_after_exactly_two_attempts() {
    let invalid = sse_text(r#"{"images":[]}"#);
    let transport = ScriptedTransport::new(vec![vec![invalid.clone()], vec![invalid]]);
    let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::InvalidResponse);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn empty_success_output_retries_once_then_accepts_valid_structured_evidence() {
    let empty = sse_empty_text();
    let valid = sse_unframed_text(&valid_result().to_string());
    let transport = ScriptedTransport::new(vec![vec![empty], vec![valid]]);

    let result = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap();
    assert_eq!(result.images().len(), 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn two_empty_success_outputs_exhaust_the_single_semantic_retry() {
    let empty = sse_empty_text();
    let transport = ScriptedTransport::new(vec![vec![empty.clone()], vec![empty]]);

    let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::InvalidResponse);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn json_fence_trimming_accepts_only_pinned_ascii_edges() {
    let result = valid_result().to_string();
    let ascii_wrapped = format!(" \t\r\n```json\r\n \t\r\n{result}\r\n\t ``` \t\r\n");
    let transport = ScriptedTransport::one(sse_unframed_text(&ascii_wrapped));
    let response = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap();
    assert_eq!(response.images().len(), 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);

    for wrapper in ["\u{00a0}", "\u{000b}", "\u{000c}"] {
        for invalid in [
            format!("{wrapper}```json\n{result}\n```{wrapper}"),
            format!("```json\n{wrapper}{result}{wrapper}\n```"),
        ] {
            let invalid_response = sse_unframed_text(&invalid);
            let valid_response = sse_unframed_text(&ascii_wrapped);
            let recovered =
                ScriptedTransport::new(vec![vec![invalid_response.clone()], vec![valid_response]]);
            let response =
                execute(Arc::new(recovered.clone()), request(vec![png(1, &[1])])).unwrap();
            assert_eq!(response.images().len(), 1);
            assert_eq!(recovered.calls.load(Ordering::SeqCst), 2);

            let exhausted = ScriptedTransport::new(vec![
                vec![invalid_response.clone()],
                vec![invalid_response],
            ]);
            let error =
                execute(Arc::new(exhausted.clone()), request(vec![png(1, &[1])])).unwrap_err();
            assert_eq!(error.kind(), VisionTransportErrorKind::InvalidResponse);
            assert_eq!(exhausted.calls.load(Ordering::SeqCst), 2);
        }
    }
}

#[test]
fn summary_blankness_uses_exact_pinned_edges_and_preserves_other_content() {
    let ascii_blank = json!({"images": [{
        "image_id": 1,
        "status": "ok",
        "summary": " \t\r\n",
        "visible_text": [],
        "details": []
    }]});
    let blank_response = sse_unframed_text(&ascii_blank.to_string());
    let blank_transport =
        ScriptedTransport::new(vec![vec![blank_response.clone()], vec![blank_response]]);
    let error = execute(
        Arc::new(blank_transport.clone()),
        request(vec![png(1, &[1])]),
    )
    .unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::InvalidResponse);
    assert_eq!(blank_transport.calls.load(Ordering::SeqCst), 2);

    let nbsp = json!({"images": [{
        "image_id": 1,
        "status": "ok",
        "summary": "\u{00a0}",
        "visible_text": [],
        "details": []
    }]});
    let nbsp_response = sse_unframed_text(&nbsp.to_string());
    let nbsp_transport =
        ScriptedTransport::new(vec![vec![nbsp_response.clone()], vec![nbsp_response]]);
    let result = execute(
        Arc::new(nbsp_transport.clone()),
        request(vec![png(1, &[1])]),
    )
    .unwrap();
    let VisionImageOutcome::Ok { summary, .. } = result.images()[0].outcome() else {
        panic!("expected successful evidence")
    };
    assert_eq!(summary, "\u{00a0}");
    assert_eq!(nbsp_transport.calls.load(Ordering::SeqCst), 1);

    for accepted in ["\u{000b}", "\u{000c}"] {
        let evidence = json!({"images": [{
            "image_id": 1,
            "status": "ok",
            "summary": accepted,
            "visible_text": [],
            "details": []
        }]});
        let transport = ScriptedTransport::one(sse_unframed_text(&evidence.to_string()));
        let result = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap();
        let VisionImageOutcome::Ok { summary, .. } = result.images()[0].outcome() else {
            panic!("expected successful evidence")
        };
        assert_eq!(summary, accepted);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn structured_evidence_resource_ceilings_never_semantically_retry() {
    let oversized = [
        json!({"images": [{
            "image_id": 1,
            "status": "ok",
            "summary": "s".repeat(MAX_VISION_EVIDENCE_STRING_BYTES + 1),
            "visible_text": [],
            "details": []
        }]}),
        json!({"images": [{
            "image_id": 1,
            "status": "ok",
            "summary": "ok",
            "visible_text": vec![""; MAX_VISION_EVIDENCE_LIST_ITEMS + 1],
            "details": []
        }]}),
        json!({"images": [{
            "image_id": 1,
            "status": "ok",
            "summary": "s".repeat(7_000),
            "visible_text": ["v".repeat(7_000)],
            "details": ["d".repeat(7_000)]
        }]}),
    ];

    for result in oversized {
        let transport = ScriptedTransport::one(sse_unframed_text(&result.to_string()));
        let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
        assert_eq!(error.kind(), VisionTransportErrorKind::ResponseTooLarge);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn extra_structured_records_receive_only_the_bounded_semantic_retry() {
    let extra = json!({"images": [
        {
            "image_id": 1,
            "status": "ok",
            "summary": "requested",
            "visible_text": [],
            "details": []
        },
        {
            "image_id": 2,
            "status": "ok",
            "summary": "extra",
            "visible_text": [],
            "details": []
        }
    ]});
    let extra_response = sse_unframed_text(&extra.to_string());
    let valid_response = sse_unframed_text(&valid_result().to_string());
    let recovered =
        ScriptedTransport::new(vec![vec![extra_response.clone()], vec![valid_response]]);
    let response = execute(Arc::new(recovered.clone()), request(vec![png(1, &[1])])).unwrap();
    assert_eq!(response.images().len(), 1);
    assert_eq!(recovered.calls.load(Ordering::SeqCst), 2);

    let exhausted =
        ScriptedTransport::new(vec![vec![extra_response.clone()], vec![extra_response]]);
    let error = execute(Arc::new(exhausted.clone()), request(vec![png(1, &[1])])).unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::InvalidResponse);
    assert_eq!(exhausted.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn provider_outages_protocol_failures_and_output_limits_never_semantically_retry() {
    for (provider_kind, expected) in [
        (
            ProviderErrorKind::Unavailable,
            VisionTransportErrorKind::Unavailable,
        ),
        (
            ProviderErrorKind::InvalidRequest,
            VisionTransportErrorKind::InvalidRequest,
        ),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let outage = ErrorTransport {
            kind: provider_kind,
            calls: Arc::clone(&calls),
        };
        let error = execute(Arc::new(outage), request(vec![png(1, &[1])])).unwrap_err();
        assert_eq!(error.kind(), expected);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("PRIVATE_PROVIDER"));
    }

    for (response, expected) in [
        (
            sse_events(&[
                json!({"type": "text-start", "id": "vision-text"}),
                json!({"type": "file", "data": "PRIVATE_FILE_OUTPUT"}),
            ]),
            VisionTransportErrorKind::Protocol,
        ),
        (
            sse_events(&[
                json!({"type": "text-start", "id": "vision-text"}),
                json!({
                    "type": "tool-call",
                    "toolCallId": "private-call",
                    "toolName": "private-tool",
                    "input": {}
                }),
            ]),
            VisionTransportErrorKind::Protocol,
        ),
        (
            sse_events(&[
                json!({"type": "text-start", "id": "vision-text"}),
                json!({
                    "type": "text-delta",
                    "id": "vision-text",
                    "delta": "x".repeat(MAX_VISION_ATTEMPT_EVIDENCE_BYTES + 1)
                }),
            ]),
            VisionTransportErrorKind::ResponseTooLarge,
        ),
        (
            vec![b'x'; MAX_VISION_RESPONSE_BYTES + 1],
            VisionTransportErrorKind::ResponseTooLarge,
        ),
        (
            sse_events(&[
                json!({"type": "stream-start", "warnings": []}),
                json!({"type": "text-start", "id": "vision-text"}),
                json!({
                    "type": "text-delta",
                    "id": "vision-text",
                    "delta": valid_result().to_string()
                }),
                json!({"type": "text-end", "id": "vision-text"}),
                json!({"type": "finish", "finishReason": {"unified": "length"}}),
            ]),
            VisionTransportErrorKind::ResponseTooLarge,
        ),
    ] {
        let transport = ScriptedTransport::one(response);
        let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
        assert_eq!(error.kind(), expected);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn optional_text_lifecycle_remains_strict_when_start_or_end_is_present() {
    let cases = [
        vec![
            json!({"type": "text-delta", "id": "vision-text", "delta": "{}"}),
            json!({"type": "text-start", "id": "vision-text"}),
        ],
        vec![
            json!({"type": "text-end", "id": "vision-text"}),
            json!({"type": "finish", "finishReason": {"unified": "stop"}}),
        ],
        vec![
            json!({"type": "text-start", "id": "vision-text"}),
            json!({"type": "text-delta", "id": "vision-text", "delta": "{}"}),
            json!({"type": "finish", "finishReason": {"unified": "stop"}}),
        ],
        vec![
            json!({"type": "text-delta", "id": "vision-text", "delta": "{}"}),
            json!({"type": "text-end", "id": "vision-text"}),
        ],
        vec![
            json!({"type": "text-start", "id": "vision-text"}),
            json!({"type": "text-delta", "id": "vision-text", "delta": "{}"}),
            json!({"type": "text-end", "id": "vision-text"}),
            json!({"type": "finish", "finishReason": {"unified": "tool-calls"}}),
        ],
    ];
    for events in cases {
        let transport = ScriptedTransport::one(sse_events(&events));
        let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
        assert_eq!(error.kind(), VisionTransportErrorKind::Protocol);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn framed_and_unframed_text_deltas_require_one_consistent_identity() {
    let malformed = [
        // A framed delta must carry the ID established by `text-start`.
        vec![
            json!({"type": "text-start", "id": "vision-text"}),
            json!({"type": "text-delta", "delta": valid_result().to_string()}),
            json!({"type": "text-end", "id": "vision-text"}),
            json!({"type": "finish", "finishReason": {"unified": "stop"}}),
        ],
        vec![
            json!({"type": "text-start", "id": "vision-text"}),
            json!({
                "type": "text-delta",
                "id": "different-text",
                "delta": valid_result().to_string()
            }),
            json!({"type": "text-end", "id": "vision-text"}),
            json!({"type": "finish", "finishReason": {"unified": "stop"}}),
        ],
        // The pinned unframed path is delta-only, not identity-free: its
        // first delta establishes the canonical ID for all later deltas.
        vec![
            json!({"type": "text-delta", "delta": valid_result().to_string()}),
            json!({"type": "finish", "finishReason": {"unified": "stop"}}),
        ],
        vec![
            json!({"type": "text-delta", "id": "vision-text", "delta": ""}),
            json!({
                "type": "text-delta",
                "id": "different-text",
                "delta": valid_result().to_string()
            }),
            json!({"type": "finish", "finishReason": {"unified": "stop"}}),
        ],
    ];

    for events in malformed {
        let invalid = sse_events(&events);
        let would_be_retry = sse_unframed_text(&valid_result().to_string());
        let transport = ScriptedTransport::new(vec![vec![invalid], vec![would_be_retry]]);
        let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
        assert_eq!(error.kind(), VisionTransportErrorKind::Protocol);
        // Event identity is an envelope invariant, not structured semantic
        // invalidity, so it never consumes the one semantic retry.
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn usage_requires_unsigned_bounded_totals_and_exact_optional_counters() {
    let malformed_usage = [
        json!({"inputTokens": {}, "outputTokens": {"total": 1}}),
        json!({"inputTokens": {"total": 1}, "outputTokens": {}}),
        json!({
            "inputTokens": {"cacheRead": 1},
            "outputTokens": {"total": 1}
        }),
        json!({
            "inputTokens": {"total": 1},
            "outputTokens": {"reasoning": 1}
        }),
        json!({
            "inputTokens": {"total": -1},
            "outputTokens": {"total": 1}
        }),
        json!({
            "inputTokens": {"total": 1},
            "outputTokens": {"total": 1.5}
        }),
        json!({
            "inputTokens": {"total": 1, "unknown": 0},
            "outputTokens": {"total": 1}
        }),
    ];

    for usage in malformed_usage {
        assert_usage_is_non_retryable_protocol_error(&usage);
    }
}

#[test]
fn every_usage_breakdown_counter_must_be_unsigned_and_at_most_its_total() {
    for (group, counter) in [
        ("inputTokens", "noCache"),
        ("inputTokens", "cacheRead"),
        ("inputTokens", "cacheWrite"),
        ("outputTokens", "text"),
        ("outputTokens", "reasoning"),
    ] {
        for invalid_counter in [json!(-1), json!(1.5), json!(2)] {
            let mut usage = json!({
                "inputTokens": {"total": 1},
                "outputTokens": {"total": 1}
            });
            usage[group][counter] = invalid_counter;
            assert_usage_is_non_retryable_protocol_error(&usage);
        }
    }
}

fn assert_usage_is_non_retryable_protocol_error(usage: &Value) {
    let invalid = sse_events(&[
        json!({
            "type": "text-delta",
            "id": "vision-text",
            "delta": valid_result().to_string()
        }),
        json!({
            "type": "finish",
            "finishReason": {"unified": "stop"},
            "usage": usage
        }),
    ]);
    let would_be_retry = sse_unframed_text(&valid_result().to_string());
    let transport = ScriptedTransport::new(vec![vec![invalid], vec![would_be_retry]]);
    let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::Protocol);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn usage_breakdown_counters_accept_zero_equality_and_unsigned_max_boundaries() {
    let valid_usage = [
        json!({
            "inputTokens": {
                "total": 0,
                "noCache": 0,
                "cacheRead": 0,
                "cacheWrite": 0
            },
            "outputTokens": {"total": 0, "text": 0, "reasoning": 0}
        }),
        json!({
            "inputTokens": {
                "total": u64::MAX,
                "cacheRead": u64::MAX,
                "cacheWrite": u64::MAX
            },
            "outputTokens": {"total": u64::MAX, "reasoning": u64::MAX}
        }),
    ];

    for usage in valid_usage {
        let transport = ScriptedTransport::one(sse_events(&[
            json!({
                "type": "text-delta",
                "id": "vision-text",
                "delta": valid_result().to_string()
            }),
            json!({
                "type": "finish",
                "finishReason": {"unified": "stop"},
                "usage": usage
            }),
        ]));
        let result = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap();
        assert_eq!(result.images().len(), 1);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn structured_result_rejects_extra_duplicate_or_unauthorized_records_then_retries_once() {
    let invalid_results = [
        json!({"images": [{
            "image_id": 1, "status": "ok", "summary": "ok",
            "visible_text": [], "details": [], "path": "/private/path"
        }]}),
        json!({"images": [
            {"image_id": 1, "status": "ok", "summary": "a", "visible_text": [], "details": []},
            {"image_id": 1, "status": "ok", "summary": "b", "visible_text": [], "details": []}
        ]}),
        json!({"images": [{
            "image_id": 99, "status": "ok", "summary": "ok",
            "visible_text": [], "details": []
        }]}),
        json!({"images": [{
            "image_id": 1, "status": "failed", "error": "provider_response_invalid"
        }]}),
    ];
    for invalid in invalid_results {
        let response = sse_text(&invalid.to_string());
        let transport = ScriptedTransport::new(vec![vec![response.clone()], vec![response]]);
        let error = execute(
            Arc::new(transport.clone()),
            request(vec![png(1, &[1]), png(2, &[2])]),
        )
        .unwrap_err();
        assert_eq!(error.kind(), VisionTransportErrorKind::InvalidResponse);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("/private/path"));
    }
}

#[test]
fn provider_failure_record_is_typed_and_missing_sibling_is_preserved_for_tool_merge() {
    let result = json!({"images": [{
        "image_id": 2,
        "status": "failed",
        "error": "vision_unavailable"
    }]});
    let response = execute(
        Arc::new(ScriptedTransport::one(sse_text(&result.to_string()))),
        request(vec![png(1, &[1]), png(2, &[2])]),
    )
    .unwrap();
    assert_eq!(response.images().len(), 1);
    assert_eq!(response.images()[0].image_id(), 2);
    let VisionImageOutcome::Failed { error } = response.images()[0].outcome() else {
        panic!("expected provider failure")
    };
    assert_eq!(error.code().as_str(), "vision_unavailable");
}

#[derive(Clone)]
struct ReadyByteTransport {
    response: Arc<Vec<u8>>,
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct ReadyByteStream {
    response: Arc<Vec<u8>>,
    offset: usize,
    drops: Arc<AtomicUsize>,
}

impl Stream for ReadyByteStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(byte) = self.response.get(self.offset).copied() else {
            return Poll::Ready(None);
        };
        self.offset += 1;
        Poll::Ready(Some(Ok(vec![byte])))
    }
}

impl Drop for ReadyByteStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for ReadyByteTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = Arc::clone(&self.response);
        let drops = Arc::clone(&self.drops);
        Box::pin(async move {
            Ok(Box::pin(ReadyByteStream {
                response,
                offset: 0,
                drops,
            }) as AiGatewayByteStream)
        })
    }
}

fn ready_byte_transport() -> ReadyByteTransport {
    ReadyByteTransport {
        response: Arc::new(sse_text(&valid_result().to_string())),
        calls: Arc::new(AtomicUsize::new(0)),
        drops: Arc::new(AtomicUsize::new(0)),
    }
}

#[test]
fn immediately_ready_byte_stream_yields_and_eventually_preserves_exact_semantics() {
    let transport = ready_byte_transport();
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let mut future = worker.analyze(request(vec![png(1, &[1])]), CancellationToken::new());
    let notification_count = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(CountingWake {
        wakes: Arc::clone(&notification_count),
    }));

    // Encoding and pre-dispatch each return control once. The next outer poll
    // consumes exactly one accepted nonterminal item, not the complete response.
    for expected_notifications in 1..=3 {
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert_eq!(
            notification_count.load(Ordering::SeqCst),
            expected_notifications
        );
    }
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 0);

    let result = futures_executor::block_on(future).unwrap();
    assert_eq!(result.images().len(), 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_at_ready_item_boundary_releases_the_owned_stream() {
    let transport = ready_byte_transport();
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let mut future = worker.analyze(
        request(vec![png(1, b"PRIVATE_READY_BOUNDARY_IMAGE_SENTINEL")]),
        CancellationToken::new(),
    );

    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 0);
    drop(future);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_at_ready_item_boundary_wins_after_stream_cleanup() {
    let transport = ready_byte_transport();
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let cancellation = CancellationToken::new();
    let mut future = worker.analyze(
        request(vec![png(
            1,
            b"PRIVATE_READY_BOUNDARY_CANCELLATION_IMAGE_SENTINEL",
        )]),
        cancellation.clone(),
    );

    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 0);
    cancellation.cancel();
    let Poll::Ready(Err(error)) = poll_once(future.as_mut()) else {
        panic!("ready-item-boundary cancellation must complete")
    };
    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 1);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_READY_BOUNDARY_CANCELLATION_IMAGE_SENTINEL"));
}

#[derive(Clone)]
struct PendingSourceTransport {
    calls: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct PendingSourceStream {
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl Stream for PendingSourceStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingSourceStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for PendingSourceTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let polls = Arc::clone(&self.polls);
        let drops = Arc::clone(&self.drops);
        Box::pin(async move {
            Ok(Box::pin(PendingSourceStream { polls, drops }) as AiGatewayByteStream)
        })
    }
}

#[test]
fn cancellation_wakes_a_pending_source_and_wins_after_stream_cleanup() {
    let transport = PendingSourceTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        polls: Arc::new(AtomicUsize::new(0)),
        drops: Arc::new(AtomicUsize::new(0)),
    };
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let cancellation = CancellationToken::new();
    let mut future = worker.analyze(
        request(vec![png(1, b"PRIVATE_PENDING_SOURCE_IMAGE_SENTINEL")]),
        cancellation.clone(),
    );
    let notification_count = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(CountingWake {
        wakes: Arc::clone(&notification_count),
    }));

    for expected_notifications in 1..=2 {
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert_eq!(
            notification_count.load(Ordering::SeqCst),
            expected_notifications
        );
    }
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert_eq!(notification_count.load(Ordering::SeqCst), 2);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.polls.load(Ordering::SeqCst), 1);

    cancellation.cancel();
    assert_eq!(notification_count.load(Ordering::SeqCst), 3);
    let Poll::Ready(Err(error)) = future.as_mut().poll(&mut Context::from_waker(&waker)) else {
        panic!("pending-source cancellation must complete")
    };
    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 1);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_PENDING_SOURCE_IMAGE_SENTINEL"));
}

#[derive(Clone)]
struct PendingAfterDoneTransport {
    response: Arc<Vec<u8>>,
    drops: Arc<AtomicUsize>,
}

struct PendingAfterDoneStream {
    chunk: Option<Vec<u8>>,
    drops: Arc<AtomicUsize>,
}

impl Stream for PendingAfterDoneStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.chunk
            .take()
            .map_or(Poll::Pending, |chunk| Poll::Ready(Some(Ok(chunk))))
    }
}

impl Drop for PendingAfterDoneStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for PendingAfterDoneTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        let chunk = self.response.as_ref().clone();
        let drops = Arc::clone(&self.drops);
        Box::pin(async move {
            Ok(Box::pin(PendingAfterDoneStream {
                chunk: Some(chunk),
                drops,
            }) as AiGatewayByteStream)
        })
    }
}

#[test]
fn done_releases_pending_source_before_result_publication() {
    let drops = Arc::new(AtomicUsize::new(0));
    let transport = PendingAfterDoneTransport {
        response: Arc::new(sse_text(&valid_result().to_string())),
        drops: Arc::clone(&drops),
    };
    let result = execute(Arc::new(transport), request(vec![png(1, &[1])])).unwrap();
    assert_eq!(result.images().len(), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[derive(Clone)]
struct EmptyThenPendingTransport {
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

struct EmptyThenPendingStream {
    yielded_empty: bool,
    drops: Arc<AtomicUsize>,
}

impl Stream for EmptyThenPendingStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.yielded_empty {
            Poll::Pending
        } else {
            self.yielded_empty = true;
            Poll::Ready(Some(Ok(Vec::new())))
        }
    }
}

impl Drop for EmptyThenPendingStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for EmptyThenPendingTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let drops = Arc::clone(&self.drops);
        Box::pin(async move {
            Ok(Box::pin(EmptyThenPendingStream {
                yielded_empty: false,
                drops,
            }) as AiGatewayByteStream)
        })
    }
}

#[test]
fn empty_success_chunk_returns_protocol_failure_and_releases_pending_source() {
    let calls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let transport = EmptyThenPendingTransport {
        calls: Arc::clone(&calls),
        drops: Arc::clone(&drops),
    };
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport)).unwrap();
    let mut future = worker.analyze(
        request(vec![png(1, b"PRIVATE_EMPTY_CHUNK_IMAGE_SENTINEL")]),
        CancellationToken::new(),
    );

    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    let Poll::Ready(Err(error)) = poll_once(future.as_mut()) else {
        panic!("empty success chunk must complete without polling the pending source again")
    };
    assert_eq!(error.kind(), VisionTransportErrorKind::Protocol);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_EMPTY_CHUNK_IMAGE_SENTINEL"));
}

#[derive(Clone)]
struct CancelAfterChunkTransport {
    response: Arc<Vec<u8>>,
    drops: Arc<AtomicUsize>,
}

struct CancelAfterChunkStream {
    chunk: Option<Vec<u8>>,
    cancellation: CancellationToken,
    drops: Arc<AtomicUsize>,
}

impl Stream for CancelAfterChunkStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(chunk) = self.chunk.take() else {
            return Poll::Pending;
        };
        self.cancellation.cancel();
        Poll::Ready(Some(Ok(chunk)))
    }
}

impl Drop for CancelAfterChunkStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for CancelAfterChunkTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        let chunk = self.response.as_ref().clone();
        let drops = Arc::clone(&self.drops);
        Box::pin(async move {
            Ok(Box::pin(CancelAfterChunkStream {
                chunk: Some(chunk),
                cancellation,
                drops,
            }) as AiGatewayByteStream)
        })
    }
}

#[test]
fn cancellation_after_source_acquisition_drops_stream_before_returning() {
    let drops = Arc::new(AtomicUsize::new(0));
    let transport = CancelAfterChunkTransport {
        response: Arc::new(sse_text(&valid_result().to_string())),
        drops: Arc::clone(&drops),
    };
    let error = execute(Arc::new(transport), request(vec![png(1, &[1])])).unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[derive(Clone)]
struct CancelWithStartupErrorTransport;

impl AiGatewayTransport for CancelWithStartupErrorTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        Box::pin(async move {
            cancellation.cancel();
            Err(ProviderError::new(
                ProviderErrorKind::Transport,
                "PRIVATE_STARTUP_ERROR_CODE",
                "PRIVATE_STARTUP_ERROR_MESSAGE",
                true,
            ))
        })
    }
}

#[test]
fn cancellation_wins_same_poll_transport_startup_error() {
    let worker =
        AiGatewayVisionTransport::new(MODEL, Arc::new(CancelWithStartupErrorTransport)).unwrap();
    let error = futures_executor::block_on(worker.analyze(
        request(vec![png(1, b"PRIVATE_STARTUP_IMAGE_SENTINEL")]),
        CancellationToken::new(),
    ))
    .unwrap_err();

    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_STARTUP_ERROR_CODE"));
    assert!(!rendered.contains("PRIVATE_STARTUP_ERROR_MESSAGE"));
    assert!(!rendered.contains("PRIVATE_STARTUP_IMAGE_SENTINEL"));
}

#[derive(Clone)]
struct PendingStartupTransport {
    calls: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    live_request_bodies: Arc<AtomicUsize>,
}

struct PendingStartup {
    _request_body: Vec<u8>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    live_request_bodies: Arc<AtomicUsize>,
}

impl Future for PendingStartup {
    type Output = Result<AiGatewayByteStream, ProviderError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for PendingStartup {
    fn drop(&mut self) {
        assert_eq!(self.live_request_bodies.fetch_sub(1, Ordering::SeqCst), 1);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for PendingStartupTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            self.live_request_bodies.fetch_add(1, Ordering::SeqCst),
            0,
            "only one startup may own an encoded request body"
        );
        Box::pin(PendingStartup {
            _request_body: request.into_body(),
            polls: Arc::clone(&self.polls),
            drops: Arc::clone(&self.drops),
            live_request_bodies: Arc::clone(&self.live_request_bodies),
        })
    }
}

fn pending_startup_transport() -> PendingStartupTransport {
    PendingStartupTransport {
        calls: Arc::new(AtomicUsize::new(0)),
        polls: Arc::new(AtomicUsize::new(0)),
        drops: Arc::new(AtomicUsize::new(0)),
        live_request_bodies: Arc::new(AtomicUsize::new(0)),
    }
}

#[test]
fn cancellation_wakes_pending_startup_and_releases_its_request_body() {
    let transport = pending_startup_transport();
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let cancellation = CancellationToken::new();
    let mut future = worker.analyze(
        request(vec![png(1, b"PRIVATE_PENDING_STARTUP_IMAGE_SENTINEL")]),
        cancellation.clone(),
    );
    let notification_count = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(CountingWake {
        wakes: Arc::clone(&notification_count),
    }));

    // Encoding and pre-dispatch self-wake once each. Startup itself remains
    // pending without retaining or waking the supplied task Waker.
    for expected_notifications in 1..=2 {
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert_eq!(
            notification_count.load(Ordering::SeqCst),
            expected_notifications
        );
    }
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 0);
    assert_eq!(transport.live_request_bodies.load(Ordering::SeqCst), 1);

    let notifications_before_cancel = notification_count.load(Ordering::SeqCst);
    assert!(cancellation.cancel());
    assert_eq!(
        notification_count.load(Ordering::SeqCst),
        notifications_before_cancel + 1,
        "the worker-owned cancellation waiter must wake pending startup"
    );
    let Poll::Ready(Err(error)) = future.as_mut().poll(&mut Context::from_waker(&waker)) else {
        panic!("pending-startup cancellation must complete")
    };
    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.drops.load(Ordering::SeqCst), 1);
    assert_eq!(transport.live_request_bodies.load(Ordering::SeqCst), 0);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_PENDING_STARTUP_IMAGE_SENTINEL"));
}

#[test]
fn manual_repoll_checks_cancellation_before_polling_pending_startup_again() {
    let transport = pending_startup_transport();
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let cancellation = CancellationToken::new();
    let mut future = worker.analyze(
        request(vec![png(
            1,
            b"PRIVATE_MANUAL_REPOLL_STARTUP_IMAGE_SENTINEL",
        )]),
        cancellation.clone(),
    );

    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
    assert_eq!(transport.live_request_bodies.load(Ordering::SeqCst), 1);

    cancellation.cancel();
    let Poll::Ready(Err(error)) = poll_once(future.as_mut()) else {
        panic!("manual cancellation repoll must complete")
    };
    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(
        transport.polls.load(Ordering::SeqCst),
        1,
        "cancelled startup must not receive another poll"
    );
    assert_eq!(transport.drops.load(Ordering::SeqCst), 1);
    assert_eq!(transport.live_request_bodies.load(Ordering::SeqCst), 0);
}

#[derive(Clone)]
struct CancelWithStartupSuccessTransport {
    startup_drops: Arc<AtomicUsize>,
    stream_drops: Arc<AtomicUsize>,
    live_request_bodies: Arc<AtomicUsize>,
}

struct CancelWithStartupSuccess {
    cancellation: CancellationToken,
    _request_body: Vec<u8>,
    startup_drops: Arc<AtomicUsize>,
    stream_drops: Arc<AtomicUsize>,
    live_request_bodies: Arc<AtomicUsize>,
}

struct AcquiredPendingStream {
    drops: Arc<AtomicUsize>,
}

impl Stream for AcquiredPendingStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

impl Drop for AcquiredPendingStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl Future for CancelWithStartupSuccess {
    type Output = Result<AiGatewayByteStream, ProviderError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.cancellation.cancel();
        Poll::Ready(Ok(Box::pin(AcquiredPendingStream {
            drops: Arc::clone(&self.stream_drops),
        }) as AiGatewayByteStream))
    }
}

impl Drop for CancelWithStartupSuccess {
    fn drop(&mut self) {
        assert_eq!(self.live_request_bodies.fetch_sub(1, Ordering::SeqCst), 1);
        self.startup_drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for CancelWithStartupSuccessTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        assert_eq!(self.live_request_bodies.fetch_add(1, Ordering::SeqCst), 0);
        Box::pin(CancelWithStartupSuccess {
            cancellation,
            _request_body: request.into_body(),
            startup_drops: Arc::clone(&self.startup_drops),
            stream_drops: Arc::clone(&self.stream_drops),
            live_request_bodies: Arc::clone(&self.live_request_bodies),
        })
    }
}

#[test]
fn cancellation_wins_same_poll_startup_success_after_full_teardown() {
    let transport = CancelWithStartupSuccessTransport {
        startup_drops: Arc::new(AtomicUsize::new(0)),
        stream_drops: Arc::new(AtomicUsize::new(0)),
        live_request_bodies: Arc::new(AtomicUsize::new(0)),
    };
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let error = futures_executor::block_on(worker.analyze(
        request(vec![png(1, b"PRIVATE_STARTUP_SUCCESS_IMAGE_SENTINEL")]),
        CancellationToken::new(),
    ))
    .unwrap_err();

    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(transport.startup_drops.load(Ordering::SeqCst), 1);
    assert_eq!(transport.stream_drops.load(Ordering::SeqCst), 1);
    assert_eq!(transport.live_request_bodies.load(Ordering::SeqCst), 0);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_STARTUP_SUCCESS_IMAGE_SENTINEL"));
}

#[derive(Clone, Copy)]
enum DropCancellingStartupOutcome {
    Success,
    Error,
}

#[derive(Clone)]
struct DropCancellingStartupTransport {
    outcome: DropCancellingStartupOutcome,
    startup_drops: Arc<AtomicUsize>,
    stream_drops: Arc<AtomicUsize>,
    live_request_bodies: Arc<AtomicUsize>,
}

struct DropCancellingStartup {
    outcome: DropCancellingStartupOutcome,
    cancellation: CancellationToken,
    _request_body: Vec<u8>,
    startup_drops: Arc<AtomicUsize>,
    stream_drops: Arc<AtomicUsize>,
    live_request_bodies: Arc<AtomicUsize>,
}

impl Future for DropCancellingStartup {
    type Output = Result<AiGatewayByteStream, ProviderError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(match self.outcome {
            DropCancellingStartupOutcome::Success => Ok(Box::pin(AcquiredPendingStream {
                drops: Arc::clone(&self.stream_drops),
            }) as AiGatewayByteStream),
            DropCancellingStartupOutcome::Error => Err(ProviderError::new(
                ProviderErrorKind::Transport,
                "PRIVATE_DROP_STARTUP_ERROR_CODE",
                "PRIVATE_DROP_STARTUP_ERROR_MESSAGE",
                true,
            )),
        })
    }
}

impl Drop for DropCancellingStartup {
    fn drop(&mut self) {
        assert_eq!(self.live_request_bodies.fetch_sub(1, Ordering::SeqCst), 1);
        self.startup_drops.fetch_add(1, Ordering::SeqCst);
        self.cancellation.cancel();
    }
}

impl AiGatewayTransport for DropCancellingStartupTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        assert_eq!(self.live_request_bodies.fetch_add(1, Ordering::SeqCst), 0);
        Box::pin(DropCancellingStartup {
            outcome: self.outcome,
            cancellation,
            _request_body: request.into_body(),
            startup_drops: Arc::clone(&self.startup_drops),
            stream_drops: Arc::clone(&self.stream_drops),
            live_request_bodies: Arc::clone(&self.live_request_bodies),
        })
    }
}

fn assert_startup_drop_cancellation_wins(outcome: DropCancellingStartupOutcome) {
    let transport = DropCancellingStartupTransport {
        outcome,
        startup_drops: Arc::new(AtomicUsize::new(0)),
        stream_drops: Arc::new(AtomicUsize::new(0)),
        live_request_bodies: Arc::new(AtomicUsize::new(0)),
    };
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let error = futures_executor::block_on(worker.analyze(
        request(vec![png(1, b"PRIVATE_DROP_STARTUP_REQUEST_IMAGE_SENTINEL")]),
        CancellationToken::new(),
    ))
    .unwrap_err();

    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(transport.startup_drops.load(Ordering::SeqCst), 1);
    assert_eq!(transport.live_request_bodies.load(Ordering::SeqCst), 0);
    let expected_stream_drops = match outcome {
        DropCancellingStartupOutcome::Success => 1,
        DropCancellingStartupOutcome::Error => 0,
    };
    assert_eq!(
        transport.stream_drops.load(Ordering::SeqCst),
        expected_stream_drops
    );
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_DROP_STARTUP_ERROR_CODE"));
    assert!(!rendered.contains("PRIVATE_DROP_STARTUP_ERROR_MESSAGE"));
    assert!(!rendered.contains("PRIVATE_DROP_STARTUP_REQUEST_IMAGE_SENTINEL"));
}

#[test]
fn startup_request_owner_drop_cancellation_wins_ready_success() {
    assert_startup_drop_cancellation_wins(DropCancellingStartupOutcome::Success);
}

#[test]
fn startup_request_owner_drop_cancellation_wins_ready_error() {
    assert_startup_drop_cancellation_wins(DropCancellingStartupOutcome::Error);
}

#[derive(Clone)]
struct CancelWithStreamErrorTransport {
    drops: Arc<AtomicUsize>,
}

struct CancelWithStreamError {
    cancellation: CancellationToken,
    drops: Arc<AtomicUsize>,
}

impl Stream for CancelWithStreamError {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.cancellation.cancel();
        Poll::Ready(Some(Err(ProviderError::new(
            ProviderErrorKind::Transport,
            "PRIVATE_STREAM_ERROR_CODE",
            "PRIVATE_STREAM_ERROR_MESSAGE",
            true,
        ))))
    }
}

impl Drop for CancelWithStreamError {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AiGatewayTransport for CancelWithStreamErrorTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        let drops = Arc::clone(&self.drops);
        Box::pin(async move {
            Ok(Box::pin(CancelWithStreamError {
                cancellation,
                drops,
            }) as AiGatewayByteStream)
        })
    }
}

#[test]
fn cancellation_wins_same_poll_stream_item_error_and_releases_source() {
    let drops = Arc::new(AtomicUsize::new(0));
    let worker = AiGatewayVisionTransport::new(
        MODEL,
        Arc::new(CancelWithStreamErrorTransport {
            drops: Arc::clone(&drops),
        }),
    )
    .unwrap();
    let error = futures_executor::block_on(worker.analyze(
        request(vec![png(1, b"PRIVATE_STREAM_ERROR_IMAGE_SENTINEL")]),
        CancellationToken::new(),
    ))
    .unwrap_err();

    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_STREAM_ERROR_CODE"));
    assert!(!rendered.contains("PRIVATE_STREAM_ERROR_MESSAGE"));
    assert!(!rendered.contains("PRIVATE_STREAM_ERROR_IMAGE_SENTINEL"));
}

#[derive(Clone, Copy)]
enum DropCancellingStreamOutcome {
    ItemError,
    EmptyChunk,
    DecoderError,
}

#[derive(Clone)]
struct DropCancellingStreamTransport {
    outcome: DropCancellingStreamOutcome,
    drops: Arc<AtomicUsize>,
}

struct DropCancellingStream {
    outcome: DropCancellingStreamOutcome,
    yielded: bool,
    cancellation: CancellationToken,
    drops: Arc<AtomicUsize>,
}

impl Stream for DropCancellingStream {
    type Item = Result<Vec<u8>, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.yielded {
            return Poll::Pending;
        }
        self.yielded = true;
        Poll::Ready(Some(match self.outcome {
            DropCancellingStreamOutcome::ItemError => Err(ProviderError::new(
                ProviderErrorKind::Transport,
                "PRIVATE_DROP_STREAM_ERROR_CODE",
                "PRIVATE_DROP_STREAM_ERROR_MESSAGE",
                true,
            )),
            DropCancellingStreamOutcome::EmptyChunk => Ok(Vec::new()),
            DropCancellingStreamOutcome::DecoderError => Ok(b"data: {}\rX".to_vec()),
        }))
    }
}

impl Drop for DropCancellingStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.cancellation.cancel();
    }
}

impl AiGatewayTransport for DropCancellingStreamTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        let outcome = self.outcome;
        let drops = Arc::clone(&self.drops);
        Box::pin(async move {
            Ok(Box::pin(DropCancellingStream {
                outcome,
                yielded: false,
                cancellation,
                drops,
            }) as AiGatewayByteStream)
        })
    }
}

fn assert_drop_cancellation_wins(outcome: DropCancellingStreamOutcome) {
    let drops = Arc::new(AtomicUsize::new(0));
    let worker = AiGatewayVisionTransport::new(
        MODEL,
        Arc::new(DropCancellingStreamTransport {
            outcome,
            drops: Arc::clone(&drops),
        }),
    )
    .unwrap();
    let error = futures_executor::block_on(worker.analyze(
        request(vec![png(1, b"PRIVATE_DROP_CANCELLATION_IMAGE_SENTINEL")]),
        CancellationToken::new(),
    ))
    .unwrap_err();

    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("PRIVATE_DROP_STREAM_ERROR_CODE"));
    assert!(!rendered.contains("PRIVATE_DROP_STREAM_ERROR_MESSAGE"));
    assert!(!rendered.contains("PRIVATE_DROP_CANCELLATION_IMAGE_SENTINEL"));
}

#[test]
fn stream_drop_cancellation_wins_item_error() {
    assert_drop_cancellation_wins(DropCancellingStreamOutcome::ItemError);
}

#[test]
fn stream_drop_cancellation_wins_empty_chunk_error() {
    assert_drop_cancellation_wins(DropCancellingStreamOutcome::EmptyChunk);
}

#[test]
fn stream_drop_cancellation_wins_decoder_error() {
    assert_drop_cancellation_wins(DropCancellingStreamOutcome::DecoderError);
}

#[test]
fn future_is_inert_before_poll_and_precancelled_request_never_reaches_transport() {
    let transport = ScriptedTransport::one(sse_text(&valid_result().to_string()));
    let worker = AiGatewayVisionTransport::new(MODEL, Arc::new(transport.clone())).unwrap();
    let token = CancellationToken::new();
    let mut future = worker.analyze(request(vec![png(1, &[1])]), token.clone());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    token.cancel();
    let Poll::Ready(Err(error)) = poll_once(future.as_mut()) else {
        panic!("pre-cancelled future should complete")
    };
    assert_eq!(error.kind(), VisionTransportErrorKind::Cancelled);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn config_and_transport_debug_never_reflect_model_or_provider_data() {
    let invalid = AiGatewayVisionTransport::new(
        "PRIVATE MODEL SENTINEL",
        Arc::new(ScriptedTransport::new(Vec::new())),
    )
    .unwrap_err();
    assert_eq!(invalid.kind(), AiGatewayVisionConfigErrorKind::InvalidModel);
    assert!(!format!("{invalid:?} {invalid}").contains("PRIVATE MODEL SENTINEL"));

    let worker = AiGatewayVisionTransport::new(
        "private/model-sentinel",
        Arc::new(ScriptedTransport::new(Vec::new())),
    )
    .unwrap();
    assert!(!format!("{worker:?}").contains("private/model-sentinel"));
}
