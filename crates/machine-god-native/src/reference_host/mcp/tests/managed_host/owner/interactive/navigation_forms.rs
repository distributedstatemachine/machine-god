use super::human::{close, command, create, open, submit};
use super::navigation_ui::ready;
use super::*;
use crate::{
    NativeManagedFormError, NativeManagedFormKind as Kind, NativeManagedNavigationAction as Action,
    NativeManagedNavigationError as Error, NativeManagedNavigationRoute as Route,
};

fn displayed_action(owner: &mut NativeInteractiveSession, action: Action) -> Result<(), Error> {
    let frame = owner.managed_navigation().unwrap().frame;
    owner.acknowledge_managed_frame(&frame).unwrap();
    owner.act_on_managed_frame(&frame, action)
}

#[test]
fn create_form_edits_require_a_new_frame_before_actual_human_admission() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        owner.open_managed_navigation().unwrap();
        ready(&mut owner).await;
        displayed_action(&mut owner, Action::OpenForm(Kind::Create)).unwrap();
        let view = owner.managed_navigation().unwrap();
        let before_edit = view.frame;
        let editor = view.editor;
        owner.acknowledge_managed_frame(&before_edit).unwrap();
        owner
            .edit_managed_form(&editor, "form-created child")
            .unwrap();
        assert_eq!(
            owner.act_on_managed_frame(&before_edit, Action::SubmitForm),
            Err(Error::StaleFrame)
        );
        let frame = owner.managed_navigation().unwrap().frame;
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::SubmitForm),
            Err(Error::NotDisplayed)
        );
        displayed_action(&mut owner, Action::SubmitForm).unwrap();
        ready(&mut owner).await;
        let view = owner.managed_navigation().unwrap();
        assert!(matches!(view.route, Route::Catalog(_)));
        assert!(view.form.is_none());
        assert!(view.result.unwrap().ok);
        assert!(view.result.unwrap().child_id.is_some());
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn rejected_field_edit_cannot_submit_the_previous_prefix_or_change_field_owner() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        owner.open_managed_navigation().unwrap();
        ready(&mut owner).await;
        displayed_action(&mut owner, Action::OpenForm(Kind::Create)).unwrap();
        let editor = owner.managed_navigation().unwrap().editor;
        owner
            .edit_managed_form(&editor, "previous valid value")
            .unwrap();
        let error = Error::Form(NativeManagedFormError::TooLarge(
            crate::NativeManagedFormField::Name,
        ));
        assert_eq!(
            owner.edit_managed_form(&editor, &"x".repeat(129)),
            Err(error)
        );
        assert_eq!(displayed_action(&mut owner, Action::Next), Err(error));
        assert_eq!(owner.managed_navigation().unwrap().editor, editor);
        assert_eq!(displayed_action(&mut owner, Action::SubmitForm), Err(error));
        assert!(owner.managed_agents().is_empty());
        owner.edit_managed_form(&editor, "corrected").unwrap();
        displayed_action(&mut owner, Action::Next).unwrap();
        assert_eq!(
            owner.edit_managed_form(&editor, "old field"),
            Err(Error::StaleFrame)
        );
        close(owner, completion).await;
    });
}

#[test]
fn configure_preserves_draft_after_stale_rejection_and_refreshes_only_the_same_target() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        owner.open_managed_navigation().unwrap();
        ready(&mut owner).await;
        displayed_action(&mut owner, Action::OpenForm(Kind::Configure)).unwrap();
        ready(&mut owner).await;
        let editor = owner.managed_navigation().unwrap().editor;
        owner.edit_managed_form(&editor, "preserved draft").unwrap();
        assert!(
            submit(
                &mut owner,
                command(serde_json::json!({"configure":{"id":child,"name":"concurrent winner"}}))
            )
            .await
            .ok
        );
        displayed_action(&mut owner, Action::SubmitForm).unwrap();
        ready(&mut owner).await;
        let view = owner.managed_navigation().unwrap();
        assert_eq!(
            view.result.unwrap().error_code,
            Some(machine_god_core::ManagedFailureCode::StaleGeneration)
        );
        assert_eq!(view.form.unwrap().values[0], "preserved draft");
        assert_ne!(view.editor, editor);
        displayed_action(&mut owner, Action::Refresh).unwrap();
        ready(&mut owner).await;
        assert_eq!(
            owner.managed_navigation().unwrap().form.unwrap().values[0],
            "preserved draft"
        );
        displayed_action(&mut owner, Action::SubmitForm).unwrap();
        ready(&mut owner).await;
        assert!(owner.managed_navigation().unwrap().result.unwrap().ok);
        let result = submit(
            &mut owner,
            command(serde_json::json!({"inspect":{"id":child,"sections":["configuration"]}})),
        )
        .await;
        let Some(machine_god_core::ManagedRequested::Inspection(inspection)) = result.requested
        else {
            panic!("inspection");
        };
        assert_eq!(inspection.configuration.unwrap().name, "preserved draft");
        close(owner, completion).await;
    });
}

#[test]
fn refreshing_a_form_never_retargets_a_missing_or_replaced_generation() {
    for reopen in [false, true] {
        let mut fixture = Fixture::with_options("auto", true, options);
        run(async {
            let (mut owner, completion) = open(&mut fixture).await;
            let child = submit(&mut owner, create()).await.child_id.unwrap();
            owner.open_managed_navigation().unwrap();
            ready(&mut owner).await;
            displayed_action(&mut owner, Action::OpenForm(Kind::Configure)).unwrap();
            ready(&mut owner).await;
            let editor = owner.managed_navigation().unwrap().editor;
            owner
                .edit_managed_form(&editor, "old generation draft")
                .unwrap();
            for action in if reopen {
                &["close", "reopen"][..]
            } else {
                &["close"][..]
            } {
                assert!(
                    submit(
                        &mut owner,
                        command(serde_json::json!({
                            "lifecycle": {"id":child,"action":action}
                        }))
                    )
                    .await
                    .ok
                );
            }
            displayed_action(&mut owner, Action::Refresh).unwrap();
            ready(&mut owner).await;
            let view = owner.managed_navigation().unwrap();
            assert!(matches!(view.route, Route::Catalog(_)));
            assert!(view.form.is_none());
            assert!(view.target.is_none());
            assert!(view.rows.is_empty());
            assert_ne!(view.editor, editor);
            assert_eq!(
                owner.edit_managed_form(&editor, "retargeted edit"),
                Err(Error::StaleFrame)
            );
            close(owner, completion).await;
        });
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
    }
}
