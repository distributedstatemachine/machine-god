#![cfg(all(feature = "ai-gateway-http", not(target_family = "wasm")))]

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures_util::{Stream, stream};
use machine_god_core::{BoxFuture, CancellationToken, ProviderError, ProviderErrorKind, SessionId};
use machine_god_native::{
    AI_GATEWAY_LANGUAGE_MODEL_SPECIFICATION_VERSION, AI_GATEWAY_PROTOCOL_VERSION,
    AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest,
    AiGatewayVisionConfigErrorKind, AiGatewayVisionTransport, MAX_VISION_ATTEMPT_EVIDENCE_BYTES,
    MAX_VISION_BATCH_RAW_BYTES, MAX_VISION_REQUEST_BYTES, MAX_VISION_RESPONSE_BYTES,
    VisionBatchRequest, VisionImage, VisionImageOutcome, VisionMediaType, VisionTransport,
    VisionTransportErrorKind,
};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

const MODEL: &str = "test/configured-vision-model";
const PRIVATE_FOCUS: &str = "inspect PRIVATE_FOCUS_SENTINEL";

#[derive(Clone, Debug, Eq, PartialEq)]
struct CapturedRequest {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
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
        self.requests.lock().unwrap().push(CapturedRequest {
            headers: headers
                .into_iter()
                .map(machine_god_native::AiGatewayHeader::into_parts)
                .collect(),
            body,
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

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(&Waker::from(Arc::new(NoopWake))))
}

fn request(images: Vec<VisionImage>) -> VisionBatchRequest {
    VisionBatchRequest::new(
        SessionId::new("vision-gateway-session").unwrap(),
        PRIVATE_FOCUS.to_owned(),
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

fn execute(
    transport: Arc<dyn AiGatewayTransport>,
    request: VisionBatchRequest,
) -> Result<machine_god_native::VisionBatchResponse, machine_god_native::VisionTransportError> {
    let worker = AiGatewayVisionTransport::new(MODEL, transport).unwrap();
    futures_executor::block_on(worker.analyze(request, CancellationToken::new()))
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
fn exact_raw_batch_limit_fits_the_serialized_request_ceiling() {
    let transport = ScriptedTransport::one(sse_text(&valid_result().to_string()));
    let bytes = vec![0xa5; MAX_VISION_BATCH_RAW_BYTES];
    execute(
        Arc::new(transport.clone()),
        request(vec![
            VisionImage::new(1, VisionMediaType::Png, bytes).unwrap(),
        ]),
    )
    .unwrap();
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body.len() <= MAX_VISION_REQUEST_BYTES);
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

#[test]
fn second_semantic_invalidity_returns_stable_error_after_exactly_two_attempts() {
    let invalid = sse_text(r#"{"images":[]}"#);
    let transport = ScriptedTransport::new(vec![vec![invalid.clone()], vec![invalid]]);
    let error = execute(Arc::new(transport.clone()), request(vec![png(1, &[1])])).unwrap_err();
    assert_eq!(error.kind(), VisionTransportErrorKind::InvalidResponse);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
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
fn strict_text_lifecycle_rejects_delta_end_or_finish_out_of_sequence() {
    let cases = [
        vec![
            json!({"type": "text-delta", "id": "vision-text", "delta": "{}"}),
            json!({"type": "text-end", "id": "vision-text"}),
            json!({"type": "finish", "finishReason": {"unified": "stop"}}),
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
