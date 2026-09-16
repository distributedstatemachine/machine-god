use super::*;
use crate::mcp::ephemeral::NativeMcpEphemeralConfiguration;
mod startup;

fn fixture() -> Fixture {
    Fixture::with_options("ask", true, |selected, directory, clock| {
        let mcp =
            NativeReferenceHostMcpOptions::new(Arc::new(NativeMcpContexts::new()), clock.clone())
                .with_ephemeral_startup(crate::reference_host::mcp::tests::ephemeral::startup(
                    clock.clone(),
                ))
                .unwrap();
        options(selected, directory, clock).with_mcp_runtime(mcp)
    })
}

#[test]
fn staged_foreground_first_poll_rechecks_shutdown_without_starting_mcp() {
    let mut fixture = fixture();
    let path = journal_path(&fixture);
    run(async {
        let host = fixture.host.as_mut().unwrap();
        let mut agents = host
            .open_managed_agents(
                directory(&path),
                host.loaded_config().config().model_preferences(),
                NativeSessionOrigin::Acp,
            )
            .await
            .unwrap();
        let reservation = agents.reserve_foreground().unwrap();
        poll_fn(|cx| {
            let _ = agents.poll_progress(cx, 1);
            agents.poll_foreground_reservation(&reservation, cx)
        })
        .await
        .unwrap();
        let pending = agents.stage_foreground_mcp(
            reservation,
            #[cfg(feature = "mcp-http")]
            None,
            NativeMcpEphemeralConfiguration::decode(None).unwrap(),
            CancellationToken::new(),
        );
        agents.request_shutdown();
        let before = fixture.clock.0.load(Ordering::Relaxed);
        assert!(matches!(
            pending.await,
            Err(NativeManagedAgentsError::Unavailable)
        ));
        assert_eq!(fixture.clock.0.load(Ordering::Relaxed), before);
        poll_fn(|cx| agents.poll_shutdown(cx, 2)).await.unwrap();
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn failed_staged_startup_retains_residency_until_original_cleanup_and_release() {
    let mut fixture = fixture();
    let path = journal_path(&fixture);
    run(async {
        let host = fixture.host.as_mut().unwrap();
        let mut agents = host
            .open_managed_agents(
                directory(&path),
                host.loaded_config().config().model_preferences(),
                NativeSessionOrigin::Acp,
            )
            .await
            .unwrap();
        let reservation = agents.reserve_foreground().unwrap();
        poll_fn(|cx| {
            let _ = agents.poll_progress(cx, 1);
            agents.poll_foreground_reservation(&reservation, cx)
        })
        .await
        .unwrap();
        let mut stage = agents
            .stage_foreground_mcp(
                reservation,
                #[cfg(feature = "mcp-http")]
                None,
                NativeMcpEphemeralConfiguration::decode(Some(
                    br#"[{"name":"missing","command":"/bin/sh","args":[],"env":[]}]"#,
                ))
                .unwrap(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(stage.ready().is_err());
        assert!(
            agents
                .poll_shutdown(
                    &mut std::task::Context::from_waker(futures_util::task::noop_waker_ref()),
                    2
                )
                .is_pending()
        );
        stage.settle().await.unwrap();
        assert!(
            agents
                .poll_shutdown(
                    &mut std::task::Context::from_waker(futures_util::task::noop_waker_ref()),
                    2
                )
                .is_pending()
        );
        drop(stage);
        poll_fn(|cx| agents.poll_shutdown(cx, 2)).await.unwrap();
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

async fn interactive(fixture: &mut Fixture) -> NativeInteractiveSession {
    let path = journal_path(fixture);
    let host = fixture.host.take().unwrap();
    let options = NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        host.loaded_config().config().model_preferences(),
    )
    .unwrap()
    .with_origin(NativeSessionOrigin::Acp);
    NativeInteractiveSession::open_managed(
        host,
        directory(&path),
        options,
        NativeInteractiveInitialSession::Fresh,
        1,
    )
    .await
    .unwrap()
}

async fn ready_stage(
    owner: &mut NativeInteractiveSession,
) -> crate::reference_host::NativeManagedStagedParent {
    let reservation = owner.reserve_parent_stage().unwrap();
    poll_fn(|cx| {
        let _ = owner.poll_progress(cx, 2);
        owner.poll_parent_stage_reservation(&reservation, cx)
    })
    .await
    .unwrap();
    let candidate = owner
        .start_parent_stage(
            reservation,
            #[cfg(feature = "mcp-http")]
            None,
            NativeMcpEphemeralConfiguration::decode(None).unwrap(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    candidate.ready().unwrap();
    candidate
}

async fn selected_outcome(owner: &mut NativeInteractiveSession) -> NativeInteractiveOutcome {
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 3);
        assert!(owner.shutdown_error().is_none());
        assert!(owner.managed_error().is_none());
        let _ = owner.take_presentation();
        if let Some(outcome) = owner.take_outcome() {
            return Poll::Ready(outcome);
        }
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    })
    .await
}

async fn close_interactive(owner: &mut NativeInteractiveSession) {
    owner.request_shutdown();
    poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 4);
        let _ = owner.take_outcome();
        let _ = owner.take_presentation();
        assert!(owner.shutdown_error().is_none());
        if owner.is_closed() {
            Poll::Ready(())
        } else {
            if progress.is_ready() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        }
    })
    .await;
}

#[test]
fn staged_interactive_replacement_adopts_publication_and_keeps_the_child_manager() {
    let mut fixture = fixture();
    run(async {
        let mut owner = interactive(&mut fixture).await;
        let parent = owner.runtime().id();
        let original_mcp = owner.acp_mcp_runtime().unwrap();
        let mut created = owner
            .request_managed_command(
                machine_god_core::ManagedSubagentCommand::decode(serde_json::json!({
                    "command":{"create":{"name":"retained child","mode":"persistent"}}
                }))
                .unwrap(),
                CancellationToken::new(),
            )
            .unwrap();
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 2);
            created.as_mut().poll(cx)
        })
        .await
        .unwrap();
        let child = owner.managed_agents().remove(0).id;
        let candidate = ready_stage(&mut owner).await;
        let requested = owner
            .request_staged_transition(
                NativeInteractiveTransition::New,
                candidate,
                3,
                CancellationToken::new(),
            )
            .unwrap();
        assert!(matches!(
            owner.request_transition(NativeInteractiveTransition::New, 3),
            Err(crate::NativeInteractiveError::Busy)
        ));
        let NativeInteractiveOutcome::Transition(receipt) = selected_outcome(&mut owner).await
        else {
            panic!("expected exact replacement receipt");
        };
        assert_eq!(receipt.request, requested.id);
        assert_ne!(parent, owner.runtime().id());
        assert_eq!(owner.managed_agents()[0].id, child);
        assert_eq!(
            owner.managed_agents()[0].state,
            machine_god_core::ManagedAgentState::Idle
        );
        let selected_mcp = owner.acp_mcp_runtime().unwrap();
        assert!(!Arc::ptr_eq(&original_mcp, &selected_mcp));
        assert!(
            !selected_mcp
                .publication_checkpoint()
                .unwrap()
                .is_unpublished()
        );
        drop(original_mcp);
        drop(selected_mcp);
        close_interactive(&mut owner).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn rejected_staged_resume_settles_candidate_and_preserves_the_original_parent() {
    let mut fixture = fixture();
    run(async {
        let mut owner = interactive(&mut fixture).await;
        let parent = owner.runtime().id();
        let original_mcp = owner.acp_mcp_runtime().unwrap();
        let candidate = ready_stage(&mut owner).await;
        owner
            .request_staged_transition(
                NativeInteractiveTransition::Resume(crate::NativeResumeTarget::Exact(
                    machine_god_core::SessionId::new("missing-staged-session").unwrap(),
                )),
                candidate,
                3,
                CancellationToken::new(),
            )
            .unwrap();
        assert!(matches!(
            selected_outcome(&mut owner).await,
            NativeInteractiveOutcome::Rejected { .. }
        ));
        assert_eq!(owner.runtime().id(), parent);
        assert!(!owner.is_fenced());
        assert!(Arc::ptr_eq(
            &owner.acp_mcp_runtime().unwrap(),
            &original_mcp
        ));
        drop(original_mcp);
        close_interactive(&mut owner).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn shutdown_settles_a_staged_request_accepted_before_its_first_driver_poll() {
    let mut fixture = fixture();
    run(async {
        let mut owner = interactive(&mut fixture).await;
        let candidate = ready_stage(&mut owner).await;
        owner
            .request_staged_transition(
                NativeInteractiveTransition::New,
                candidate,
                3,
                CancellationToken::new(),
            )
            .unwrap();
        close_interactive(&mut owner).await;
        assert!(!owner.is_fenced());
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn cancelled_staged_selection_settles_without_replacing_the_parent() {
    let mut fixture = fixture();
    run(async {
        let mut owner = interactive(&mut fixture).await;
        let parent = owner.runtime().id();
        let candidate = ready_stage(&mut owner).await;
        let cancellation = CancellationToken::new();
        owner
            .request_staged_transition(
                NativeInteractiveTransition::New,
                candidate,
                3,
                cancellation.clone(),
            )
            .unwrap();
        cancellation.cancel();
        assert!(matches!(
            selected_outcome(&mut owner).await,
            NativeInteractiveOutcome::Rejected {
                error: crate::NativeInteractiveError::Closed,
                ..
            }
        ));
        assert_eq!(owner.runtime().id(), parent);
        assert!(!owner.is_fenced());
        close_interactive(&mut owner).await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}
