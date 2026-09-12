use super::super::{
    NativeMcpControllerError, NativeMcpStartupPhase, configuration, state::Loaded, tests::Fixture,
};
use super::*;
use machine_god_core::CancellationToken;
use std::sync::Mutex;

fn selected(fixture: &Fixture) -> (NativeMcpController, Arc<Generation>) {
    fixture.seed(r#"{"mcp":{}}"#);
    let controller = fixture.controller();
    let snapshot = Arc::new(fixture.options.management.config_store().load().unwrap());
    let cancellation = CancellationToken::new();
    let startup =
        configuration::startup(&controller.inner.options, &snapshot, cancellation.clone()).unwrap();
    fixture
        .options
        .runtime
        .publish(
            fixture
                .options
                .runtime
                .prepare_candidate(vec![], &[])
                .unwrap(),
        )
        .unwrap();
    let generation = Arc::new(Generation {
        phase: NativeMcpStartupPhase::AskStartup,
        cancellation,
        loaded: Mutex::new(Some(Loaded {
            snapshot,
            startup,
            checkpoint: Some(fixture.options.runtime.publication_checkpoint().unwrap()),
        })),
        deferred: Mutex::new(None),
        workers: std::sync::atomic::AtomicUsize::new(0),
    });
    lock(&controller.inner.state).active = Some(generation.clone());
    (controller, generation)
}

#[test]
fn exact_handoff_updates_readiness_and_rejects_mutation_during_callback() {
    let fixture = Fixture::new();
    let (controller, generation) = selected(&fixture);
    let expected = fixture.options.runtime.publication_checkpoint().unwrap();
    let candidate = fixture
        .options
        .runtime
        .prepare_candidate(vec![], &[])
        .unwrap();
    let prospective = candidate.publication_checkpoint();
    let committed = controller
        .sync_catalog_publication(&expected, &prospective, || {
            assert_eq!(
                controller.required_readiness().unwrap_err().kind(),
                NativeMcpControllerError::Busy
            );
            let result = futures_executor::block_on(controller.reload(
                CancellationToken::new(),
                std::time::Instant::now() + std::time::Duration::from_secs(1),
            ));
            assert_eq!(result.unwrap_err().kind(), NativeMcpControllerError::Busy);
            fixture.options.runtime.publish_if(candidate, &expected)?;
            Ok(prospective.clone())
        })
        .unwrap();
    assert!(committed.same_selection(&prospective));
    assert!(
        lock(&generation.loaded)
            .as_ref()
            .unwrap()
            .checkpoint
            .as_ref()
            .unwrap()
            .same_selection(&prospective)
    );
    assert!(!lock(&controller.inner.state).catalog_handoff);
    controller.required_readiness().unwrap();
}

#[test]
fn failed_or_foreign_handoff_does_not_change_controller_selection() {
    let fixture = Fixture::new();
    let (controller, generation) = selected(&fixture);
    let expected = fixture.options.runtime.publication_checkpoint().unwrap();
    let prospective = fixture
        .options
        .runtime
        .prepare_candidate(vec![], &[])
        .unwrap()
        .publication_checkpoint();
    assert!(matches!(
        controller.sync_catalog_publication(&expected, &prospective, || Err(Error::Limit)),
        Err(Error::Limit)
    ));
    assert!(!lock(&controller.inner.state).catalog_handoff);
    assert!(
        lock(&generation.loaded)
            .as_ref()
            .unwrap()
            .checkpoint
            .as_ref()
            .unwrap()
            .same_selection(&expected)
    );
    assert!(matches!(
        controller
            .sync_catalog_publication(&prospective, &expected, || panic!("stale handoff invoked")),
        Err(Error::Unavailable)
    ));
    let foreign = Fixture::new()
        .options
        .runtime
        .publication_checkpoint()
        .unwrap();
    assert!(matches!(
        controller
            .sync_catalog_publication(&expected, &foreign, || panic!("foreign handoff invoked")),
        Err(Error::Invalid)
    ));
}

#[test]
fn reentrant_close_after_commit_keeps_success_without_reactivating_generation() {
    let fixture = Fixture::new();
    let (controller, generation) = selected(&fixture);
    let expected = fixture.options.runtime.publication_checkpoint().unwrap();
    let candidate = fixture
        .options
        .runtime
        .prepare_candidate(vec![], &[])
        .unwrap();
    let prospective = candidate.publication_checkpoint();
    let result = controller
        .sync_catalog_publication(&expected, &prospective, || {
            fixture.options.runtime.publish_if(candidate, &expected)?;
            controller.close();
            Ok(prospective.clone())
        })
        .unwrap();
    assert!(result.same_selection(&prospective));
    let state = lock(&controller.inner.state);
    assert!(state.closed);
    assert!(state.active.is_none());
    assert!(!state.catalog_handoff);
    assert!(
        lock(&generation.loaded)
            .as_ref()
            .unwrap()
            .checkpoint
            .as_ref()
            .unwrap()
            .same_selection(&prospective)
    );
}
