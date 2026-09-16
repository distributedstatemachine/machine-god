use super::human::{close, command, create, open, submit};
use super::navigation_ui::ready;
use super::*;
use crate::{
    NativeManagedNavigationAction as Action, NativeManagedNavigationError as Error,
    NativeManagedNavigationRoute as Route, NativeManagedProcessScope as Scope,
};
use machine_god_core::{BackgroundOutputOwner, ManagedLifecycleAction};

async fn act(owner: &mut NativeInteractiveSession, action: Action) {
    let frame = ready(owner).await;
    owner.acknowledge_managed_frame(&frame).unwrap();
    owner.act_on_managed_frame(&frame, action).unwrap();
    ready(owner).await;
}

#[test]
fn child_process_is_visible_only_through_its_original_runtime_scope() {
    let helper = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY")
        .expect("real child process fixture requires the fresh explicit release helper");
    let mut fixture = Fixture::with_options("auto", true, |selected, directory, clock| {
        options(selected, directory, clock).with_terminal(
            crate::NativeReferenceHostTerminalOptions::new(
                helper.into(),
                Some("/bin/bash".into()),
                vec![],
            )
            .unwrap(),
        )
    });
    let process_command = "printf child-process; exec /bin/sleep 30";
    fixture.transport.responses.lock().unwrap().extend([
        call(
            "child-terminal",
            "terminal",
            &serde_json::json!({"action":"start","profile":"clean","command":process_command}),
        ),
        answer(),
    ]);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        assert!(submit(&mut owner, command(serde_json::json!({"message":{"send":{"id":child,"content":"start child process"}}}))).await.ok);
        poll_fn(|cx| {
            let progress = owner.poll_progress(cx, 10);
            assert!(owner.managed_error().is_none());
            let ordinary_requests = fixture
                .transport
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| {
                    !request["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|tool| tool["name"] == "permission_decision")
                })
                .count();
            assert!(
                ordinary_requests <= 2,
                "unexpected additional child model request"
            );
            if ordinary_requests == 2
                && owner
                    .managed_agents()
                    .iter()
                    .all(|agent| agent.state == machine_god_core::ManagedAgentState::Idle)
            {
                return Poll::Ready(());
            }
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        })
        .await;
        owner.open_managed_navigation().unwrap();
        act(&mut owner, Action::Processes(Scope::Parent)).await;
        assert!(
            owner
                .managed_navigation()
                .unwrap()
                .processes
                .unwrap()
                .entries()
                .is_empty()
        );
        act(&mut owner, Action::Processes(Scope::SelectedAgent)).await;
        let view = owner.managed_navigation().unwrap();
        let entries = view.processes.unwrap().entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command(), Some(process_command));
        assert!(entries[0].owns_backend());
        close(owner, completion).await;
    });
}

#[test]
fn process_snapshots_use_distinct_actual_principals_without_replacing_parent() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        submit(&mut owner, create()).await;
        let parent = owner.runtime().clone();
        let parent_principal = BackgroundOutputOwner::new(parent.id(), parent.incarnation_id());
        owner.open_managed_navigation().unwrap();
        act(&mut owner, Action::Processes(Scope::Parent)).await;
        let view = owner.managed_navigation().unwrap();
        assert_eq!(view.route, Route::Processes(Scope::Parent));
        assert_eq!(view.process_owner, Some(parent_principal.clone()));
        assert!(view.target.is_none());
        assert!(view.processes.unwrap().entries().is_empty());
        act(&mut owner, Action::Processes(Scope::SelectedAgent)).await;
        let view = owner.managed_navigation().unwrap();
        assert_eq!(view.route, Route::Processes(Scope::SelectedAgent));
        assert_ne!(view.process_owner, Some(parent_principal));
        assert!(view.target.is_some());
        assert!(view.processes.unwrap().entries().is_empty());
        assert!(Arc::ptr_eq(&parent, owner.runtime()));
        act(&mut owner, Action::Refresh).await;
        act(&mut owner, Action::Back).await;
        assert!(matches!(
            owner.managed_navigation().unwrap().route,
            Route::Catalog(_)
        ));
        assert!(owner.managed_navigation().unwrap().processes.is_none());
        drop(parent);
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn process_frame_needs_ack_and_cannot_be_used_as_agent_command_authority() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        submit(&mut owner, create()).await;
        owner.open_managed_navigation().unwrap();
        let frame = ready(&mut owner).await;
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::Processes(Scope::Parent)),
            Err(Error::NotDisplayed)
        );
        act(&mut owner, Action::Processes(Scope::SelectedAgent)).await;
        let process_frame = ready(&mut owner).await;
        owner.acknowledge_managed_frame(&process_frame).unwrap();
        for action in [
            Action::Select,
            Action::Message("not a prompt".into()),
            Action::Lifecycle(ManagedLifecycleAction::Cancel),
            Action::Lifecycle(ManagedLifecycleAction::Close),
        ] {
            assert_eq!(
                owner.act_on_managed_frame(&process_frame, action),
                Err(Error::InvalidAction)
            );
        }
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::Select),
            Err(Error::StaleFrame)
        );
        close(owner, completion).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn closing_unpolled_process_snapshot_retains_settlement_before_reopening() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        owner.open_managed_navigation().unwrap();
        let frame = ready(&mut owner).await;
        owner.acknowledge_managed_frame(&frame).unwrap();
        owner
            .act_on_managed_frame(&frame, Action::Processes(Scope::Parent))
            .unwrap();
        assert!(owner.managed_navigation().unwrap().busy);
        owner.close_managed_navigation();
        assert_eq!(owner.open_managed_navigation(), Err(Error::Busy));
        // Shutdown must drive the original cancelled snapshot and release its
        // original runtime lease; dropping its view is not a cleanup receipt.
        close(owner, completion).await;
    });
}

#[test]
fn replaced_child_generation_cannot_retarget_a_retained_process_scope() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut owner, completion) = open(&mut fixture).await;
        let child = submit(&mut owner, create()).await.child_id.unwrap();
        owner.open_managed_navigation().unwrap();
        act(&mut owner, Action::Processes(Scope::SelectedAgent)).await;
        let original = owner.managed_navigation().unwrap().process_owner;
        for action in ["close", "reopen"] {
            assert!(
                submit(
                    &mut owner,
                    command(serde_json::json!({"lifecycle":{"id":child,"action":action}}))
                )
                .await
                .ok
            );
        }
        let frame = ready(&mut owner).await;
        owner.acknowledge_managed_frame(&frame).unwrap();
        assert_eq!(
            owner.act_on_managed_frame(&frame, Action::Refresh),
            Err(Error::NoSelection)
        );
        // Retained data may still be shown with its explicit error, but never
        // relabelled as the new generation's live process snapshot.
        let view = owner.managed_navigation().unwrap();
        assert_eq!(view.process_owner, original);
        assert_eq!(view.error, Some(Error::NoSelection));
        close(owner, completion).await;
    });
}
