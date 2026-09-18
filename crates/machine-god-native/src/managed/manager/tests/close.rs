use super::Fixture;
use futures_executor::block_on;
use machine_god_core::{CancellationToken, ManagedAgentState, ManagedSubagentAuthority};
use std::sync::atomic::Ordering;
use std::task::{Context, Poll, Waker};

#[test]
fn ordinary_close_waits_for_retired_resources_before_success() {
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "worker", "mode": "persistent"
            }}))
            .ok
    );
    fixture.factory.cleanup.store(false, Ordering::Release);
    let (_admission, invocation) = fixture.invocation(serde_json::json!({"lifecycle": {
        "id": "child-1", "action": "close"
    }}));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.len() == 1);
    let early = response.as_mut().poll(&mut cx);
    assert_eq!(
        block_on(fixture.journal.inspect("child-1".into()))
            .unwrap()
            .head
            .status,
        ManagedAgentState::Archived
    );
    // Always release fixture-owned cleanup, including on the broken path.
    fixture.factory.cleanup.store(true, Ordering::Release);
    fixture.drive(|f| f.manager.retiring.is_empty());
    let (reported_early, result) = match early {
        Poll::Ready(result) => (true, result),
        Poll::Pending => (false, block_on(response)),
    };
    assert!(result.unwrap().ok);
    assert!(
        !reported_early,
        "ordinary close reported success before actual resource closure"
    );
    assert!(fixture.factory.provider.requests().is_empty());
}

#[test]
fn close_preserves_original_observer_and_archive_across_cleanup_error() {
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "worker", "mode": "persistent"
            }}))
            .ok
    );
    fixture.factory.cleanup.store(false, Ordering::Release);
    let (_admission, invocation) = fixture.invocation(serde_json::json!({"lifecycle": {
        "id": "child-1", "action": "close"
    }}));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.len() == 1);
    fixture.factory.close_error.store(true, Ordering::Release);
    assert!(matches!(
        fixture.manager.poll_progress(&mut cx, 100),
        Poll::Ready(Err(super::ManagedRuntimeError::Unavailable))
    ));
    assert!(response.as_mut().poll(&mut cx).is_pending());
    assert!(fixture.manager.retiring[0].completion.is_some());
    assert_eq!(
        block_on(fixture.journal.inspect("child-1".into()))
            .unwrap()
            .head
            .status,
        ManagedAgentState::Archived
    );
    fixture.factory.close_error.store(false, Ordering::Release);
    fixture.factory.cleanup.store(true, Ordering::Release);
    fixture.drive(|f| f.manager.retiring.is_empty());
    assert!(block_on(response).unwrap().ok);
}

#[test]
fn shutdown_rejects_close_observer_but_retains_actual_cleanup_custody() {
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "worker", "mode": "persistent"
            }}))
            .ok
    );
    fixture.factory.cleanup.store(false, Ordering::Release);
    let (_admission, invocation) = fixture.invocation(serde_json::json!({"lifecycle": {
        "id": "child-1", "action": "close"
    }}));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.len() == 1);
    assert!(fixture.manager.poll_shutdown(&mut cx, 100).is_pending());
    assert!(matches!(
        response.as_mut().poll(&mut cx),
        Poll::Ready(Err(machine_god_core::ManagedSubagentError::Unavailable))
    ));
    // Mailbox closure rejects observation, not the already-accepted job or
    // its resource ownership. In particular it never reports close success.
    assert!(fixture.manager.retiring[0].completion.is_some());
    fixture.factory.cleanup.store(true, Ordering::Release);
    block_on(std::future::poll_fn(|cx| {
        fixture.manager.poll_shutdown(cx, 101)
    }))
    .unwrap();
    assert!(fixture.manager.retiring.is_empty());
    assert_eq!(
        block_on(fixture.journal.inspect("child-1".into()))
            .unwrap()
            .head
            .status,
        ManagedAgentState::Archived
    );
}

#[test]
fn abandoned_close_observer_does_not_release_actual_cleanup_custody() {
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "worker", "mode": "persistent"
            }}))
            .ok
    );
    fixture.factory.cleanup.store(false, Ordering::Release);
    let (_admission, invocation) = fixture.invocation(serde_json::json!({"lifecycle": {
        "id": "child-1", "action": "close"
    }}));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    let mut cx = Context::from_waker(Waker::noop());
    assert!(response.as_mut().poll(&mut cx).is_pending());
    fixture.drive(|f| f.manager.children.is_empty() && f.manager.retiring.len() == 1);
    drop(response);
    assert!(fixture.manager.poll_shutdown(&mut cx, 100).is_pending());
    assert_eq!(fixture.manager.retiring.len(), 1);
    fixture.factory.cleanup.store(true, Ordering::Release);
    block_on(std::future::poll_fn(|cx| {
        fixture.manager.poll_shutdown(cx, 101)
    }))
    .unwrap();
    assert!(fixture.manager.retiring.is_empty());
    assert_eq!(
        block_on(fixture.journal.inspect("child-1".into()))
            .unwrap()
            .head
            .status,
        ManagedAgentState::Archived
    );
}
