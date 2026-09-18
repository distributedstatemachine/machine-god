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
