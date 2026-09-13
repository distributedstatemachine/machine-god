use super::*;
use crate::mcp::{
    ephemeral::{NativeMcpEphemeralConfiguration, NativeMcpEphemeralError},
    lifetime::McpPeerLifetime,
    store::NativeMcpConfigStore,
};

fn startup(clock: Arc<Clock>) -> NativeReferenceHostMcpEphemeralStartupOptions {
    NativeReferenceHostMcpEphemeralStartupOptions {
        captured_environment: vec![],
        stdio: None,
        clock,
        catalog_epoch: Instant::now(),
        owner_cancellation: CancellationToken::new(),
        #[cfg(feature = "mcp-http")]
        network: None,
        peer_lifetime: McpPeerLifetime::OwnerControlled,
        max_retained_bytes: 1024 * 1024,
        max_retained_generations: 4,
    }
}

fn fixture() -> Fixture {
    Fixture::with_options("ask", true, |mut options, _, clock| {
        let mcp = options
            .mcp_runtime
            .take()
            .unwrap()
            .with_ephemeral_startup(startup(clock))
            .unwrap();
        options.with_mcp_runtime(mcp)
    })
}

#[test]
fn ephemeral_host_deadline_uses_the_selected_clock_and_unpolled_settle_is_inert() {
    struct FixedClock(Instant, AtomicUsize);
    impl NativeMcpRuntimeClock for FixedClock {
        fn now(&self) -> Instant {
            self.1.fetch_add(1, Ordering::Relaxed);
            self.0
        }
        fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }
    let fixed = Arc::new(FixedClock(
        Instant::now()
            .checked_add(Duration::from_secs(100))
            .unwrap(),
        AtomicUsize::new(0),
    ));
    let fixture = Fixture::with_options("ask", true, |mut options, _, clock| {
        let contexts = options.mcp_runtime.take().unwrap().contexts;
        let mut startup = startup(clock);
        startup.clock = fixed.clone();
        startup.catalog_epoch = fixed.0;
        options.with_mcp_runtime(
            NativeReferenceHostMcpOptions::new(contexts, fixed.clone())
                .with_ephemeral_startup(startup)
                .unwrap(),
        )
    });
    assert_eq!(fixed.1.load(Ordering::Relaxed), 0);
    let host = fixture.host();
    let deadline = host.mcp_deadline_after(Duration::from_secs(5)).unwrap();
    assert_eq!(deadline, fixed.0 + Duration::from_secs(5));
    assert_eq!(fixed.1.load(Ordering::Relaxed), 1);
    let owner = host.mcp_ephemeral_owner().unwrap();
    run(async {
        owner
            .replace(
                NativeMcpEphemeralConfiguration::decode(None).unwrap(),
                CancellationToken::new(),
                deadline,
            )
            .await
            .unwrap();
        let calls = fixed.1.load(Ordering::Relaxed);
        let pending = host.settle_mcp_ephemeral(deadline, CancellationToken::new());
        assert_eq!(fixed.1.load(Ordering::Relaxed), calls);
        drop(pending);
        owner.ready().unwrap();
        host.settle_mcp_ephemeral(deadline, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(owner.ready(), Err(NativeMcpEphemeralError::Closed));
    });
}

#[test]
fn ephemeral_host_selection_rejects_profile_authority_in_both_orders_without_effects() {
    let directory = Directory::new();
    let clock = Arc::new(Clock::default());
    let make =
        || NativeReferenceHostMcpOptions::new(Arc::new(NativeMcpContexts::new()), clock.clone());
    let profile = || crate::mcp::controller::NativeMcpControllerStartupOptions {
        captured_environment: vec![],
        stdio: None,
        clock: clock.clone(),
        catalog_epoch: Instant::now(),
        owner_cancellation: CancellationToken::new(),
        #[cfg(feature = "mcp-http")]
        network: None,
        #[cfg(feature = "mcp-http")]
        authentication: vec![],
        peer_lifetime: McpPeerLifetime::OwnerControlled,
        max_retained_bytes: 1024 * 1024,
    };
    assert!(
        make()
            .with_controller_startup(profile())
            .with_ephemeral_startup(startup(clock.clone()))
            .is_err()
    );
    let selected = make()
        .with_ephemeral_startup(startup(clock.clone()))
        .unwrap();
    assert!(
        selected
            .clone()
            .with_controller_startup(profile())
            .validate_controller(true)
            .is_err()
    );
    assert!(
        make()
            .with_ephemeral_startup(startup(Arc::new(Clock::default())))
            .is_err()
    );
    assert!(
        selected
            .clone()
            .with_ephemeral_startup(startup(clock.clone()))
            .is_err()
    );
    let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
        .with_terminal(terminal())
        .with_permissions(permission(clock.clone()))
        .with_mcp_runtime(selected);
    let config = LoadedNativeConfig::from_file(NativeConfig::default());
    validate_prepared_selections(&config, &options.clone().into()).unwrap();
    let service = Arc::new(NativeMcpManagementService::new(Arc::new(
        NativeMcpConfigStore::new(directory.0.join("unopened-profile")).unwrap(),
    )));
    assert!(
        validate_prepared_selections(&config, &options.with_mcp_management(service).into())
            .is_err()
    );
    assert_eq!(clock.0.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
}

#[test]
fn ephemeral_host_requires_exact_ready_publication_for_each_native_prompt() {
    let fixture = fixture();
    let host = fixture.host();
    let owner = host.mcp_ephemeral_owner().unwrap();
    assert!(host.mcp_controller().is_none());
    assert!(host.mcp_management().is_none());
    #[cfg(feature = "mcp-http")]
    assert!(host.mcp_authentication().is_none());
    assert_eq!(fixture.clock.0.load(Ordering::Relaxed), 0);
    run(async {
        let conversation = fixture.conversation().await;
        assert!(matches!(
            conversation.prompt("unready".into(), 1).await,
            Err(NativeConversationError::McpRequiredUnavailable)
        ));
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        let deadline = host.mcp_deadline_after(Duration::from_secs(5)).unwrap();
        let publication = owner.replace(
            NativeMcpEphemeralConfiguration::decode(None).unwrap(),
            CancellationToken::new(),
            deadline,
        );
        assert!(owner.ready().is_err());
        let receipt = publication.await.unwrap();
        owner.ready().unwrap();
        let turn = conversation.prompt("ready".into(), 2).await.unwrap();
        let _events: Vec<_> = turn.collect().await;
        host.close_mcp();
        assert!(matches!(
            conversation.prompt("closed".into(), 3).await,
            Err(NativeConversationError::McpRequiredUnavailable)
        ));
        host.settle_mcp_ephemeral(deadline, CancellationToken::new())
            .await
            .unwrap();
        assert!(receipt.cleanup_complete());
    });
}

#[test]
fn ephemeral_host_omission_and_empty_never_read_or_modify_profile_sentinels() {
    let fixture = fixture();
    let profile = fixture.state.join("mcp.json");
    let credentials = fixture.state.join("mcp-credentials.json");
    fs::write(&profile, b"invalid profile sentinel").unwrap();
    fs::write(&credentials, b"invalid credential sentinel").unwrap();
    run(async {
        let host = fixture.host();
        let owner = host.mcp_ephemeral_owner().unwrap();
        let deadline = host.mcp_deadline_after(Duration::from_secs(5)).unwrap();
        for raw in [None, Some(b"[]".as_slice())] {
            let receipt = owner
                .replace(
                    NativeMcpEphemeralConfiguration::decode(raw).unwrap(),
                    CancellationToken::new(),
                    deadline,
                )
                .await
                .unwrap();
            owner.ready().unwrap();
            assert!(!receipt.closed_after_publication());
            assert_eq!(fs::read(&profile).unwrap(), b"invalid profile sentinel");
            assert_eq!(
                fs::read(&credentials).unwrap(),
                b"invalid credential sentinel"
            );
        }
        host.settle_mcp_ephemeral(deadline, CancellationToken::new())
            .await
            .unwrap();
    });
}

#[test]
fn ephemeral_hosts_have_distinct_runtime_context_and_owned_close_boundaries() {
    let first = fixture();
    let mut second = fixture();
    let first_owner = first.host().mcp_ephemeral_owner().unwrap();
    let second_host = second.host.take().unwrap();
    let second_owner = second_host.mcp_ephemeral_owner().unwrap();
    assert!(!Arc::ptr_eq(&first_owner, &second_owner));
    assert!(!Arc::ptr_eq(
        &first.host().mcp_contexts().unwrap(),
        &second_host.mcp_contexts().unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &first.host().mcp_runtime().unwrap(),
        &second_host.mcp_runtime().unwrap()
    ));
    let completion = second_host.terminal_shutdown_completion().unwrap();
    run(async {
        let deadline = first
            .host()
            .mcp_deadline_after(Duration::from_secs(5))
            .unwrap();
        for owner in [&first_owner, &second_owner] {
            owner
                .replace(
                    NativeMcpEphemeralConfiguration::decode(None).unwrap(),
                    CancellationToken::new(),
                    deadline,
                )
                .await
                .unwrap();
        }
        let requester = second_host.engine().requester();
        let engine = second_host.into_engine();
        second_owner.ready().unwrap();
        drop(engine);
        assert_eq!(second_owner.ready(), Err(NativeMcpEphemeralError::Closed));
        first_owner.ready().unwrap();
        drop(requester);
        first
            .host()
            .settle_mcp_ephemeral(deadline, CancellationToken::new())
            .await
            .unwrap();
    });
    completion.wait_on_worker().unwrap();
    assert!(completion.is_complete());
}
