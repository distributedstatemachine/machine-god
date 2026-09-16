use super::*;
use crate::{
    McpFeatureAction,
    mcp::{
        interaction::{
            McpClientUrlEndpoint, McpClientUrlOutcome, McpElicitationPresenter,
            McpElicitationPromptRequest,
        },
        mrtr::{McpElicitationRequest, McpMrtrLimits},
        protocol::ProtocolVersion,
    },
};
use machine_god_core::{BoxFuture, CancellationToken, SessionId, SessionIncarnationId};
use serde_json::{json, value::RawValue};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Wake, Waker};
mod managed;

fn owner() -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new("session").unwrap(),
        SessionIncarnationId::new("life").unwrap(),
    )
}
fn connection() -> NativeAcpClientRequests {
    let mut connection = NativeAcpClientRequests::new().unwrap();
    connection
        .activate(owner(), Arc::new(NativePermissionContexts::new()))
        .unwrap();
    connection
}
fn request(url: bool) -> McpElicitationPromptRequest {
    request_for(owner(), url)
}
fn request_for(owner: BackgroundOutputOwner, url: bool) -> McpElicitationPromptRequest {
    let text = if url {
        r#"{"mode":"url","message":"Confirm","url":"https://example.test/connect"}"#
    } else {
        r#"{"mode":"form","message":"Confirm","requestedSchema":{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]}}"#
    };
    let raw = RawValue::from_string(text.into()).unwrap();
    McpElicitationPromptRequest::new_human_feature(
        owner,
        Arc::from("server"),
        McpFeatureAction::ResourceRead,
        Arc::new(
            McpElicitationRequest::parse(&raw, ProtocolVersion::Modern, McpMrtrLimits::default())
                .unwrap(),
        ),
    )
    .unwrap()
}
fn poll<T>(future: &mut BoxFuture<'_, T>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}
fn frame(connection: &mut NativeAcpClientRequests) -> AcpMessage {
    let Poll::Ready(Ok(Some(bytes))) =
        connection.poll_request(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("request frame");
    };
    protocol::decode_frame(&bytes).unwrap()
}
fn id(message: AcpMessage) -> AcpId {
    let AcpMessage::Request { id, .. } = message else {
        panic!("request");
    };
    id
}

#[test]
fn native_form_has_one_rpc_and_exact_answer_custody() {
    let mut connection = connection();
    let presenter = connection.presenter();
    let mut pending = presenter.present(request(false), CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let id = id(frame(&mut connection));
    assert!(
        connection
            .poll_request(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(
        connection.reply(&AcpId::Integer(123), Ok(json!({"action":"cancel"}))),
        Err(NativeAcpClientRequestError::Stale)
    );
    assert!(poll(&mut pending).is_pending());
    connection
        .reply(
            &id,
            Ok(json!({"action":"accept","content":{"answer":"yes"}})),
        )
        .unwrap();
    assert!(matches!(poll(&mut pending), Poll::Ready(Ok(_))));
    assert_eq!(
        connection.reply(&id, Ok(json!({"action":"cancel"}))),
        Err(NativeAcpClientRequestError::Stale)
    );
}

#[test]
fn permission_without_exact_native_call_context_is_not_sent_or_invented() {
    use crate::{PermissionPromptDecision, PermissionPrompter};
    use machine_god_core::{
        Capability, PermissionRequest, PermissionRequestId, PermissionRisk, TurnId,
    };
    let mut connection = connection();
    let bridge = connection.bridge();
    let mut pending = bridge.prompt(PermissionRequest {
        id: PermissionRequestId::new("not-a-call-id").unwrap(),
        session_id: owner().session_id().clone(),
        session_incarnation_id: owner().session_incarnation_id().clone(),
        turn_id: TurnId::new("turn").unwrap(),
        capability: Capability::Custom {
            name: "private".into(),
            details: Value::Null,
        },
        risk: PermissionRisk::Low,
        reason: "private".into(),
    });
    assert!(poll(&mut pending).is_pending());
    assert_eq!(
        connection.poll_request(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(NativeAcpClientRequestError::Stale))
    );
    assert_eq!(
        poll(&mut pending),
        Poll::Ready(Ok(PermissionPromptDecision::Deny))
    );
    assert!(connection.ids.is_empty());
}

#[test]
fn malformed_and_remote_error_replies_cancel_without_wedging_next_request() {
    let mut connection = connection();
    let presenter = connection.presenter();
    for answer in [
        Ok(json!({"action":"accept","content":{"answer":12}})),
        Err(AcpRpcError {
            code: -1,
            message: "private".into(),
            data: None,
        }),
    ] {
        let mut pending = presenter.present(request(false), CancellationToken::new());
        assert!(poll(&mut pending).is_pending());
        let id = id(frame(&mut connection));
        assert_eq!(
            connection.reply(&id, answer),
            Err(NativeAcpClientRequestError::InvalidResponse)
        );
        assert!(matches!(poll(&mut pending), Poll::Ready(Ok(_))));
        assert!(connection.pending.is_none());
        assert!(connection.ids.is_empty());
    }
}

#[test]
fn same_principal_reactivation_invalidates_old_response_and_never_reuses_ids() {
    let mut connection = connection();
    let presenter = connection.presenter();
    let mut first = presenter.present(request(false), CancellationToken::new());
    assert!(poll(&mut first).is_pending());
    let first_id = id(frame(&mut connection));
    connection
        .activate(owner(), Arc::new(NativePermissionContexts::new()))
        .unwrap();
    assert!(matches!(poll(&mut first), Poll::Ready(Err(_))));
    let mut second = presenter.present(request(false), CancellationToken::new());
    assert!(poll(&mut second).is_pending());
    let second_id = id(frame(&mut connection));
    assert_ne!(first_id, second_id);
    assert_eq!(
        connection.reply(&first_id, Ok(json!({"action":"cancel"}))),
        Err(NativeAcpClientRequestError::Stale)
    );
    assert!(poll(&mut second).is_pending());
    connection
        .reply(&second_id, Ok(json!({"action":"cancel"})))
        .unwrap();
    assert!(matches!(poll(&mut second), Poll::Ready(Ok(_))));
}

struct CountWake(AtomicUsize);
impl Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn dropped_native_waiter_wakes_connection_and_releases_stale_correlation() {
    let mut connection = connection();
    let presenter = connection.presenter();
    let mut pending = presenter.present(request(false), CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let old_id = id(frame(&mut connection));
    let wake = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wake));
    assert!(
        connection
            .poll_request(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(pending);
    assert!(wake.0.load(Ordering::Relaxed) > 0);
    let mut next = presenter.present(request(false), CancellationToken::new());
    assert!(poll(&mut next).is_pending());
    let new_id = id(frame(&mut connection));
    assert_ne!(old_id, new_id);
    assert_eq!(connection.ids.len(), 1);
}

#[test]
fn close_invalidates_accepted_but_unconsumed_response_and_is_terminal() {
    let mut connection = connection();
    let presenter = connection.presenter();
    let mut pending = presenter.present(request(false), CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let id = id(frame(&mut connection));
    connection
        .reply(
            &id,
            Ok(json!({"action":"accept","content":{"answer":"yes"}})),
        )
        .unwrap();
    connection.close();
    assert!(matches!(poll(&mut pending), Poll::Ready(Err(_))));
    assert_eq!(
        connection.activate(owner(), Arc::new(NativePermissionContexts::new())),
        Err(NativeAcpClientRequestError::Closed)
    );
    assert!(connection.ids.is_empty());
}

#[test]
fn modern_url_completion_requires_native_terminal_observation_not_answer() {
    let mut connection = connection();
    let presenter = connection.presenter();
    let request = request(true);
    let complete = presenter.register(&request).unwrap();
    let mut pending = presenter.present(request, CancellationToken::new());
    assert!(poll(&mut pending).is_pending());
    let message = frame(&mut connection);
    let AcpMessage::Request {
        id,
        params: Some(params),
        ..
    } = message
    else {
        panic!("request");
    };
    let url_id = params["elicitationId"].clone();
    connection
        .reply(&id, Ok(json!({"action":"accept"})))
        .unwrap();
    assert!(matches!(poll(&mut pending), Poll::Ready(Ok(_))));
    assert!(
        connection
            .poll_complete(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    complete.finish(McpClientUrlOutcome::Completed);
    let Poll::Ready(Ok(Some(bytes))) =
        connection.poll_complete(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("completion");
    };
    let AcpMessage::Notification {
        method,
        params: Some(params),
    } = protocol::decode_frame(&bytes).unwrap()
    else {
        panic!("notification");
    };
    assert_eq!(method, "elicitation/complete");
    assert_eq!(params["elicitationId"], url_id);
    assert_eq!(params["sessionId"], "session");
    assert!(
        connection
            .poll_complete(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

#[test]
fn abandoned_or_replaced_url_registration_never_emits_success() {
    for replace in [false, true] {
        let mut connection = connection();
        let presenter = connection.presenter();
        let request = request(true);
        let complete = presenter.register(&request).unwrap();
        let mut pending = presenter.present(request, CancellationToken::new());
        assert!(poll(&mut pending).is_pending());
        let _ = frame(&mut connection);
        if replace {
            connection
                .activate(owner(), Arc::new(NativePermissionContexts::new()))
                .unwrap();
            complete.finish(McpClientUrlOutcome::Completed);
        } else {
            drop(complete);
        }
        assert!(
            connection
                .poll_complete(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
}
