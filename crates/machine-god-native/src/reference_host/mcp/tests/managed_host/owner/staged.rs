use super::*;
use crate::mcp::ephemeral::NativeMcpEphemeralConfiguration;

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
