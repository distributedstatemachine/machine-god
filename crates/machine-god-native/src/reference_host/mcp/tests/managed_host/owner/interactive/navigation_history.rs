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

async fn settle_child(owner: &mut NativeInteractiveSession, fixture: &Fixture) {
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
}

async fn select_first(owner: &mut NativeInteractiveSession) {
    owner.open_managed_navigation().unwrap();
    let frame = super::navigation_ui::ready(owner).await;
    owner.acknowledge_managed_frame(&frame).unwrap();
    owner
        .act_on_managed_frame(&frame, crate::NativeManagedNavigationAction::Select)
        .unwrap();
}

#[test]
fn native_history_navigation_owns_reads_and_restores_only_valid_source_positions() {
    use super::navigation_ui::ready;
    use crate::{
        NativeManagedHistoryPosition as Position, NativeManagedNavigationAction as Action,
        NativeManagedNavigationError as NavigationError,
    };
    let mut fixture = Fixture::with_options("auto", true, options);
    fixture
        .transport
        .responses
        .lock()
        .unwrap()
        .push_back(answer());
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        submit(
            &mut owner,
            command(serde_json::json!({
                "message":{"send":{"id":child,"content":"α🙂\nlong child history"}}
            })),
        )
        .await;
        settle_child(&mut owner, &fixture).await;
        owner.open_managed_navigation().unwrap();
        let frame = ready(&mut owner).await;
        let observed = owner
            .managed_navigation()
            .unwrap()
            .target
            .unwrap()
            .observation
            .clone();
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner.act_on_managed_frame(&frame, Action::Select).unwrap();
        assert!(owner.take_managed_history_outcome().is_none());
        assert_eq!(owner.request_managed_history(observed), Err(Error::Busy));
        let frame = ready(&mut owner).await;
        let view = owner.managed_navigation().unwrap();
        let record = view.history.unwrap().record;
        let message = record
            .messages
            .iter()
            .position(|message| message.role == machine_god_core::Role::User)
            .unwrap();
        let position = Position {
            message,
            block: Some(0),
            byte: 2,
        };
        let editor = view.editor;
        owner.acknowledge_managed_frame(&frame).unwrap();
        assert_eq!(
            owner.act_on_managed_frame(
                &frame,
                Action::SeekHistory(Some(Position {
                    byte: 1,
                    ..position
                }))
            ),
            Err(NavigationError::InvalidAction)
        );
        let frame = ready(&mut owner).await;
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner
            .act_on_managed_frame(&frame, Action::SeekHistory(Some(position)))
            .unwrap();
        assert_eq!(owner.managed_navigation().unwrap().editor, editor);
        assert_eq!(
            owner
                .managed_navigation()
                .unwrap()
                .history
                .unwrap()
                .position,
            Some(position)
        );
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::SeekHistory(None)),
            Err(NavigationError::StaleFrame)
        );
        owner.close_managed_navigation();
        submit(
            &mut owner,
            command(serde_json::json!({"configure":{"id":child,"name":"renamed"}})),
        )
        .await;
        select_first(&mut owner).await;
        ready(&mut owner).await;
        let view = owner.managed_navigation().unwrap();
        assert_ne!(view.editor, editor);
        assert_eq!(view.history.unwrap().position, Some(position));
        owner.close_managed_navigation();
        // An accepted read is cancelled on close, but its owner keeps draining
        // the original request before another navigation can be admitted.
        select_first(&mut owner).await;
        owner.close_managed_navigation();
        assert_eq!(owner.open_managed_navigation(), Err(NavigationError::Busy));
        close(owner, completion).await;
    });
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1);
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
        settle_child(&mut owner, &fixture).await;
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
