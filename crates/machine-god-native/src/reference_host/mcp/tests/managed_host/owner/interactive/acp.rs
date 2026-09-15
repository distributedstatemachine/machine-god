use super::*;
use crate::acp::{
    selection::{NativeAcpSelectionOutcome, NativeAcpSelectionOwner, tests as selection_fixture},
    session::NativeAcpSession,
};

async fn open(fixture: &mut Fixture) -> (NativeAcpSession, Arc<NativeReferenceHost>) {
    let mut host = fixture.host.take().unwrap();
    let preferences = host.loaded_config().config().model_preferences();
    let agents = host
        .open_managed_agents(
            directory(&journal_path(fixture)),
            preferences.clone(),
            NativeSessionOrigin::Acp,
        )
        .await
        .unwrap();
    let options = NativeInteractiveSessionOptions::new(fixture.workspace.clone(), preferences)
        .unwrap()
        .with_origin(NativeSessionOrigin::Acp);
    let host = Arc::new(host);
    let mut startup =
        crate::NativeManagedInteractiveStartup::new(host.clone(), options, agents).unwrap();
    startup
        .request_open(NativeInteractiveInitialSession::Fresh, 1)
        .unwrap();
    let inner = poll_fn(|cx| startup.poll_open(cx, 1))
        .await
        .unwrap()
        .unwrap();
    drop(startup);
    (NativeAcpSession::from_interactive(inner, false), host)
}

async fn approval(
    owner: &mut NativeAcpSelectionOwner,
    inbox: &mut NativeInteractivePromptInbox,
) -> NativeInteractivePromptView {
    let parent = owner.current().unwrap().principal();
    let mut child = None;
    poll_fn(|cx| {
        let _ = owner.poll_progress(cx, 10);
        let _ = owner.take_presentation();
        if child.is_none()
            && let Poll::Ready(Some(prompt)) = inbox.poll_prompt(cx)
        {
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
        if child.is_some() && selection_fixture::retains_turn_outcome(owner) {
            return Poll::Ready(child.take().unwrap());
        }
        // Presentation drainage and native effects supply the next wake. Keep
        // the original parent outcome retained by the selection owner.
        Poll::Pending
    })
    .await
}

#[test]
fn acp_retained_parent_response_does_not_block_hidden_child_approval_or_completion() {
    let mut inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let mut fixture = prompts::fixture(&inbox);
    run(async {
        let (session, host) = open(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        let parent = session.id();
        let mut owner = selection_fixture::preopened(session, host);
        owner
            .current_mut()
            .unwrap()
            .enqueue(
                &parent,
                crate::acp::prompt::decode_prompt_input(&serde_json::json!({
                    "prompt":[{"type":"text","text":"create the child"}]
                }))
                .unwrap(),
            )
            .unwrap();
        let child = approval(&mut owner, &mut inbox).await;
        assert!(!fixture.workspace.join("child.txt").exists());
        inbox
            .reply(
                child.token(),
                NativeInteractivePromptResponse::Permission(PermissionPromptDecision::AllowOnce),
            )
            .unwrap();
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 20);
            assert!(selection_fixture::retains_turn_outcome(&owner));
            if fixture.transport.requests.lock().unwrap().len() == 4 {
                return Poll::Ready(());
            }
            Poll::Pending
        })
        .await;
        assert_eq!(
            fs::read(fixture.workspace.join("child.txt")).unwrap(),
            b"approved child"
        );
        let retained = owner.take_turn_outcome().unwrap();
        assert_eq!(retained.owner.session_id(), &parent);
        assert!(matches!(
            retained.outcome.unwrap().payload,
            TurnEvent::Completed { .. }
        ));
        owner.request_shutdown();
        let closed = poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 30);
            let _ = owner.take_presentation();
            owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
        })
        .await;
        assert!(
            matches!(closed, NativeAcpSelectionOutcome::Closed { .. }),
            "{closed:?}"
        );
        assert!(owner.is_closed());
        drop(owner);
        completion.wait().await;
    });
}

#[test]
fn acp_retired_runtime_closes_managed_resources_without_second_quiescence() {
    let mut fixture = Fixture::with_options("auto", true, options);
    run(async {
        let (mut session, host) = open(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        assert!(
            session.close_retired().is_err(),
            "unretired runtime is not a cleanup receipt"
        );
        let mut guard = session.begin_quiescence().unwrap();
        guard.wait_idle().await.unwrap();
        assert!(session.foreground_settled());
        guard.try_retire().unwrap();
        drop(guard);
        session.close_retired().unwrap();
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 10);
            let _ = session.take_outcome();
            assert!(session.shutdown_error().is_none());
            if session.is_closed() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        drop(session);
        drop(host);
        completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn acp_commands_select_the_actual_foreground_ephemeral_runtime_not_the_host_seed() {
    let mut fixture = Fixture::with_options("ask", true, |selected, directory, clock| {
        let mcp =
            NativeReferenceHostMcpOptions::new(Arc::new(NativeMcpContexts::new()), clock.clone())
                .with_ephemeral_startup(NativeReferenceHostMcpEphemeralStartupOptions {
                    captured_environment: vec![],
                    stdio: None,
                    clock,
                    catalog_epoch: std::time::Instant::now(),
                    owner_cancellation: CancellationToken::new(),
                    #[cfg(feature = "mcp-http")]
                    network: None,
                    peer_lifetime: crate::mcp::lifetime::McpPeerLifetime::OwnerControlled,
                    max_retained_bytes: 1024 * 1024,
                    max_retained_generations: 4,
                })
                .unwrap();
        let clock = Arc::new(Clock::default());
        options(selected, directory, clock).with_mcp_runtime(mcp)
    });
    run(async {
        let (mut session, host) = open(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        assert!(host.mcp_runtime().is_none());
        assert!(host.mcp_ephemeral_owner().is_none());
        let runtime = session.command_services.mcp.as_ref().unwrap();
        assert!(runtime.publication_checkpoint().unwrap().is_unpublished());
        session.request_close(&session.id()).unwrap();
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 10);
            let _ = session.take_outcome();
            assert!(session.shutdown_error().is_none());
            if session.is_closed() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        drop(session);
        drop(host);
        completion.wait().await;
    });
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}
