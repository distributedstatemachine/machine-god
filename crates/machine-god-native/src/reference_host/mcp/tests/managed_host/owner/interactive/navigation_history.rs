use super::human::{close, command, create, open, submit};
use super::navigation::page;
use super::*;
use crate::{
    NativeManagedCatalogFilter as Filter, NativeManagedHistoryError as Error,
    NativeManagedHistoryOutcome,
};

async fn history(owner: &mut NativeInteractiveSession) -> NativeManagedHistoryOutcome {
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 20);
        assert!(owner.managed_error().is_none());
        assert!(owner.shutdown_error().is_none());
        if let Some(outcome) = owner.take_managed_history_outcome() {
            return Poll::Ready(outcome);
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await
}

#[test]
fn canonical_child_history_preserves_full_unicode_and_reads_archives_without_residency() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let text = "α🙂 child history\n".repeat(2048);
    fixture
        .transport
        .responses
        .lock()
        .unwrap()
        .push_back(answer());
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        assert!(
            submit(
                &mut owner,
                command(serde_json::json!({
                    "message":{"send":{"id":child,"content":text}}
                }))
            )
            .await
            .ok
        );
        poll_fn(|cx| {
            let progress = owner.poll_progress(cx, 20);
            assert!(owner.managed_error().is_none());
            if !fixture.transport.requests.lock().unwrap().is_empty()
                && owner
                    .managed_agents()
                    .iter()
                    .all(|child| child.state == machine_god_core::ManagedAgentState::Idle)
            {
                return Poll::Ready(());
            }
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        })
        .await;
        let observed = page(&mut owner, Filter::Current, None, 64)
            .await
            .entries
            .remove(0)
            .observation;
        let request = owner.request_managed_history(observed.clone()).unwrap();
        assert_eq!(
            owner.request_managed_history(observed.clone()),
            Err(Error::Busy)
        );
        let outcome = history(&mut owner).await;
        assert_eq!(outcome.request, request);
        let snapshot = outcome.result.unwrap();
        assert!(snapshot.record().messages.iter().any(|message| message.role == machine_god_core::Role::User
            && message.content.iter().any(|block| matches!(block, machine_god_core::ContentBlock::Text { text: value } if value == &text))));
        assert!(!format!("{snapshot:?}").contains("child history"));
        assert_eq!(owner.request_managed_history(observed), Err(Error::Busy));
        let transcript = snapshot.record().clone();
        drop(snapshot);
        assert!(
            submit(
                &mut owner,
                command(serde_json::json!({"lifecycle":{"id":child,"action":"close"}}))
            )
            .await
            .ok
        );
        assert!(owner.managed_agents().is_empty());
        let observed = page(&mut owner, Filter::Archived, None, 64)
            .await
            .entries
            .remove(0)
            .observation;
        owner.request_managed_history(observed).unwrap();
        let archived = history(&mut owner).await.result.unwrap();
        assert_eq!(archived.record().messages, transcript.messages);
        assert!(owner.managed_agents().is_empty());
        close(owner, completion).await;
        // Retaining full immutable history cannot retain the host or journal lease.
        assert_eq!(archived.record().id, transcript.id);
    });
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn history_rejects_foreign_and_changed_heads_and_retains_cancelled_request_identity() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let mut foreign = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let (mut other, other_completion) = open(&mut foreign).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        let observed = page(&mut owner, Filter::Current, None, 64)
            .await
            .entries
            .remove(0)
            .observation;
        assert_eq!(
            other.request_managed_history(observed.clone()),
            Err(Error::Stale)
        );
        let request = owner.request_managed_history(observed.clone()).unwrap();
        assert!(owner.cancel_managed_history(&request));
        let result = history(&mut owner).await;
        assert_eq!(result.request, request);
        assert!(matches!(result.result, Err(Error::Cancelled)));
        assert!(!owner.cancel_managed_history(&request));
        assert!(
            submit(
                &mut owner,
                command(serde_json::json!({"configure":{"id":child,"name":"renamed"}}))
            )
            .await
            .ok
        );
        owner.request_managed_history(observed).unwrap();
        assert!(matches!(
            history(&mut owner).await.result,
            Err(Error::Stale)
        ));
        let current = page(&mut owner, Filter::Current, None, 64)
            .await
            .entries
            .remove(0)
            .observation;
        owner.request_managed_history(current).unwrap();
        // Shutdown must drain an accepted unpolled read, not abandon its slot.
        close(owner, completion).await;
        close(other, other_completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
    assert!(foreign.transport.requests.lock().unwrap().is_empty());
}
