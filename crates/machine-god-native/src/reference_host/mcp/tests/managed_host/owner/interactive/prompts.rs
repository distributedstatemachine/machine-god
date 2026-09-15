use super::*;
use machine_god_core::{BackgroundOutputOwner, ManagedAgentState};

async fn child_approval(
    owner: &mut NativeInteractiveSession,
    inbox: &mut NativeInteractivePromptInbox,
) -> NativeInteractivePromptView {
    let parent = BackgroundOutputOwner::new(owner.runtime().id(), owner.runtime().incarnation_id());
    let mut parent_done = false;
    let mut child = None;
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 10);
        assert!(owner.managed_error().is_none());
        let _ = owner.take_presentation();
        if let Some(result) = owner.take_outcome() {
            match result {
                NativeInteractiveOutcome::Turn(Ok(event)) => {
                    assert!(matches!(event.payload, TurnEvent::Completed { .. }));
                    parent_done = true;
                }
                NativeInteractiveOutcome::Turn(Err(error)) => panic!("parent: {error:?}"),
                _ => panic!("unexpected parent outcome"),
            }
        }
        if child.is_none()
            && let Poll::Ready(Some(prompt)) = inbox.poll_prompt(cx)
        {
            assert!(prompt.permission().is_some());
            if prompt.token().owner() == &parent {
                inbox
                    .reply(
                        prompt.token(),
                        NativeInteractivePromptResponse::Permission(
                            PermissionPromptDecision::AllowOnce,
                        ),
                    )
                    .unwrap();
            } else {
                child = Some(prompt);
            }
        }
        if parent_done
            && child.is_some()
            && owner
                .managed_agents()
                .first()
                .is_some_and(|child| child.state == ManagedAgentState::AwaitingApproval)
        {
            return Poll::Ready(child.take().unwrap());
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await
}

fn fixture(inbox: &NativeInteractivePromptInbox) -> Fixture {
    let bridge = inbox.router();
    let fixture = Fixture::with_options_and_bridge(
        "ask",
        true,
        Some(bridge),
        |selected, directory, clock| {
            options(selected, directory, clock.clone()).with_managed_agents(
                NativeReferenceHostManagedOptions::new(clock).with_prompt_inbox(inbox),
            )
        },
    );
    fixture.transport.responses.lock().unwrap().extend([
        call("spawn", "subagent", &serde_json::json!({"command":{"create":{
            "name":"worker","mode":"persistent","model":"fixture/child","prompt":"write the child file"}}})),
        answer(),
    ]);
    fixture.transport.model_responses.lock().unwrap().insert(
        "fixture/child".into(),
        [
            call(
                "write",
                "write_file",
                &serde_json::json!({"path":"child.txt","content":"approved child"}),
            ),
            answer(),
        ]
        .into(),
    );
    fixture
}

async fn scenario(shutdown_pending: bool) {
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let mut fixture = fixture(&inbox);
    let path = journal_path(&fixture);
    let host = fixture.host.take().unwrap();
    let completion = host.terminal_shutdown_completion().unwrap();
    let options = NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        host.loaded_config().config().model_preferences(),
    )
    .unwrap();
    let mut owner = NativeInteractiveSession::open_managed(
        host,
        directory(&path),
        options,
        NativeInteractiveInitialSession::Fresh,
        1,
    )
    .await
    .unwrap();
    owner
        .enqueue("create the persistent worker".into())
        .unwrap();
    let prompt = child_approval(&mut owner, &mut inbox).await;
    assert!(!fixture.workspace.join("child.txt").exists());
    if !shutdown_pending {
        owner
            .request_transition(NativeInteractiveTransition::New, 20)
            .unwrap();
        assert!(matches!(
            outcome_at(&mut owner, 21).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        let retained = inbox.select_prompt(prompt.token()).unwrap();
        assert_eq!(retained.token(), prompt.token());
        inbox
            .reply(
                prompt.token(),
                NativeInteractivePromptResponse::Permission(PermissionPromptDecision::AllowOnce),
            )
            .unwrap();
        poll_fn(|cx| {
            let progress = owner.poll_progress(cx, 22);
            assert!(owner.managed_error().is_none());
            if owner
                .managed_agents()
                .first()
                .is_some_and(|child| child.state == ManagedAgentState::Idle)
            {
                return Poll::Ready(());
            }
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        })
        .await;
        assert_eq!(
            fs::read(fixture.workspace.join("child.txt")).unwrap(),
            b"approved child"
        );
    }
    owner.request_shutdown();
    assert!(matches!(
        outcome_at(&mut owner, 23).await,
        NativeInteractiveOutcome::Shutdown
    ));
    assert_eq!(
        inbox.select_prompt(prompt.token()).unwrap_err(),
        NativeInteractivePromptError::Stale
    );
    if shutdown_pending {
        assert!(!fixture.workspace.join("child.txt").exists());
    }
    drop(owner);
    completion.wait().await;
}

#[test]
fn parent_replacement_preserves_hidden_child_approval_and_actual_child_execution() {
    run(scenario(false));
}

#[test]
fn shutdown_retires_child_approval_without_granting_or_executing_it() {
    run(scenario(true));
}

#[test]
fn dropped_inbox_rejects_managed_preparation_before_any_provider_execution() {
    run(async {
        let inbox =
            NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
        let mut fixture = fixture(&inbox);
        drop(inbox);
        let path = journal_path(&fixture);
        let host = fixture.host.take().unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            host.loaded_config().config().model_preferences(),
        )
        .unwrap();
        let result = NativeInteractiveSession::open_managed(
            host,
            directory(&path),
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        )
        .await;
        assert!(matches!(
            result,
            Err(NativeInteractiveError::Managed(
                NativeManagedAgentsError::Unavailable
            ))
        ));
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        completion.wait().await;
    });
}
