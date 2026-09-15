use super::*;
use crate::mcp::{
    controller::{state::WorkerReservation, tests::Fixture},
    lifetime::McpPeerLifetime,
    runtime::NativeMcpRuntimeClock,
};
use futures_util::task::AtomicWaker;
use std::{
    task::{Context, Poll, Waker},
    time::Duration,
};

struct Clock {
    now: Mutex<Instant>,
    wake: AtomicWaker,
}
impl Clock {
    fn advance(&self, duration: Duration) {
        *lock(&self.now) += duration;
        self.wake.wake();
    }
}
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        *lock(&self.now)
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::poll_fn(move |cx| {
            self.wake.register(cx.waker());
            if self.now() >= deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}
fn configured() -> (Fixture, Arc<Clock>) {
    let mut fixture = Fixture::new();
    let clock = Arc::new(Clock {
        now: Mutex::new(Instant::now()),
        wake: AtomicWaker::new(),
    });
    fixture.options.startup.clock = clock.clone();
    fixture.options.startup.catalog_epoch = clock.now();
    fixture.options.startup.peer_lifetime = McpPeerLifetime::OwnerControlled;
    (fixture, clock)
}

#[test]
fn housekeeping_has_fresh_finite_deadlines_independent_of_configured_startup_age() {
    let (fixture, clock) = configured();
    let now = clock.now();
    assert_eq!(
        budget::housekeeping_deadline(&fixture.options, None)
            .unwrap_or_else(|_| panic!("valid window")),
        now + Duration::from_secs(30)
    );
    assert_eq!(
        budget::housekeeping_deadline(&fixture.options, Some(now + Duration::from_secs(2)))
            .unwrap_or_else(|_| panic!("valid window")),
        now + Duration::from_secs(2)
    );
    clock.advance(Duration::from_secs(3600));
    assert_eq!(
        budget::housekeeping_deadline(&fixture.options, None)
            .unwrap_or_else(|_| panic!("valid window")),
        now + Duration::from_secs(3630)
    );
}

#[test]
fn configured_startup_remains_inert_then_uses_current_housekeeping_windows() {
    let (fixture, clock) = configured();
    let controller = fixture.controller();
    let future = controller.start_configured(NativeMcpStartupPhase::All, CancellationToken::new());
    assert!(lock(&controller.inner.state).generations.is_empty());
    clock.advance(Duration::from_secs(86_400));
    let receipt = futures_executor::block_on(future).unwrap();
    assert_eq!(
        receipt.publication(),
        NativeMcpControllerPublication::Published
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        futures_executor::block_on(controller.reload_configured(cancelled))
            .unwrap_err()
            .kind(),
        NativeMcpControllerError::Cancelled
    );
    controller.close();
}

#[test]
fn failed_initial_activation_is_not_forgotten_when_its_generation_is_pruned() {
    let (fixture, _) = configured();
    fixture.seed("invalid configuration");
    let controller = fixture.controller();
    let mut startup =
        controller.start_configured(NativeMcpStartupPhase::All, CancellationToken::new());
    assert!(controller.needs_initial_startup());
    let cohort = fixture.options.workers.begin_run().unwrap();
    assert!(
        futures_executor::block_on(std::future::poll_fn(|cx| {
            cohort.with_poll(|| startup.as_mut().poll(cx))
        }))
        .is_err()
    );
    drop(startup);
    cohort.close();
    cohort.completion().wait_on_worker().unwrap();
    controller.inner.prune();
    assert!(lock(&controller.inner.state).active.is_none());
    assert!(!controller.needs_initial_startup());
    assert!(lock(&controller.inner.state).generations.is_empty());
    assert!(controller.activation_failure().is_some());
    let cleanup = futures_executor::block_on(controller.settle_failed_startup(
        controller.deadline_after(Duration::from_secs(5)).unwrap(),
        CancellationToken::new(),
        Some(cohort.completion()),
    ))
    .unwrap();
    assert!(cleanup.complete);
    assert!(!lock(&controller.inner.state).closed);
    fixture.seed(r#"{"mcp":{}}"#);
    // Recovery is an explicit reload, not another admission's automatic start.
    let receipt =
        futures_executor::block_on(controller.reload_configured(CancellationToken::new())).unwrap();
    assert_eq!(
        receipt.publication(),
        NativeMcpControllerPublication::Published
    );
    assert!(!controller.needs_initial_startup());
    assert!(controller.activation_failure().is_none());
    assert_eq!(
        futures_executor::block_on(controller.settle_failed_startup(
            controller.deadline_after(Duration::from_secs(5)).unwrap(),
            CancellationToken::new(),
            None,
        ))
        .unwrap_err()
        .kind(),
        NativeMcpControllerError::Invalid
    );
    assert!(lock(&controller.inner.state).active.is_some());
    assert!(!lock(&controller.inner.state).closed);
    controller.close();
}

#[test]
fn existing_outer_deadline_and_explicit_peer_expiry_still_reject_before_loading() {
    let (mut fixture, clock) = configured();
    let controller = fixture.controller();
    assert_eq!(
        futures_executor::block_on(controller.start(
            NativeMcpStartupPhase::All,
            CancellationToken::new(),
            clock.now()
        ))
        .unwrap_err()
        .kind(),
        NativeMcpControllerError::Deadline
    );
    assert!(lock(&controller.inner.state).generations.is_empty());
    fixture.options.startup.peer_lifetime = McpPeerLifetime::Until(clock.now());
    let expiring = fixture.controller();
    assert_eq!(
        futures_executor::block_on(
            expiring.start_configured(NativeMcpStartupPhase::All, CancellationToken::new())
        )
        .unwrap_err()
        .kind(),
        NativeMcpControllerError::Deadline
    );
    assert!(lock(&expiring.inner.state).generations.is_empty());
}

#[test]
fn timed_out_housekeeping_keeps_actual_worker_generation_custody() {
    let (fixture, clock) = configured();
    let controller = fixture.controller();
    let receipt = futures_executor::block_on(
        controller.start_configured(NativeMcpStartupPhase::All, CancellationToken::new()),
    )
    .unwrap();
    let generation = lock(&controller.inner.state).active.clone().unwrap();
    let reservation = WorkerReservation::new(&generation);
    let (entered, observed) = std::sync::mpsc::sync_channel(1);
    let (release, waiting) = std::sync::mpsc::sync_channel(1);
    let operation = fixture.options.workers.run(move || {
        let _reservation = reservation;
        entered.send(()).unwrap();
        waiting.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    let mut stage = Box::pin(budget::housekeeping(&fixture.options, None, operation));
    assert!(
        stage
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    observed.recv_timeout(Duration::from_secs(10)).unwrap();
    clock.advance(Duration::from_secs(31));
    assert!(matches!(
        stage.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(Failure {
            kind: NativeMcpControllerError::Deadline,
            ..
        }))
    ));
    drop(stage);
    assert!(!generation.cleanup_complete());
    assert!(!receipt.cleanup_complete());
    controller.close();
    release.send(()).unwrap();
    fixture.options.workers.close();
    fixture
        .options
        .workers
        .completion()
        .wait_on_worker()
        .unwrap();
    assert!(generation.cleanup_complete());
}
