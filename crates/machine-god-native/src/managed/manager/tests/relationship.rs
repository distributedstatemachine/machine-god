use super::*;
use factory::{ManagedRelationshipAuthorizer, ManagedRelationshipProposal};
use machine_god_core::{BoxFuture, CancellationToken};
use std::sync::atomic::AtomicBool;

struct HeldConsent(Arc<AtomicBool>);

impl ManagedRelationshipAuthorizer for HeldConsent {
    fn authorize(
        &self,
        _: ManagedRelationshipProposal,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<bool, ManagedRuntimeError>> {
        let ready = self.0.clone();
        Box::pin(std::future::poll_fn(move |_| {
            if ready.load(Ordering::Acquire) {
                Poll::Ready(Ok(true))
            } else {
                Poll::Pending
            }
        }))
    }
}

#[test]
fn relationship_consent_rechecks_parent_after_archive() {
    check_parent_after_consent(true);
}

#[test]
fn relationship_consent_publishes_when_parent_remains_eligible() {
    check_parent_after_consent(false);
}

fn check_parent_after_consent(archive: bool) {
    let mut fixture = Fixture::new(vec![]);
    for name in ["child", "proposed-parent"] {
        assert!(
            fixture
                .command(serde_json::json!({
                    "create": {"name": name, "mode": "persistent"}
                }))
                .ok
        );
    }
    let consent = Arc::new(AtomicBool::new(false));
    fixture.manager.authorizer = Arc::new(HeldConsent(consent.clone()));
    let (_admission, invocation) = fixture.invocation(serde_json::json!({
        "relationship": {"id": "child-1", "action": "reparent", "parent_id": "child-2"}
    }));
    let requester = fixture.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    assert!(
        response
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    fixture.drive(|f| f.manager.approvals.len() == 1);
    if archive {
        assert!(
            fixture
                .command(serde_json::json!({
                    "lifecycle": {"id": "child-2", "action": "close"}
                }))
                .ok
        );
        fixture.drive(|f| {
            f.manager.active.is_none()
                && f.manager.replay.done
                && f.manager
                    .children
                    .iter()
                    .all(|c| c.snapshot.head.id != "child-2")
        });
        assert_eq!(
            block_on(fixture.journal.inspect("child-2".into()))
                .unwrap()
                .head
                .status,
            machine_god_core::ManagedAgentState::Archived
        );
    }
    consent.store(true, Ordering::Release);
    let result = block_on(std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result.unwrap());
        }
        let progress = fixture.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    // The command receipt can precede notice replay. Settle its journal worker
    // before this out-of-band assertion competes for the single operation slot.
    fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    let child = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    if archive {
        assert_eq!(
            result.error_code,
            Some(ManagedFailureCode::PermissionDenied)
        );
        assert!(!result.ok);
        assert_eq!(child.head.parent_id.as_deref(), Some("parent"));
    } else {
        assert!(result.ok);
        assert_eq!(child.head.parent_id.as_deref(), Some("child-2"));
    }
}
