//! Real core invocation -> claimed mailbox job -> exact consent -> ACP -> journal.
use super::*;
use crate::acp::{
    client_requests::{NativeAcpClientRequestError, NativeAcpClientRequests},
    protocol::{self, AcpId, AcpMessage},
};
use crate::{NativePermissionContexts, PermissionPrompter};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, ManagedSubagentError,
    ManagedSubagentResult,
};
use serde_json::{Value, json};

type Response = BoxFuture<'static, Result<ManagedSubagentResult, ManagedSubagentError>>;

fn fixture() -> Fixture {
    let mut f = Fixture::new(vec![]);
    for name in ["child", "new-parent"] {
        assert!(
            f.command(json!({"create":{"name":name,"mode":"persistent"}}))
                .ok
        );
    }
    f.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    f
}

fn start(
    f: &mut Fixture,
    connection: &mut NativeAcpClientRequests,
) -> (
    fixture::Admission,
    CancellationToken,
    Response,
    AcpId,
    Value,
) {
    let (admission, invocation) = f.invocation(json!({
        "relationship":{"id":"child-1","action":"reparent","parent_id":"child-2"}
    }));
    let owner = BackgroundOutputOwner::new(
        invocation.context().session_id.clone(),
        invocation.context().session_incarnation_id.clone(),
    );
    let admitted_arguments = invocation.arguments().clone();
    // Intentionally no ordinary permission-review snapshot. Core has already
    // completed that scope and delivered the actual admitted invocation.
    connection
        .activate(owner, Arc::new(NativePermissionContexts::new()))
        .unwrap();
    f.manager.authorizer = Arc::new(crate::reference_host::RelationshipConsent::new(
        f.factory.registry.requester(),
        connection.bridge(),
    ));
    let cancellation = CancellationToken::new();
    let token = cancellation.clone();
    let requester = f.requester.clone();
    let mut response: Response =
        Box::pin(async move { requester.execute(invocation, token).await });
    let message = block_on(std::future::poll_fn(|cx| {
        assert!(response.as_mut().poll(cx).is_pending());
        let progress = f.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        match connection.poll_request(cx) {
            Poll::Ready(Ok(Some(bytes))) => Poll::Ready(protocol::decode_frame(&bytes).unwrap()),
            Poll::Ready(other) => {
                panic!("execution consent must not fail the ACP connection: {other:?}")
            }
            Poll::Pending => {
                if progress.is_ready() {
                    cx.waker().wake_by_ref();
                }
                Poll::Pending
            }
        }
    }));
    let AcpMessage::Request {
        id,
        method,
        params: Some(params),
    } = message
    else {
        panic!("consent RPC");
    };
    assert_eq!(method, "session/request_permission");
    assert_eq!(params["toolCall"]["toolCallId"], "operation");
    assert_eq!(params["toolCall"]["rawInput"], admitted_arguments);
    assert_eq!(
        params["toolCall"]["rawInput"]["command"]["relationship"]["id"],
        "child-1"
    );
    assert_eq!(params["options"].as_array().unwrap().len(), 2);
    assert_eq!(params["options"][0]["optionId"], "allow_once");
    assert_eq!(params["options"][1]["optionId"], "reject_once");
    let detail: Value = serde_json::from_str(
        params["toolCall"]["content"][0]["content"]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(detail["details"]["child_id"], "child-1");
    assert_eq!(detail["details"]["child_generation"], 1);
    assert_eq!(detail["details"]["previous_parent"]["session_id"], "parent");
    assert_eq!(detail["details"]["parent"]["session_id"], "child-2");
    (admission, cancellation, response, id, params)
}

fn finish(
    f: &mut Fixture,
    response: &mut Response,
) -> Result<ManagedSubagentResult, ManagedSubagentError> {
    let result = block_on(std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result);
        }
        let progress = f.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    f.drive(|f| {
        f.manager.active.is_none() && f.manager.replay.done && f.manager.approvals.is_empty()
    });
    result
}

fn selected(option: &str) -> Value {
    json!({"outcome":{"outcome":"selected","optionId":option}})
}

#[test]
fn active_model_relationship_acp_approve_deny_cancel_are_exact_one_shot() {
    for answer in [
        selected("allow_once"),
        selected("reject_once"),
        json!({"outcome":{"outcome":"cancelled"}}),
    ] {
        let approve = answer == selected("allow_once");
        let mut f = fixture();
        let mut connection = NativeAcpClientRequests::new().unwrap();
        let (_admission, _cancellation, mut response, id, _) = start(&mut f, &mut connection);
        assert_eq!(
            connection.reply(&AcpId::Integer(999), Ok(answer.clone())),
            Err(NativeAcpClientRequestError::Stale)
        );
        connection.reply(&id, Ok(answer.clone())).unwrap();
        let result = finish(&mut f, &mut response).unwrap();
        assert_eq!(result.ok, approve);
        let child = block_on(f.journal.inspect("child-1".into())).unwrap();
        assert_eq!(
            child.head.parent_id.as_deref(),
            Some(if approve { "child-2" } else { "parent" })
        );
        assert_eq!(
            connection.reply(&id, Ok(answer)),
            Err(NativeAcpClientRequestError::Stale)
        );
        assert!(
            connection
                .poll_request(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
}

#[test]
fn active_model_relationship_acp_rejects_broad_grants() {
    let mut f = fixture();
    let mut connection = NativeAcpClientRequests::new().unwrap();
    let (_admission, _cancel, mut response, id, _) = start(&mut f, &mut connection);
    assert_eq!(
        connection.reply(&id, Ok(selected("allow_always"))),
        Err(NativeAcpClientRequestError::InvalidResponse)
    );
    assert!(!finish(&mut f, &mut response).unwrap().ok);
    assert_eq!(
        block_on(f.journal.inspect("child-1".into()))
            .unwrap()
            .head
            .parent_id
            .as_deref(),
        Some("parent")
    );
}

#[test]
fn active_model_relationship_cancel_or_retirement_invalidates_accepted_consent() {
    for retire in [false, true] {
        let mut f = fixture();
        let mut connection = NativeAcpClientRequests::new().unwrap();
        let (admission, cancellation, mut response, id, _) = start(&mut f, &mut connection);
        connection.reply(&id, Ok(selected("allow_once"))).unwrap();
        // Revoke after the reply is accepted but before authorizer consumption.
        if retire {
            drop(admission);
        } else {
            cancellation.cancel();
        }
        let result = finish(&mut f, &mut response);
        assert!(result.is_err() || result.is_ok_and(|result| !result.ok));
        assert_eq!(
            block_on(f.journal.inspect("child-1".into()))
                .unwrap()
                .head
                .parent_id
                .as_deref(),
            Some("parent")
        );
        assert!(
            connection
                .poll_request(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
}

struct Headless;
impl PermissionPrompter for Headless {
    fn prompt(
        &self,
        _: machine_god_core::PermissionRequest,
    ) -> BoxFuture<'_, Result<crate::PermissionPromptDecision, crate::PermissionPromptError>> {
        panic!("ordinary permission is not replacement consent");
    }
}

#[test]
fn headless_model_relationship_consent_fails_closed() {
    let mut f = fixture();
    f.manager.authorizer = Arc::new(crate::reference_host::RelationshipConsent::new(
        f.factory.registry.requester(),
        Arc::new(Headless),
    ));
    assert!(
        !f.command(
            json!({"relationship":{"id":"child-1","action":"reparent","parent_id":"child-2"}})
        )
        .ok
    );
}

#[test]
fn admitted_job_consent_is_nonreusable_and_inbox_responses_are_distinct() {
    use crate::{
        NativeInteractivePromptError, NativeInteractivePromptInbox, NativeInteractivePromptLimits,
        NativeInteractivePromptResponse, PermissionPromptDecision,
    };
    use machine_god_core::{Capability, PermissionRequest, PermissionRequestId, PermissionRisk};
    let f = Fixture::new(vec![]);
    let (_admission, invocation) = f.invocation(
        json!({"relationship":{"id":"child-1","action":"reparent","parent_id":"parent"}}),
    );
    let context = invocation.context().clone();
    let cancellation = CancellationToken::new();
    let requester = f.requester.clone();
    let mut response = requester.execute(invocation, cancellation);
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    let Poll::Ready(Some(job)) = f.manager.mailbox.poll_next(&mut cx) else {
        panic!("claimed job");
    };
    let source = job.execution_consent().unwrap();
    assert!(job.execution_consent().is_none());
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let principal = inbox
        .register(BackgroundOutputOwner::new(
            context.session_id.clone(),
            context.session_incarnation_id.clone(),
        ))
        .unwrap();
    let bridge = principal.bridge();
    let request = source.request(
        Capability::Custom {
            name: "managed_relationship".into(),
            details: json!({"child_revision":7}),
        },
        "Exact proposal".into(),
    );
    let mut pending = bridge.prompt_execution_consent(request);
    assert!(pending.as_mut().poll(&mut cx).is_pending());
    let Poll::Ready(Some(view)) = inbox.poll_prompt(&mut cx) else {
        panic!("consent view");
    };
    assert!(view.permission().is_none());
    assert!(!view.can_save_rule());
    assert_eq!(
        inbox.reply(
            view.token(),
            NativeInteractivePromptResponse::Permission(PermissionPromptDecision::AllowOnce)
        ),
        Err(NativeInteractivePromptError::InvalidResponse)
    );
    // Retaining the immutable view does not keep the admitted job/lease alive.
    drop(job);
    assert_eq!(
        inbox.reply(
            view.token(),
            NativeInteractivePromptResponse::ExecutionConsent(true)
        ),
        Err(NativeInteractivePromptError::Stale)
    );
    inbox.cancel(view.token()).unwrap();
    assert!(matches!(
        pending.as_mut().poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    let mut permission = bridge.prompt(PermissionRequest {
        id: PermissionRequestId::new("permission").unwrap(),
        session_id: context.session_id,
        session_incarnation_id: context.session_incarnation_id,
        turn_id: context.turn_id,
        capability: Capability::Custom {
            name: "ordinary".into(),
            details: Value::Null,
        },
        risk: PermissionRisk::Low,
        reason: "Ordinary permission".into(),
    });
    assert!(permission.as_mut().poll(&mut cx).is_pending());
    let Poll::Ready(Some(view)) = inbox.poll_prompt(&mut cx) else {
        panic!("permission view");
    };
    assert_eq!(
        inbox.reply(
            view.token(),
            NativeInteractivePromptResponse::ExecutionConsent(true)
        ),
        Err(NativeInteractivePromptError::InvalidResponse)
    );
    inbox.cancel(view.token()).unwrap();
    assert_eq!(
        permission.as_mut().poll(&mut cx),
        Poll::Ready(Ok(PermissionPromptDecision::Deny))
    );
}

#[test]
fn revoked_queued_execution_consent_is_not_an_acp_connection_failure() {
    use machine_god_core::Capability;
    for cancel in [false, true] {
        let f = Fixture::new(vec![]);
        let (_admission, invocation) = f.invocation(json!({
            "relationship":{"id":"child-1","action":"reparent","parent_id":"parent"}
        }));
        let context = invocation.context().clone();
        let cancellation = CancellationToken::new();
        let requester = f.requester.clone();
        let mut response = requester.execute(invocation, cancellation.clone());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(response.as_mut().poll(&mut cx).is_pending());
        let Poll::Ready(Some(job)) = f.manager.mailbox.poll_next(&mut cx) else {
            panic!("claimed job");
        };
        let request = job.execution_consent().unwrap().request(
            Capability::Custom {
                name: "managed_relationship".into(),
                details: json!({"child_revision":7}),
            },
            "Exact proposal".into(),
        );
        let mut connection = NativeAcpClientRequests::new().unwrap();
        connection
            .activate(
                BackgroundOutputOwner::new(context.session_id, context.session_incarnation_id),
                Arc::new(NativePermissionContexts::new()),
            )
            .unwrap();
        let bridge = connection.bridge();
        let mut pending = bridge.prompt_execution_consent(request);
        assert!(pending.as_mut().poll(&mut cx).is_pending());
        if cancel {
            cancellation.cancel();
        } else {
            drop(response);
        }
        // Queued but not projected: do not emit a stale proposal or terminate
        // the whole connection. The job still exists, so observer loss itself
        // must invalidate its weak consent witness.
        assert!(connection.poll_request(&mut cx).is_pending());
        assert!(matches!(
            pending.as_mut().poll(&mut cx),
            Poll::Ready(Err(_))
        ));
        assert!(connection.poll_request(&mut cx).is_pending());
        drop(job);
    }
}

#[test]
fn admitted_execution_consent_cannot_borrow_a_foreign_principal_inbox() {
    use machine_god_core::{Capability, SessionId, SessionIncarnationId};
    let f = Fixture::new(vec![]);
    let (_admission, invocation) = f.invocation(json!({
        "relationship":{"id":"child-1","action":"reparent","parent_id":"parent"}
    }));
    let requester = f.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    let Poll::Ready(Some(job)) = f.manager.mailbox.poll_next(&mut cx) else {
        panic!("claimed job");
    };
    let mut connection = NativeAcpClientRequests::new().unwrap();
    connection
        .activate(
            BackgroundOutputOwner::new(
                SessionId::new("foreign").unwrap(),
                SessionIncarnationId::new("foreign-life").unwrap(),
            ),
            Arc::new(NativePermissionContexts::new()),
        )
        .unwrap();
    let request = job.execution_consent().unwrap().request(
        Capability::Custom {
            name: "managed_relationship".into(),
            details: json!({"child_revision":7}),
        },
        "Exact proposal".into(),
    );
    let bridge = connection.bridge();
    let mut pending = bridge.prompt_execution_consent(request);
    assert!(matches!(
        pending.as_mut().poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    assert!(connection.poll_request(&mut cx).is_pending());
}
