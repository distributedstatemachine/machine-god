use super::*;
use crate::{
    McpFeatureAction, NativeInteractivePromptLimits,
    mcp::{
        mrtr::{McpElicitationRequest, McpMrtrLimits},
        protocol::ProtocolVersion,
    },
};
use machine_god_core::{SessionId, SessionIncarnationId};
use serde_json::value::RawValue;

fn owner(id: &str) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new(id).unwrap(),
        SessionIncarnationId::new("incarnation").unwrap(),
    )
}
fn presenter() -> (
    NativeAcpElicitationPresenter,
    crate::NativeInteractivePromptInbox,
) {
    let inbox =
        crate::NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    (NativeAcpElicitationPresenter::new(inbox.router()), inbox)
}
fn request(owner: BackgroundOutputOwner) -> McpElicitationPromptRequest {
    let raw = RawValue::from_string(
        r#"{"mode":"url","message":"Confirm","url":"https://example.test/connect"}"#.into(),
    )
    .unwrap();
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
fn take(presenter: &NativeAcpElicitationPresenter) -> Poll<Option<NativeAcpElicitationComplete>> {
    presenter.poll_complete(&mut Context::from_waker(Waker::noop()))
}

fn submitted(
    presenter: &NativeAcpElicitationPresenter,
    owner: BackgroundOutputOwner,
) -> (McpClientUrlCompletion, NativeAcpElicitationId) {
    let request = request(owner);
    let completion = presenter.register(&request).unwrap();
    let id = presenter.id_for(&request).unwrap();
    presenter.mark_submitted(id).unwrap();
    (completion, id)
}

#[test]
fn parent_retirement_discards_only_its_own_queued_completion() {
    let (presenter, mut inbox) = presenter();
    let parent = inbox.register(owner("parent")).unwrap();
    let _child = inbox.register(owner("child")).unwrap();
    presenter.activate();
    let (parent_completion, _) = submitted(&presenter, owner("parent"));
    let (child_completion, child_id) = submitted(&presenter, owner("child"));
    parent_completion.finish(McpClientUrlOutcome::Completed);
    drop(parent);
    let _replacement = inbox.register(owner("parent")).unwrap();
    presenter.activate();
    assert!(take(&presenter).is_pending());
    child_completion.finish(McpClientUrlOutcome::Completed);
    assert!(
        matches!(take(&presenter), Poll::Ready(Some(event)) if event.id == child_id && event.owner == owner("child"))
    );
    assert!(take(&presenter).is_pending());
    assert!(presenter.state.lock().unwrap().entries.is_empty());
}

#[test]
fn fixed_principal_bridge_cannot_register_a_different_live_owner() {
    let mut inbox =
        crate::NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let parent = inbox.register(owner("parent")).unwrap();
    let _child = inbox.register(owner("child")).unwrap();
    let presenter = NativeAcpElicitationPresenter::new(parent.bridge());
    presenter.activate();
    assert!(presenter.register(&request(owner("child"))).is_err());
    let (completion, _) = submitted(&presenter, owner("parent"));
    drop(parent);
    let _replacement = inbox.register(owner("parent")).unwrap();
    completion.finish(McpClientUrlOutcome::Completed);
    assert!(take(&presenter).is_pending());
    assert!(presenter.register(&request(owner("parent"))).is_err());
}

#[test]
fn retired_unsubmitted_registrations_refund_capacity_without_touching_live_siblings() {
    let (presenter, mut inbox) = presenter();
    let parent = inbox.register(owner("parent")).unwrap();
    let _child = inbox.register(owner("child")).unwrap();
    presenter.activate();
    let abandoned = (0..MAX_REGISTRATIONS)
        .map(|_| presenter.register(&request(owner("parent"))).unwrap())
        .collect::<Vec<_>>();
    assert!(presenter.state.lock().unwrap().bytes > 0);
    assert!(matches!(
        presenter.register(&request(owner("child"))),
        Err(McpElicitationPromptError::Limit)
    ));
    drop(parent);
    let (child, id) = submitted(&presenter, owner("child"));
    assert_eq!(presenter.state.lock().unwrap().bytes, 0);
    assert_eq!(presenter.state.lock().unwrap().entries.len(), 1);
    for old in abandoned {
        old.finish(McpClientUrlOutcome::Completed);
    }
    assert!(take(&presenter).is_pending());
    child.finish(McpClientUrlOutcome::Completed);
    assert!(matches!(take(&presenter), Poll::Ready(Some(event)) if event.id == id));
}

#[test]
fn registration_needs_actual_principal_and_exact_request_allocation() {
    let (presenter, mut inbox) = presenter();
    let request = request(owner("session"));
    assert!(presenter.register(&request).is_err());
    presenter.activate();
    let _other = inbox.register(owner("other")).unwrap();
    assert!(presenter.register(&request).is_err());
    let _principal = inbox.register(owner("session")).unwrap();
    let completion = presenter.register(&request).unwrap();
    let id = presenter.id_for(&request).unwrap();
    assert!(presenter.register(&request).is_err());
    assert!(
        presenter
            .id_for(&super::tests::request(owner("session")))
            .is_err()
    );
    assert!(take(&presenter).is_pending());
    presenter.mark_submitted(id).unwrap();
    assert!(presenter.mark_submitted(id).is_err());
    assert_eq!(presenter.state.lock().unwrap().bytes, 0);
    assert!(take(&presenter).is_pending());
    completion.finish(McpClientUrlOutcome::Completed);
    let Poll::Ready(Some(event)) = take(&presenter) else {
        panic!("completion");
    };
    assert_eq!(event.id, id);
    assert_eq!(event.owner, owner("session"));
    assert!(take(&presenter).is_pending());
}

#[test]
fn never_submitted_or_abandoned_registrations_never_emit_success() {
    for outcome in [
        McpClientUrlOutcome::Completed,
        McpClientUrlOutcome::Unresolved,
        McpClientUrlOutcome::Abandoned,
    ] {
        let (presenter, mut inbox) = presenter();
        let _principal = inbox.register(owner("session")).unwrap();
        presenter.activate();
        let completion = presenter.register(&request(owner("session"))).unwrap();
        completion.finish(outcome);
        assert!(take(&presenter).is_pending());
        assert!(presenter.state.lock().unwrap().entries.is_empty());
    }
    let (presenter, mut inbox) = presenter();
    let _principal = inbox.register(owner("session")).unwrap();
    presenter.activate();
    let request = request(owner("session"));
    let completion = presenter.register(&request).unwrap();
    presenter
        .mark_submitted(presenter.id_for(&request).unwrap())
        .unwrap();
    drop(completion);
    assert!(take(&presenter).is_pending());
    assert!(presenter.state.lock().unwrap().entries.is_empty());
}

#[test]
fn same_session_reactivation_cannot_rebind_old_completion() {
    let (presenter, mut inbox) = presenter();
    let principal = inbox.register(owner("session")).unwrap();
    presenter.activate();
    let old = request(owner("session"));
    let old_completion = presenter.register(&old).unwrap();
    let old_id = presenter.id_for(&old).unwrap();
    presenter.mark_submitted(old_id).unwrap();
    drop(principal);
    let _replacement = inbox.register(owner("session")).unwrap();
    presenter.activate();
    let new = request(owner("session"));
    let new_completion = presenter.register(&new).unwrap();
    let new_id = presenter.id_for(&new).unwrap();
    assert_ne!(old_id, new_id);
    old_completion.finish(McpClientUrlOutcome::Completed);
    assert!(take(&presenter).is_pending());
    presenter.mark_submitted(new_id).unwrap();
    new_completion.finish(McpClientUrlOutcome::Completed);
    assert!(matches!(take(&presenter), Poll::Ready(Some(event)) if event.id == new_id));
}

#[test]
fn queued_completions_share_registration_capacity_and_close_discards_them() {
    let (presenter, mut inbox) = presenter();
    let _principal = inbox.register(owner("session")).unwrap();
    presenter.activate();
    for _ in 0..MAX_REGISTRATIONS {
        let request = request(owner("session"));
        let completion = presenter.register(&request).unwrap();
        presenter
            .mark_submitted(presenter.id_for(&request).unwrap())
            .unwrap();
        completion.finish(McpClientUrlOutcome::Completed);
    }
    assert!(matches!(
        presenter.register(&request(owner("session"))),
        Err(McpElicitationPromptError::Limit)
    ));
    assert!(matches!(take(&presenter), Poll::Ready(Some(_))));
    let completion = presenter.register(&request(owner("session"))).unwrap();
    presenter.deactivate();
    drop(completion);
    assert!(matches!(take(&presenter), Poll::Ready(None)));
    assert_eq!(presenter.state.lock().unwrap().bytes, 0);
    assert!(presenter.state.lock().unwrap().ready.is_empty());
}

#[test]
fn sequence_exhaustion_fails_without_mutating_registration_budget() {
    let (presenter, mut inbox) = presenter();
    let _principal = inbox.register(owner("session")).unwrap();
    presenter.activate();
    presenter.state.lock().unwrap().next_id = u64::MAX;
    assert!(matches!(
        presenter.register(&request(owner("session"))),
        Err(McpElicitationPromptError::Limit)
    ));
    let state = presenter.state.lock().unwrap();
    assert_eq!(state.bytes, 0);
    assert!(state.entries.is_empty());
}

#[test]
fn completion_waker_clone_drop_and_wake_are_outside_the_registry_lock() {
    use machine_god_reentrant_waker_test::{Callback, new};
    for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
        let (presenter, mut inbox) = presenter();
        let _principal = inbox.register(owner("session")).unwrap();
        presenter.activate();
        let state = presenter.state.clone();
        let (waker, calls) = new(callback, move || {
            assert!(state.try_lock().is_ok());
        });
        assert!(
            presenter
                .poll_complete(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert!(
            presenter
                .poll_complete(&mut Context::from_waker(&waker))
                .is_pending()
        );
        let request = request(owner("session"));
        let completion = presenter.register(&request).unwrap();
        presenter
            .mark_submitted(presenter.id_for(&request).unwrap())
            .unwrap();
        completion.finish(McpClientUrlOutcome::Completed);
        assert!(matches!(
            presenter.poll_complete(&mut Context::from_waker(&waker)),
            Poll::Ready(Some(_))
        ));
        drop(waker);
        assert!(calls.calls() > 0);
    }
}
