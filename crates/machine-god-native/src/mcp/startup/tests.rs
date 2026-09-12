use super::*;
use crate::mcp::{
    context::NativeMcpContexts,
    runtime::{
        NativeMcpRuntime, NativeMcpRuntimeLimits, NativeMcpRuntimeToolCall,
        NativeMcpToolExecutionPolicy, NativeMcpToolExecutor,
    },
};
use futures_executor::block_on;
use machine_god_core::{ToolError, ToolExecution};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

#[cfg(feature = "mcp-http")]
mod http;

#[derive(Default)]
struct Clock(AtomicUsize);
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.0.fetch_add(1, Ordering::Relaxed);
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
fn options(configuration: &str) -> NativeMcpStartupOptions {
    NativeMcpStartupOptions {
        configuration: Arc::new(McpConfig::decode(configuration.as_bytes()).unwrap()),
        captured_environment: vec![],
        stdio: None,
        workers: NativeOwnedWorkerScope::new(),
        clock: Arc::new(Clock::default()),
        catalog_epoch: Instant::now(),
        owner_cancellation: CancellationToken::new(),
        configuration_cancellation: CancellationToken::new(),
        #[cfg(feature = "mcp-http")]
        network: None,
        #[cfg(feature = "mcp-http")]
        authentication: vec![],
        peer_lifetime: McpPeerLifetime::Until(Instant::now() + Duration::from_secs(30)),
        max_retained_bytes: MAX_RETAINED_BYTES,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
struct NeverExecute;
impl NativeMcpToolExecutor for NeverExecute {
    fn execute(
        &self,
        _: NativeMcpRuntimeToolCall,
    ) -> BoxFuture<'_, std::result::Result<ToolExecution, ToolError>> {
        panic!("startup must never execute an application tool")
    }
}
fn runtime() -> NativeMcpRuntime {
    NativeMcpRuntime::new(
        Arc::new(NativeMcpContexts::new()),
        Arc::new(Clock::default()),
        Arc::new(NeverExecute),
        NativeMcpToolExecutionPolicy::default(),
        NativeMcpRuntimeLimits::default(),
    )
    .unwrap()
}

#[test]
fn construction_and_unpolled_build_do_not_read_clock_or_admit_workers() {
    let mut selected =
        options(r#"{"mcp":{"selected":{"command":"/missing/server","required":true}}}"#);
    let clock = Arc::new(Clock::default());
    selected.clock = clock.clone();
    let workers = selected.workers.clone();
    let completion = workers.completion();
    let startup = NativeMcpStartup::new(selected).unwrap();
    drop(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    assert_eq!(clock.0.load(Ordering::Relaxed), 0);
    assert!(startup.cleanup_observations().is_empty());
    workers.close();
    assert!(completion.is_complete());
}

#[test]
fn a_future_catalog_origin_rejects_even_an_empty_build_before_effects() {
    let mut selected = options(r#"{"mcp":{}}"#);
    selected.catalog_epoch = Instant::now() + Duration::from_secs(3600);
    let startup = NativeMcpStartup::new(selected).unwrap();
    let batch = block_on(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    assert_eq!(batch.receipt.failure, Some(NativeMcpStartupError::Invalid));
    assert!(batch.servers().is_empty());
    assert!(startup.cleanup_observations().is_empty());
}

#[test]
fn required_disabled_and_optional_deferred_are_distinct_without_process_authority() {
    let startup = NativeMcpStartup::new(options(r#"{"mcp":{"required":{"command":"/missing/required","required":true},"optional":{"command":"/missing/optional"},"disabled":{"command":"/missing/disabled","required":true,"enabled":false}}}"#)).unwrap();
    let batch = block_on(startup.build(
        NativeMcpStartupPhase::AskStartup,
        CancellationToken::new(),
        deadline(),
    ));
    assert_eq!(
        batch.receipt.servers[0].state,
        NativeMcpStartupState::Failed(NativeMcpStartupError::Unavailable)
    );
    assert_eq!(
        batch.receipt.servers[1].state,
        NativeMcpStartupState::Deferred
    );
    assert_eq!(
        batch.receipt.servers[2].state,
        NativeMcpStartupState::Disabled
    );
    assert!(!batch.receipt.required_ready());
    assert!(batch.receipt.cleanup_complete());
    assert!(batch.servers().is_empty());
    let failure = batch
        .prepare(&runtime(), &[], NativeMcpStartupRequirement::Required)
        .unwrap_err();
    assert_eq!(failure.error, NativeMcpStartupError::RequiredUnavailable);
}

#[test]
fn deferred_batch_cannot_replace_required_publication_or_prove_required_readiness() {
    let startup = NativeMcpStartup::new(options(
        r#"{"mcp":{"required":{"command":"/missing/server","required":true}}}"#,
    ))
    .unwrap();
    let batch = block_on(startup.build(
        NativeMcpStartupPhase::AskDeferred,
        CancellationToken::new(),
        deadline(),
    ));
    assert!(!batch.receipt().required_ready());
    let failure = batch
        .prepare(&runtime(), &[], NativeMcpStartupRequirement::Required)
        .unwrap_err();
    assert_eq!(failure.error, NativeMcpStartupError::DeferredBatch);
    let batch = block_on(startup.build(
        NativeMcpStartupPhase::AskDeferred,
        CancellationToken::new(),
        deadline(),
    ));
    let (servers, receipt) = batch.into_deferred_servers().unwrap();
    assert!(servers.is_empty());
    assert_eq!(receipt.servers[0].state, NativeMcpStartupState::Deferred);
}

#[test]
fn one_pending_batch_and_explicit_optional_failure_policy() {
    let startup = NativeMcpStartup::new(options(
        r#"{"mcp":{"optional":{"command":"/missing/server"}}}"#,
    ))
    .unwrap();
    let batch = block_on(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    assert!(batch.receipt().required_ready());
    assert!(batch.receipt().has_failures());
    let blocked = block_on(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    assert_eq!(
        blocked.receipt().failure,
        Some(NativeMcpStartupError::Unavailable)
    );
    assert!(blocked.servers().is_empty());
    drop(blocked);
    let failure = batch
        .prepare(&runtime(), &[], NativeMcpStartupRequirement::AllSelected)
        .unwrap_err();
    assert_eq!(failure.error, NativeMcpStartupError::Unavailable);
    let batch = block_on(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    let (candidate, receipt) = batch
        .prepare(&runtime(), &[], NativeMcpStartupRequirement::Required)
        .unwrap();
    assert!(candidate.descriptors().servers().is_empty());
    assert!(receipt.has_failures());
}

#[test]
fn cancellation_deadline_and_aggregate_bounds_precede_transport_admission() {
    let mut selected = options(r#"{"mcp":{"server":{"command":"/missing/server"}}}"#);
    selected.max_retained_bytes = 1;
    let startup = NativeMcpStartup::new(selected).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let batch = block_on(startup.build(NativeMcpStartupPhase::All, cancellation, deadline()));
    assert_eq!(
        batch.receipt().failure,
        Some(NativeMcpStartupError::Cancelled)
    );
    assert_eq!(batch.receipt().servers[0].attempts, 0);
    drop(batch);
    let batch = block_on(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        Instant::now(),
    ));
    assert_eq!(
        batch.receipt().failure,
        Some(NativeMcpStartupError::Deadline)
    );
    drop(batch);
    let batch = block_on(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    assert_eq!(
        batch.receipt().servers[0].state,
        NativeMcpStartupState::Failed(NativeMcpStartupError::Limit)
    );
    assert_eq!(batch.receipt().servers[0].attempts, 0);
    assert!(batch.receipt().cleanup_complete());
}

#[test]
fn completion_cap_rejects_before_acquisition_and_retains_pending_observations() {
    use crate::mcp::runtime::NativeMcpPeerCompletion;
    let completion = NativeMcpStartupCompletion::new();
    let mut scopes = Vec::new();
    for _ in 0..4 {
        let scope = NativeOwnedWorkerScope::new();
        assert!(completion.record(NativeMcpPeerCompletion::Stdio(scope.completion())));
        scopes.push(scope);
    }
    let excess = NativeOwnedWorkerScope::new();
    assert!(!completion.record(NativeMcpPeerCompletion::Stdio(excess.completion())));
    completion.seal();
    assert!(!completion.is_complete());
    for scope in scopes {
        scope.close();
    }
    assert!(completion.is_complete());
    excess.close();
    assert!(!completion.record(NativeMcpPeerCompletion::Stdio(excess.completion())));
}

#[test]
fn empty_or_unselected_configuration_still_observes_cancellation_and_deadline() {
    for configuration in [
        r#"{"mcp":{}}"#,
        r#"{"mcp":{"optional":{"command":"/missing/server"}}}"#,
    ] {
        let startup = NativeMcpStartup::new(options(configuration)).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let batch =
            block_on(startup.build(NativeMcpStartupPhase::AskStartup, cancellation, deadline()));
        assert_eq!(
            batch.receipt().failure,
            Some(NativeMcpStartupError::Cancelled)
        );
        assert!(!batch.receipt().required_ready());
        drop(batch);
        let batch = block_on(startup.build(
            NativeMcpStartupPhase::AskStartup,
            CancellationToken::new(),
            Instant::now(),
        ));
        assert_eq!(
            batch.receipt().failure,
            Some(NativeMcpStartupError::Deadline)
        );
    }
}
