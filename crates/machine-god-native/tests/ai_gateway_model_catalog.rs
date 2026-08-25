use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Instant;

use machine_god_core::{
    AvailableModel, CancellationToken, ModelCatalog, ModelCatalogAccess, ModelCatalogProvider,
    ProviderError, ProviderErrorKind, PublicCatalogReason,
};
use machine_god_native::{
    AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES, AI_GATEWAY_MODEL_CATALOG_MAX_JSON_NODES,
    AI_GATEWAY_MODEL_CATALOG_MAX_MODEL_ID_BYTES, AI_GATEWAY_MODEL_CATALOG_MAX_MODELS,
    AI_GATEWAY_MODEL_CATALOG_MAX_RAW_ENTRIES, AiGatewayModelCatalogAccessMode,
    AiGatewayModelCatalogProvider, AiGatewayModelCatalogRequestAccess,
    AiGatewayModelCatalogTransport, AiGatewayModelCatalogTransportError,
    AiGatewayModelCatalogTransportErrorKind, AiGatewayModelCatalogTransportResponse,
};
use serde_json::{Value, json};

#[derive(Debug)]
enum Action {
    Response(u16, Vec<u8>),
    Error(AiGatewayModelCatalogTransportErrorKind),
    CancelAndRespond(u16, Vec<u8>),
    Pending(Arc<AtomicUsize>),
}

#[derive(Debug)]
struct Call {
    access: AiGatewayModelCatalogRequestAccess,
    deadline: Instant,
}

#[derive(Debug)]
struct ScriptedTransport {
    actions: Mutex<VecDeque<Action>>,
    calls: Mutex<Vec<Call>>,
    deadline: Arc<DeadlineGate>,
}

impl ScriptedTransport {
    fn new(actions: impl IntoIterator<Item = Action>) -> Arc<Self> {
        Arc::new(Self {
            actions: Mutex::new(actions.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
            deadline: Arc::new(DeadlineGate::default()),
        })
    }

    fn call_accesses(&self) -> Vec<AiGatewayModelCatalogRequestAccess> {
        self.calls
            .lock()
            .expect("calls lock")
            .iter()
            .map(|call| call.access)
            .collect()
    }

    fn expire_deadline(&self) {
        self.deadline.expire();
    }
}

impl AiGatewayModelCatalogTransport for ScriptedTransport {
    fn wait_until(&self, _: Instant) -> machine_god_core::BoxFuture<'_, ()> {
        Box::pin(DeadlineFuture {
            gate: Arc::clone(&self.deadline),
        })
    }

    fn get(
        &self,
        access: AiGatewayModelCatalogRequestAccess,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> machine_god_core::BoxFuture<
        '_,
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>,
    > {
        self.calls
            .lock()
            .expect("calls lock")
            .push(Call { access, deadline });
        let action = self
            .actions
            .lock()
            .expect("actions lock")
            .pop_front()
            .expect("transport called beyond script");
        match action {
            Action::Response(status, body) => {
                Box::pin(
                    async move { Ok(AiGatewayModelCatalogTransportResponse::new(status, body)) },
                )
            }
            Action::Error(kind) => {
                Box::pin(async move { Err(AiGatewayModelCatalogTransportError::new(kind)) })
            }
            Action::CancelAndRespond(status, body) => Box::pin(async move {
                cancellation.cancel();
                Ok(AiGatewayModelCatalogTransportResponse::new(status, body))
            }),
            Action::Pending(drops) => Box::pin(PendingTransportFuture {
                drops,
                polled: false,
            }),
        }
    }
}

#[derive(Debug, Default)]
struct DeadlineGate {
    expired: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl DeadlineGate {
    fn expire(&self) {
        self.expired.store(true, Ordering::Release);
        if let Some(waker) = self.waker.lock().expect("deadline waker lock").take() {
            waker.wake();
        }
    }
}

struct DeadlineFuture {
    gate: Arc<DeadlineGate>,
}

impl Future for DeadlineFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.gate.expired.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        *self.gate.waker.lock().expect("deadline waker lock") = Some(context.waker().clone());
        if self.gate.expired.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

struct PendingTransportFuture {
    drops: Arc<AtomicUsize>,
    polled: bool,
}

impl Future for PendingTransportFuture {
    type Output =
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        self.polled = true;
        Poll::Pending
    }
}

impl Drop for PendingTransportFuture {
    fn drop(&mut self) {
        if self.polled {
            self.drops.fetch_add(1, Ordering::AcqRel);
        }
    }
}

#[derive(Debug)]
struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn body(value: &Value) -> Vec<u8> {
    serde_json::to_vec(&value).expect("serialize catalog fixture")
}

fn one_model_body(id: &str) -> Vec<u8> {
    body(&json!({"data": [{"id": id, "type": "language"}]}))
}

fn list(
    mode: AiGatewayModelCatalogAccessMode,
    transport: Arc<ScriptedTransport>,
    cancellation: CancellationToken,
) -> Result<ModelCatalog, ProviderError> {
    let provider = AiGatewayModelCatalogProvider::new(mode, transport);
    futures_executor::block_on(provider.list_models(cancellation))
}

fn public_catalog(body: Vec<u8>) -> Result<ModelCatalog, ProviderError> {
    list(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        ScriptedTransport::new([Action::Response(200, body)]),
        CancellationToken::new(),
    )
}

fn assert_error(error: &ProviderError, kind: ProviderErrorKind, code: &str, retryable: bool) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
    assert_eq!(error.retryable, retryable);
    let diagnostic = format!("{error:?} {error}");
    for secret in [
        "HOSTILE_SECRET",
        "https://hostile.invalid/private",
        "Bearer",
        "401",
        "403",
        "500",
    ] {
        assert!(
            !diagnostic.contains(secret),
            "diagnostic reflected {secret}"
        );
    }
}

#[test]
fn public_and_authenticated_success_preserve_final_access() {
    let public_transport =
        ScriptedTransport::new([Action::Response(200, one_model_body("public/model"))]);
    let public = list(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        Arc::clone(&public_transport),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(public.models()[0].id(), "public/model");
    assert_eq!(
        public.access(),
        ModelCatalogAccess::PublicOnly {
            reason: PublicCatalogReason::NoCredential
        }
    );
    assert_eq!(
        public_transport.call_accesses(),
        [AiGatewayModelCatalogRequestAccess::Public]
    );

    let authenticated_transport =
        ScriptedTransport::new([Action::Response(200, one_model_body("private/model"))]);
    let authenticated = list(
        AiGatewayModelCatalogAccessMode::Authenticated,
        Arc::clone(&authenticated_transport),
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(authenticated.models()[0].id(), "private/model");
    assert_eq!(authenticated.access(), ModelCatalogAccess::Authenticated);
    assert_eq!(
        authenticated_transport.call_accesses(),
        [AiGatewayModelCatalogRequestAccess::Authenticated]
    );
}

#[test]
fn authenticated_401_and_403_fallback_once_anonymously_under_one_deadline() {
    for status in [401, 403] {
        let transport = ScriptedTransport::new([
            Action::Response(status, b"HOSTILE_SECRET".to_vec()),
            Action::Response(200, one_model_body("public/fallback")),
        ]);
        let catalog = list(
            AiGatewayModelCatalogAccessMode::Authenticated,
            Arc::clone(&transport),
            CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(catalog.models()[0].id(), "public/fallback");
        assert_eq!(
            catalog.access(),
            ModelCatalogAccess::PublicOnly {
                reason: PublicCatalogReason::AuthenticatedCredentialRejected
            }
        );
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls.iter().map(|call| call.access).collect::<Vec<_>>(),
            [
                AiGatewayModelCatalogRequestAccess::Authenticated,
                AiGatewayModelCatalogRequestAccess::Public
            ]
        );
        assert_eq!(calls[0].deadline, calls[1].deadline);
    }
}

#[test]
fn public_auth_rejections_and_all_other_authenticated_failures_never_retry() {
    for status in [401, 403] {
        let transport = ScriptedTransport::new([Action::Response(status, Vec::new())]);
        let error = list(
            AiGatewayModelCatalogAccessMode::PublicOnly,
            Arc::clone(&transport),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_error(
            &error,
            ProviderErrorKind::Authentication,
            "AuthenticationRejected",
            false,
        );
        assert_eq!(transport.call_accesses().len(), 1);
    }

    for (status, kind, code, retryable) in [
        (300, ProviderErrorKind::Unavailable, "Unavailable", false),
        (302, ProviderErrorKind::Unavailable, "Unavailable", false),
        (399, ProviderErrorKind::Unavailable, "Unavailable", false),
        (400, ProviderErrorKind::Unavailable, "Unavailable", false),
        (404, ProviderErrorKind::Unavailable, "Unavailable", false),
        (429, ProviderErrorKind::RateLimited, "RateLimited", true),
        (
            500,
            ProviderErrorKind::Unavailable,
            "GatewayUnavailable",
            true,
        ),
        (
            502,
            ProviderErrorKind::Unavailable,
            "GatewayUnavailable",
            true,
        ),
        (
            503,
            ProviderErrorKind::Unavailable,
            "GatewayUnavailable",
            true,
        ),
        (
            504,
            ProviderErrorKind::Unavailable,
            "GatewayUnavailable",
            true,
        ),
        (
            501,
            ProviderErrorKind::Unavailable,
            "GatewayUnavailable",
            false,
        ),
        (
            599,
            ProviderErrorKind::Unavailable,
            "GatewayUnavailable",
            false,
        ),
    ] {
        let transport =
            ScriptedTransport::new([Action::Response(status, b"HOSTILE_SECRET".to_vec())]);
        let error = list(
            AiGatewayModelCatalogAccessMode::Authenticated,
            Arc::clone(&transport),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_error(&error, kind, code, retryable);
        assert_eq!(transport.call_accesses().len(), 1, "status {status}");
    }
}

#[test]
fn transport_errors_map_exactly_and_never_fallback() {
    for (transport_kind, kind, code, retryable) in [
        (
            AiGatewayModelCatalogTransportErrorKind::Transport,
            ProviderErrorKind::Transport,
            "TransportFailure",
            true,
        ),
        (
            AiGatewayModelCatalogTransportErrorKind::MalformedResponse,
            ProviderErrorKind::Protocol,
            "MalformedResponse",
            false,
        ),
        (
            AiGatewayModelCatalogTransportErrorKind::ResourceLimit,
            ProviderErrorKind::Protocol,
            "ResourceLimit",
            false,
        ),
        (
            AiGatewayModelCatalogTransportErrorKind::RuntimeRequired,
            ProviderErrorKind::Transport,
            "RuntimeRequired",
            false,
        ),
        (
            AiGatewayModelCatalogTransportErrorKind::Cancelled,
            ProviderErrorKind::Cancelled,
            "Cancelled",
            false,
        ),
    ] {
        let transport = ScriptedTransport::new([Action::Error(transport_kind)]);
        let error = list(
            AiGatewayModelCatalogAccessMode::Authenticated,
            Arc::clone(&transport),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert_error(&error, kind, code, retryable);
        assert_eq!(transport.call_accesses().len(), 1);
    }
}

#[test]
fn authenticated_success_status_parse_and_resource_defects_never_fallback() {
    for body in [
        b"HOSTILE_SECRET malformed JSON".to_vec(),
        br#"{"data":[{"id":"duplicate"},{"id":"duplicate"}]}"#.to_vec(),
        vec![b'x'; AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES + 1],
    ] {
        let transport = ScriptedTransport::new([Action::Response(200, body)]);
        let error = list(
            AiGatewayModelCatalogAccessMode::Authenticated,
            Arc::clone(&transport),
            CancellationToken::new(),
        )
        .unwrap_err();
        assert!(matches!(
            error.code.as_str(),
            "MalformedResponse" | "ResourceLimit"
        ));
        assert_eq!(
            transport.call_accesses(),
            [AiGatewayModelCatalogRequestAccess::Authenticated]
        );
        assert!(!format!("{error:?} {error}").contains("HOSTILE_SECRET"));
    }
}

#[test]
fn futures_are_inert_drop_owned_and_cancellation_wins_same_poll() {
    let unpolled_transport = ScriptedTransport::new([]);
    let provider = AiGatewayModelCatalogProvider::new(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        Arc::clone(&unpolled_transport) as Arc<dyn AiGatewayModelCatalogTransport>,
    );
    let unpolled = provider.list_models(CancellationToken::new());
    drop(unpolled);
    assert!(unpolled_transport.call_accesses().is_empty());

    let drops = Arc::new(AtomicUsize::new(0));
    let pending_transport = ScriptedTransport::new([Action::Pending(Arc::clone(&drops))]);
    let provider = AiGatewayModelCatalogProvider::new(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        Arc::clone(&pending_transport) as Arc<dyn AiGatewayModelCatalogTransport>,
    );
    let mut pending = provider.list_models(CancellationToken::new());
    assert!(poll_once(pending.as_mut()).is_pending());
    assert_eq!(pending_transport.call_accesses().len(), 1);
    drop(pending);
    assert_eq!(drops.load(Ordering::Acquire), 1);

    let cancellation = CancellationToken::new();
    let same_poll_transport = ScriptedTransport::new([Action::CancelAndRespond(
        200,
        one_model_body("must/not-escape"),
    )]);
    let error = list(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        same_poll_transport,
        cancellation,
    )
    .unwrap_err();
    assert_error(&error, ProviderErrorKind::Cancelled, "Cancelled", false);
}

#[test]
fn provider_deadline_wakes_pending_transport_and_cancellation_keeps_precedence() {
    let drops = Arc::new(AtomicUsize::new(0));
    let transport = ScriptedTransport::new([Action::Pending(Arc::clone(&drops))]);
    let provider = AiGatewayModelCatalogProvider::new(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        Arc::clone(&transport) as Arc<dyn AiGatewayModelCatalogTransport>,
    );
    let mut request = provider.list_models(CancellationToken::new());
    assert!(poll_once(request.as_mut()).is_pending());
    transport.expire_deadline();
    let error = futures_executor::block_on(request).unwrap_err();
    assert_error(&error, ProviderErrorKind::Protocol, "ResourceLimit", false);
    assert_eq!(drops.load(Ordering::Acquire), 1);

    let drops = Arc::new(AtomicUsize::new(0));
    let transport = ScriptedTransport::new([Action::Pending(Arc::clone(&drops))]);
    let cancellation = CancellationToken::new();
    let provider = AiGatewayModelCatalogProvider::new(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        Arc::clone(&transport) as Arc<dyn AiGatewayModelCatalogTransport>,
    );
    let mut request = provider.list_models(cancellation.clone());
    assert!(poll_once(request.as_mut()).is_pending());
    transport.expire_deadline();
    cancellation.cancel();
    let error = futures_executor::block_on(request).unwrap_err();
    assert_error(&error, ProviderErrorKind::Cancelled, "Cancelled", false);
    assert_eq!(drops.load(Ordering::Acquire), 1);
}

#[test]
fn cancellation_before_and_between_attempts_stops_all_later_work() {
    let before = CancellationToken::new();
    before.cancel();
    let transport = ScriptedTransport::new([]);
    let error = list(
        AiGatewayModelCatalogAccessMode::Authenticated,
        Arc::clone(&transport),
        before,
    )
    .unwrap_err();
    assert_error(&error, ProviderErrorKind::Cancelled, "Cancelled", false);
    assert!(transport.call_accesses().is_empty());

    let transport = ScriptedTransport::new([Action::CancelAndRespond(401, Vec::new())]);
    let error = list(
        AiGatewayModelCatalogAccessMode::Authenticated,
        Arc::clone(&transport),
        CancellationToken::new(),
    )
    .unwrap_err();
    assert_error(&error, ProviderErrorKind::Cancelled, "Cancelled", false);
    assert_eq!(
        transport.call_accesses(),
        [AiGatewayModelCatalogRequestAccess::Authenticated]
    );
}

#[test]
fn parser_accepts_empty_and_skips_only_the_frozen_entry_classes() {
    let catalog = public_catalog(body(&json!({"data": []}))).unwrap();
    assert!(catalog.models().is_empty());

    let catalog = public_catalog(body(&json!({
        "data": [
            null,
            7,
            [],
            {"type": "language"},
            {"id": 7, "type": "language"},
            {"id": "ignored/image", "type": "image"},
            {"id": "accepted/absent-type"},
            {"id": "accepted/non-string-type", "type": 9},
            {"id": "accepted/case", "type": "LaNgUaGe"}
        ]
    })))
    .unwrap();
    assert_eq!(
        catalog
            .models()
            .iter()
            .map(AvailableModel::id)
            .collect::<Vec<_>>(),
        [
            "accepted/absent-type",
            "accepted/case",
            "accepted/non-string-type"
        ]
    );
}

#[test]
fn standards_valid_out_of_range_numbers_default_or_ignore_without_losing_entries() {
    let oversized_integer = "9".repeat(310);
    for number in [oversized_integer.as_str(), "1e400"] {
        let catalog = public_catalog(
            format!(
                r#"{{"unknown_top":{number},"data":[{number},{{"id":"other/z","type":{number},"released":{number},"tags":[{number},"tool-use"],"unknown_entry":{number}}},{{"id":"other/a","released":1,"tags":["tool-use"]}}]}}"#
            )
            .into_bytes(),
        )
        .unwrap();
        assert_eq!(
            catalog
                .models()
                .iter()
                .map(AvailableModel::id)
                .collect::<Vec<_>>(),
            ["other/a", "other/z"]
        );
    }
}

#[test]
fn unsafe_ids_duplicate_fields_and_duplicate_ids_are_terminal_and_redacted() {
    for invalid in [
        String::new(),
        "contains space".to_owned(),
        "contains\ncontrol".to_owned(),
        "non-ascii-é".to_owned(),
        "x".repeat(129),
    ] {
        let error = public_catalog(body(&json!({"data": [{"id": invalid}]}))).unwrap_err();
        assert_error(
            &error,
            ProviderErrorKind::Protocol,
            "MalformedResponse",
            false,
        );
    }

    for raw in [
        br#"{"data":[],"data":[]}"#.to_vec(),
        br#"{"data":[{"id":"safe/a","id":"safe/b"}]}"#.to_vec(),
        br#"{"data":[{"id":"safe/a","type":"language","type":"image"}]}"#.to_vec(),
        br#"{"data":[{"id":"safe/a","tags":[],"tags":[]}]}"#.to_vec(),
        br#"{"data":[{"id":"safe/a","released":1,"released":2}]}"#.to_vec(),
        br#"{"data":[{"id":"HOSTILE_SECRET"},{"id":"HOSTILE_SECRET"}]}"#.to_vec(),
    ] {
        let error = public_catalog(raw).unwrap_err();
        assert_error(
            &error,
            ProviderErrorKind::Protocol,
            "MalformedResponse",
            false,
        );
    }
}

#[test]
fn root_data_utf8_and_trailing_rules_are_strict() {
    for malformed in [
        b"[]".to_vec(),
        b"null".to_vec(),
        br"{}".to_vec(),
        br#"{"data":null}"#.to_vec(),
        br#"{"data":{}}"#.to_vec(),
        br#"{"data":[]}{"data":[]}"#.to_vec(),
        vec![b'{', b'"', b'd', b'a', b't', b'a', b'"', b':', b'[', 0xff],
    ] {
        let error = public_catalog(malformed).unwrap_err();
        assert_error(
            &error,
            ProviderErrorKind::Protocol,
            "MalformedResponse",
            false,
        );
    }
    public_catalog(b" {\n\t\"data\" : [] } \r\n".to_vec()).unwrap();
}

fn fixed_id(index: usize, length: usize) -> String {
    let prefix = format!("p/{index:03}/");
    assert!(prefix.len() <= length);
    format!("{prefix}{}", "x".repeat(length - prefix.len()))
}

#[test]
fn body_raw_valid_and_aggregate_limits_are_inclusive() {
    let empty = br#"{"data":[],"pad":""}"#;
    let mut exact_body = br#"{"data":[],"pad":""}"#.to_vec();
    let insert_at = exact_body.len() - 2;
    exact_body.splice(
        insert_at..insert_at,
        std::iter::repeat_n(b'x', AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES - empty.len()),
    );
    assert_eq!(exact_body.len(), AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES);
    public_catalog(exact_body.clone()).unwrap();
    exact_body.push(b' ');
    let error = public_catalog(exact_body).unwrap_err();
    assert_error(&error, ProviderErrorKind::Protocol, "ResourceLimit", false);

    let exact_raw = json!({"data": vec![Value::Null; AI_GATEWAY_MODEL_CATALOG_MAX_RAW_ENTRIES]});
    public_catalog(body(&exact_raw)).unwrap();
    let excess_raw =
        json!({"data": vec![Value::Null; AI_GATEWAY_MODEL_CATALOG_MAX_RAW_ENTRIES + 1]});
    let error = public_catalog(body(&excess_raw)).unwrap_err();
    assert_eq!(error.code, "ResourceLimit");

    let exact_models = (0..AI_GATEWAY_MODEL_CATALOG_MAX_MODELS)
        .map(|index| json!({"id": fixed_id(index, 12)}))
        .collect::<Vec<_>>();
    assert_eq!(
        public_catalog(body(&json!({"data": exact_models})))
            .unwrap()
            .models()
            .len(),
        AI_GATEWAY_MODEL_CATALOG_MAX_MODELS
    );
    let excess_models = (0..=AI_GATEWAY_MODEL_CATALOG_MAX_MODELS)
        .map(|index| json!({"id": fixed_id(index, 12)}))
        .collect::<Vec<_>>();
    assert_eq!(
        public_catalog(body(&json!({"data": excess_models})))
            .unwrap_err()
            .code,
        "ResourceLimit"
    );

    let id_len = AI_GATEWAY_MODEL_CATALOG_MAX_MODEL_ID_BYTES / AI_GATEWAY_MODEL_CATALOG_MAX_MODELS;
    let exact_ids = (0..AI_GATEWAY_MODEL_CATALOG_MAX_MODELS)
        .map(|index| json!({"id": fixed_id(index, id_len)}))
        .collect::<Vec<_>>();
    let catalog = public_catalog(body(&json!({"data": exact_ids}))).unwrap();
    assert_eq!(
        catalog
            .models()
            .iter()
            .map(|model| model.id().len())
            .sum::<usize>(),
        AI_GATEWAY_MODEL_CATALOG_MAX_MODEL_ID_BYTES
    );
    let mut excess_ids = (0..AI_GATEWAY_MODEL_CATALOG_MAX_MODELS)
        .map(|index| fixed_id(index, id_len))
        .collect::<Vec<_>>();
    excess_ids[0].push('x');
    let error = public_catalog(body(&json!({
        "data": excess_ids.into_iter().map(|id| json!({"id": id})).collect::<Vec<_>>()
    })))
    .unwrap_err();
    assert_eq!(error.code, "ResourceLimit");
}

#[test]
fn json_depth_and_node_limits_include_ignored_fields() {
    let mut nested = "null".to_owned();
    for _ in 0..31 {
        nested = format!("[{nested}]");
    }
    public_catalog(format!(r#"{{"ignored":{nested},"data":[]}}"#).into_bytes()).unwrap();
    nested = format!("[{nested}]");
    assert_eq!(
        public_catalog(format!(r#"{{"ignored":{nested},"data":[]}}"#).into_bytes())
            .unwrap_err()
            .code,
        "ResourceLimit"
    );

    // Root, ignored array, each ignored null, and data array are all nodes.
    let exact_ignored = AI_GATEWAY_MODEL_CATALOG_MAX_JSON_NODES - 3;
    let exact = format!(
        r#"{{"ignored":[{}],"data":[]}}"#,
        std::iter::repeat_n("null", exact_ignored)
            .collect::<Vec<_>>()
            .join(",")
    );
    public_catalog(exact.into_bytes()).unwrap();
    let excess = format!(
        r#"{{"ignored":[{}],"data":[]}}"#,
        std::iter::repeat_n("null", exact_ignored + 1)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert_eq!(
        public_catalog(excess.into_bytes()).unwrap_err().code,
        "ResourceLimit"
    );
}

#[test]
fn out_of_range_numbers_are_one_node_and_obey_the_exact_container_depth() {
    let oversized_integer = "9".repeat(310);
    for number in [oversized_integer.as_str(), "1e400"] {
        let mut nested = number.to_owned();
        for _ in 0..31 {
            nested = format!("[{nested}]");
        }
        public_catalog(format!(r#"{{"ignored":{nested},"data":[]}}"#).into_bytes()).unwrap();
        nested = format!("[{nested}]");
        assert_eq!(
            public_catalog(format!(r#"{{"ignored":{nested},"data":[]}}"#).into_bytes())
                .unwrap_err()
                .code,
            "ResourceLimit"
        );
    }

    // Root, ignored array, each exponent, and data array are all exactly one node.
    let exact_ignored = AI_GATEWAY_MODEL_CATALOG_MAX_JSON_NODES - 3;
    let exact = format!(
        r#"{{"ignored":[{}],"data":[]}}"#,
        std::iter::repeat_n("1e400", exact_ignored)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(exact.len() < AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES);
    public_catalog(exact.into_bytes()).unwrap();
    let excess = format!(
        r#"{{"ignored":[{}],"data":[]}}"#,
        std::iter::repeat_n("1e400", exact_ignored + 1)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert_eq!(
        public_catalog(excess.into_bytes()).unwrap_err().code,
        "ResourceLimit"
    );
}

#[test]
fn metadata_defaults_and_full_sort_order_are_exact() {
    let entries = vec![
        json!({"id":"other/non-tool-pro","released":999,"tags":[]}),
        json!({"id":"other/BETA-opus","released":99,"tags":["TOOL-USE"]}),
        json!({"id":"other/MINI-flash","released":99,"tags":["tool-use"]}),
        json!({"id":"other/FLASH-pro","released":99,"tags":["tool-use"]}),
        json!({"id":"other/plain","released":"bad","tags":[7,"tool-use"]}),
        json!({"id":"anthropic/opus","released":1,"tags":["tool-use"]}),
        json!({"id":"openai/gpt-5-z","released":2,"tags":["tool-use"]}),
        json!({"id":"openai/gpt-5-a","released":2,"tags":["tool-use"]}),
        json!({"id":"openai/gpt-5-old","released":1,"tags":["tool-use"]}),
        json!({"id":"google/pro","released":1,"tags":["tool-use"]}),
        json!({"id":"xai/grok-4","released":1,"tags":["tool-use"]}),
        json!({"id":"deepseek/pro","released":1,"tags":["tool-use"]}),
        json!({"id":"meta/pro","released":1,"tags":["tool-use"]}),
        json!({"id":"mistral/pro","released":1,"tags":["tool-use"]}),
        json!({"id":"alibaba/pro","released":1,"tags":["tool-use"]}),
        json!({"id":"Other/pro","released":1,"tags":["tool-use"]}),
    ];
    let catalog = public_catalog(body(&json!({"data": entries}))).unwrap();
    assert_eq!(
        catalog
            .models()
            .iter()
            .map(AvailableModel::id)
            .collect::<Vec<_>>(),
        [
            "anthropic/opus",
            "openai/gpt-5-a",
            "openai/gpt-5-z",
            "openai/gpt-5-old",
            "google/pro",
            "xai/grok-4",
            "deepseek/pro",
            "meta/pro",
            "mistral/pro",
            "alibaba/pro",
            "Other/pro",
            "other/plain",
            "other/FLASH-pro",
            "other/MINI-flash",
            "other/BETA-opus",
            "other/non-tool-pro",
        ]
    );
}

#[test]
fn every_tier_term_obeys_precedence_and_ascii_case_folding() {
    for term in ["opus", "sonnet", "gpt-5", "o1", "o3", "o4", "pro", "grok-4"] {
        let premium = format!("other/X{term}X");
        let catalog = public_catalog(body(&json!({
            "data": [
                {"id":"anthropic/plain","tags":["tool-use"]},
                {"id":premium,"tags":["tool-use"]}
            ]
        })))
        .unwrap();
        assert_eq!(catalog.models()[0].id(), premium);
    }

    for term in ["haiku", "mini", "lite"] {
        let economy = format!("other/X{}X", term.to_ascii_uppercase());
        let catalog = public_catalog(body(&json!({
            "data": [
                {"id":"other/flash","tags":["tool-use"]},
                {"id":economy,"tags":["tool-use"]},
                {"id":"other/preview","tags":["tool-use"]}
            ]
        })))
        .unwrap();
        assert_eq!(
            catalog
                .models()
                .iter()
                .map(AvailableModel::id)
                .collect::<Vec<_>>(),
            ["other/flash", economy.as_str(), "other/preview"]
        );
    }

    for term in ["preview", "beta"] {
        let late = format!("anthropic/X{}X", term.to_ascii_uppercase());
        let catalog = public_catalog(body(&json!({
            "data": [
                {"id":late,"tags":["tool-use"]},
                {"id":"other/plain","tags":["tool-use"]}
            ]
        })))
        .unwrap();
        assert_eq!(catalog.models()[0].id(), "other/plain");
        assert_eq!(catalog.models()[1].id(), late);
    }
}
