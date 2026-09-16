use super::human::{close, command, create, open, submit};
use super::*;
use crate::{
    NativeManagedCatalogFilter as Filter, NativeManagedFrameIdentity,
    NativeManagedNavigationAction as Action, NativeManagedNavigationError as Error,
    NativeManagedNavigationRoute as Route,
};
use machine_god_core::{ManagedFailureCode, ManagedLifecycleAction as Lifecycle};

pub(super) async fn ready(owner: &mut NativeInteractiveSession) -> NativeManagedFrameIdentity {
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 10);
        assert!(
            owner.managed_error().is_none(),
            "{:?}",
            owner.managed_error()
        );
        assert!(
            owner.shutdown_error().is_none(),
            "{:?}",
            owner.shutdown_error()
        );
        let view = owner.managed_navigation().unwrap();
        if !view.busy {
            return Poll::Ready(view.frame);
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await
}

#[test]
fn navigation_requires_exact_display_ack_and_does_not_replace_the_parent() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        submit(&mut owner, create()).await;
        let parent = owner.runtime().clone();
        owner.open_managed_navigation().unwrap();
        let loading = owner.managed_navigation().unwrap().frame;
        let frame = ready(&mut owner).await;
        assert_eq!(
            owner.acknowledge_managed_frame(&loading),
            Err(Error::StaleFrame)
        );
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::Select),
            Err(Error::NotDisplayed)
        );
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner.act_on_managed_frame(&frame, Action::Select).unwrap();
        let detail = ready(&mut owner).await;
        assert!(matches!(
            owner.managed_navigation().unwrap().route,
            Route::Conversation
        ));
        assert!(owner.managed_navigation().unwrap().history.is_some());
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::Lifecycle(Lifecycle::Close)),
            Err(Error::StaleFrame)
        );
        assert!(Arc::ptr_eq(&parent, owner.runtime()));
        owner.close_managed_navigation();
        assert_eq!(
            owner.acknowledge_managed_frame(&detail),
            Err(Error::StaleFrame)
        );
        assert!(owner.managed_navigation().is_none());
        drop(parent);
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn close_confirmation_needs_its_own_flush_and_old_target_still_rejects() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        owner.open_managed_navigation().unwrap();
        let list = ready(&mut owner).await;
        owner.acknowledge_managed_frame(&list).unwrap();
        assert_eq!(
            owner.act_on_managed_frame(&list, Action::ConfirmClose),
            Err(Error::InvalidAction)
        );
        owner
            .act_on_managed_frame(&list, Action::Lifecycle(Lifecycle::Close))
            .unwrap();
        let confirmation = owner.managed_navigation().unwrap().frame;
        assert_eq!(
            owner.act_on_managed_frame(&confirmation, Action::ConfirmClose),
            Err(Error::NotDisplayed)
        );
        assert!(
            submit(
                &mut owner,
                command(serde_json::json!({
                    "configure":{"id":child,"name":"changed after display"}
                }))
            )
            .await
            .ok
        );
        owner.acknowledge_managed_frame(&confirmation).unwrap();
        owner
            .act_on_managed_frame(&confirmation, Action::ConfirmClose)
            .unwrap();
        ready(&mut owner).await;
        assert_eq!(
            owner
                .managed_navigation()
                .unwrap()
                .result
                .unwrap()
                .error_code,
            Some(ManagedFailureCode::StaleGeneration)
        );
        assert_eq!(owner.managed_agents().len(), 1);
        owner.close_managed_navigation();
        close(owner, completion).await;
    });
}

#[test]
fn catalog_result_cannot_be_stolen_from_the_navigation_request() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        owner.open_managed_navigation().unwrap();
        assert!(owner.take_managed_catalog_outcome().is_none());
        assert!(matches!(
            owner.request_managed_catalog(Filter::All, None, 1),
            Err(crate::NativeManagedCatalogError::Busy)
        ));
        ready(&mut owner).await;
        assert!(owner.managed_navigation().unwrap().rows.is_empty());
        close(owner, completion).await;
    });
}

#[test]
fn successful_close_retires_confirmation_and_displays_the_original_receipt() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        owner.open_managed_navigation().unwrap();
        let frame = ready(&mut owner).await;
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner
            .act_on_managed_frame(&frame, Action::Lifecycle(Lifecycle::Close))
            .unwrap();
        let confirmation = owner.managed_navigation().unwrap().frame;
        let editor = owner.managed_navigation().unwrap().editor;
        owner.acknowledge_managed_frame(&confirmation).unwrap();
        owner
            .act_on_managed_frame(&confirmation, Action::ConfirmClose)
            .unwrap();
        let receipt = ready(&mut owner).await;
        let view = owner.managed_navigation().unwrap();
        assert!(matches!(view.route, Route::Agent(_)));
        assert_ne!(view.editor, editor);
        assert!(view.result.unwrap().ok);
        assert_eq!(
            view.result.unwrap().child_id.as_deref(),
            Some(child.as_str())
        );
        assert_eq!(
            owner.act_on_managed_frame(&confirmation, Action::ConfirmClose),
            Err(Error::StaleFrame)
        );
        owner.acknowledge_managed_frame(&receipt).unwrap();
        owner
            .act_on_managed_frame(&receipt, Action::Filter(Filter::Archived))
            .unwrap();
        ready(&mut owner).await;
        assert!(
            owner
                .managed_navigation()
                .unwrap()
                .rows
                .iter()
                .any(|row| row.id == child)
        );
        close(owner, completion).await;
    });
}

#[test]
fn close_while_loading_keeps_original_request_and_reopen_uses_a_new_editor() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        owner.open_managed_navigation().unwrap();
        let old = owner.managed_navigation().unwrap().editor;
        owner.close_managed_navigation();
        assert_eq!(owner.open_managed_navigation(), Err(Error::Busy));
        poll_fn(|cx| {
            let progress = owner.poll_progress(cx, 10);
            match owner.open_managed_navigation() {
                Ok(()) => Poll::Ready(()),
                Err(Error::Busy) => {
                    if progress.is_ready() {
                        cx.waker().wake_by_ref();
                    }
                    Poll::Pending
                }
                Err(error) => panic!("{error:?}"),
            }
        })
        .await;
        let frame = ready(&mut owner).await;
        assert_ne!(old, owner.managed_navigation().unwrap().editor);
        owner.acknowledge_managed_frame(&frame).unwrap();
        close(owner, completion).await;
    });
}

#[test]
fn resize_foreign_and_retired_parent_frames_never_authorize_an_action() {
    let mut fixture = Fixture::with_options("auto", true, options);
    let mut other = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let (mut foreign, foreign_completion) = open(&mut other).await;
        submit(&mut owner, create()).await;
        owner.open_managed_navigation().unwrap();
        foreign.open_managed_navigation().unwrap();
        let frame = ready(&mut owner).await;
        ready(&mut foreign).await;
        assert_eq!(
            foreign.acknowledge_managed_frame(&frame),
            Err(Error::StaleFrame)
        );
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner.invalidate_managed_frame().unwrap();
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::Select),
            Err(Error::StaleFrame)
        );
        let resized = owner.managed_navigation().unwrap().frame;
        owner.acknowledge_managed_frame(&resized).unwrap();
        owner
            .request_transition(NativeInteractiveTransition::New, 11)
            .unwrap();
        assert!(owner.managed_navigation().is_none());
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        assert_eq!(
            owner.act_on_managed_frame(&resized, Action::Select),
            Err(Error::StaleFrame)
        );
        close(owner, completion).await;
        close(foreign, foreign_completion).await;
    });
}
