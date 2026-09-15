use super::*;
use machine_god_native::{NativeManagedInteractiveStartup, NativeSessionOrigin};

async fn setup() -> (support::Fixture, Harness) {
    let inbox =
        NativeInteractivePromptInbox::new(NativeInteractivePromptLimits::default()).unwrap();
    let mut fixture = support::Fixture::with_workspace_options(|options| {
        options.with_managed_agents(super::super::super::managed_startup::options(&inbox))
    });
    let state = File::open(fixture.state_root()).unwrap();
    let host = Arc::get_mut(&mut fixture.host).unwrap();
    let preferences = host.loaded_config().config().model_preferences();
    let agents = host
        .open_workspace_managed_agents(state.into(), preferences.clone(), NativeSessionOrigin::Cli)
        .await
        .unwrap();
    let options =
        NativeInteractiveSessionOptions::new(fixture.workspace.clone(), preferences).unwrap();
    let startup =
        NativeManagedInteractiveStartup::new(fixture.host.clone(), options, agents).unwrap();
    let mut harness = harness_with_inbox(&fixture, inbox).await;
    harness.startup.managed = Some(startup);
    (fixture, harness)
}

#[test]
fn managed_picker_cancel_retains_unpolled_open_until_native_shutdown() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (fixture, mut harness) = runtime.block_on(setup());
    runtime.block_on(async {
        picker_ready(&mut harness).await;
        assert_eq!(count(&fixture).await, 0);
        harness.startup.open(NativeInteractiveInitialSession::Fresh);
        assert!(harness.startup.opening());
        harness.signal.send(AskSignal::Interrupt).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| harness.startup.poll(cx, &mut harness.signals)),
        )
        .await
        .unwrap();
        assert!(harness.startup.managed.as_ref().unwrap().is_finished());
        assert!(harness.startup.owner.is_none());
        assert_eq!(count(&fixture).await, 0);
        assert!(fixture.transport.requests().is_empty());
    });
    let completion = harness.startup.input.input.completion();
    drop(harness);
    completion.wait_on_worker().unwrap();
    fixture.finish();
}

#[test]
fn managed_picker_signal_closes_transferred_owner_under_blocked_output() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (fixture, mut harness) = runtime.block_on(setup());
    runtime.block_on(async {
        picker_ready(&mut harness).await;
        harness.startup.open(NativeInteractiveInitialSession::Fresh);
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                assert!(harness.startup.poll(cx, &mut harness.signals).is_pending());
                if harness.startup.owner.is_some() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(
            harness
                .startup
                .owner
                .as_ref()
                .unwrap()
                .manages_prompt_inbox(&harness.inbox)
                .unwrap()
        );
        assert!(harness.startup.in_flight.is_some());
        harness.signal.send(AskSignal::Interrupt).await.unwrap();
        // Deliberately do not drain or acknowledge stdout while native cleanup runs.
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| harness.startup.poll(cx, &mut harness.signals)),
        )
        .await
        .unwrap();
        assert!(harness.startup.owner.as_ref().unwrap().is_closed());
        assert!(fixture.transport.requests().is_empty());
    });
    let completion = harness.startup.input.input.completion();
    drop(harness);
    completion.wait_on_worker().unwrap();
    fixture.finish();
}

#[test]
fn managed_picker_handoff_and_display_refresh_keep_the_native_prompt_lease() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (fixture, mut harness) = runtime.block_on(setup());
    runtime.block_on(async {
        picker_ready(&mut harness).await;
        harness.startup.open(NativeInteractiveInitialSession::Fresh);
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let result = harness.startup.poll(cx, &mut harness.signals);
                acknowledge(&mut harness, cx);
                result
            }),
        )
        .await
        .unwrap();
        let Ok(mut driver) = harness.startup.into_result(harness.inbox).unwrap() else {
            panic!("managed session must transfer to the driver");
        };
        assert!(driver.prompt_principal.is_none());
        driver.retire_prompt_principal();
        driver.register_prompt_principal().unwrap();
        assert!(driver.prompt_principal.is_none());
        assert!(
            driver
                .inbox
                .register(super::super::super::principal(&driver.owner))
                .is_err()
        );
        driver.shutdown();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let result = driver.poll(cx, &mut harness.signals);
                if harness.work.try_recv().is_ok() {
                    harness
                        .ack
                        .try_send(OutputAcknowledgement::Succeeded)
                        .unwrap();
                    cx.waker().wake_by_ref();
                }
                result
            }),
        )
        .await
        .unwrap();
        assert!(driver.owner.is_closed());
        assert!(fixture.transport.requests().is_empty());
        drop(driver);
    });
    fixture.finish();
}
