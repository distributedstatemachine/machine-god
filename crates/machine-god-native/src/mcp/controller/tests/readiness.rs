use super::*;

#[test]
fn required_readiness_failed_start_retains_exact_observation_and_reload_recovers() {
    run(async {
        let fixture = Fixture::new();
        fixture.seed(r#"{"mcp":{"required":{"command":"unexecuted","required":true}}}"#);
        let controller = fixture.controller();
        assert!(controller.required_readiness().is_err());
        let failure = controller
            .start_configured(NativeMcpStartupPhase::All, CancellationToken::new())
            .await
            .unwrap_err();
        let observed = lock(&controller.inner.state)
            .latest_observed
            .clone()
            .unwrap();
        assert!(observed.cancellation.is_cancelled());
        assert!(
            lock(&observed.loaded)
                .as_ref()
                .unwrap()
                .snapshot
                .config()
                .server("required")
                .is_some()
        );
        drop(failure);
        controller.inner.prune();
        assert!(lock(&controller.inner.state).latest_observed.is_some());
        assert!(controller.required_readiness().is_err());
        fixture.seed(r#"{"mcp":{}}"#);
        let receipt = controller
            .reload_configured(CancellationToken::new())
            .await
            .unwrap();
        assert!(controller.required_readiness().is_ok());
        assert!(!Arc::ptr_eq(
            lock(&controller.inner.state)
                .latest_observed
                .as_ref()
                .unwrap(),
            &observed
        ));
        controller.close();
        assert!(controller.required_readiness().is_err());
        assert!(lock(&controller.inner.state).latest_observed.is_none());
        drop(receipt);
        drop(observed);
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}

#[test]
fn required_readiness_failed_reload_preserves_active_and_bounded_observation() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        drop(
            controller
                .start_configured(NativeMcpStartupPhase::AskStartup, CancellationToken::new())
                .await
                .unwrap(),
        );
        fixture.seed(
            r#"{"mcp":{"required":{"command":"unexecuted","required":true,"enabled":false}}}"#,
        );
        for _ in 0..8 {
            drop(
                controller
                    .reload_configured(CancellationToken::new())
                    .await
                    .unwrap_err(),
            );
            assert!(controller.required_readiness().is_ok());
            controller.inner.prune();
            assert!(lock(&controller.inner.state).generations.len() <= 2);
        }
        assert!(
            controller
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}
