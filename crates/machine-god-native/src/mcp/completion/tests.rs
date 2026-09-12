use super::*;
use futures_executor::block_on;
use std::task::{Context, Poll, Waker};

mod tickets;

fn registry() -> McpCompletionRegistry {
    McpCompletionRegistry::new(McpCompletionLimits::default()).unwrap()
}
fn notification(id: &str) -> McpLegacyCompletionNotification {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "jsonrpc":"2.0", "method":"notifications/elicitation/complete",
        "params":{"elicitationId":id}
    }))
    .unwrap();
    McpLegacyCompletionNotification::parse(&bytes)
        .unwrap()
        .unwrap()
}
fn window(registry: &McpCompletionRegistry, source: &McpCompletionSource) -> McpCompletionWindow {
    registry
        .open_window(source.clone(), CancellationToken::new())
        .unwrap()
}
fn register(window: &McpCompletionWindow, ids: &[&str], now: Instant) -> McpCompletionWaiter {
    window
        .register_ids(ids, now, now + Duration::from_secs(1800))
        .unwrap()
}

#[test]
fn producer_notification_classification_is_strict_bounded_and_not_source_authority() {
    assert_eq!(notification("matching-id").elicitation_id(), "matching-id");
    assert!(
        McpLegacyCompletionNotification::parse(
            br#"{"jsonrpc":"2.0","method":"notifications/elicitation/completed","params":{}}"#
        )
        .unwrap()
        .is_none()
    );
    for bytes in [
        br#"{"method":"notifications/elicitation/complete","params":{"elicitationId":"id"}}"#.as_slice(),
        br#"{"jsonrpc":"1.0","method":"notifications/elicitation/complete","params":{"elicitationId":"id"}}"#,
        br#"{"jsonrpc":"2.0","method":"notifications/elicitation/complete"}"#,
        br#"{"jsonrpc":"2.0","method":"notifications/elicitation/complete","params":[]}"#,
        br#"{"jsonrpc":"2.0","method":"notifications/elicitation/complete","params":{"elicitationId":7}}"#,
        br#"{"jsonrpc":"2.0","method":"notifications/elicitation/complete","params":{"elicitationId":""}}"#,
        br#"{"jsonrpc":"2.0","method":"notifications/elicitation/complete","params":{"elicitationId":"id","elicitationId":"id"}}"#,
        br#"{"jsonrpc":"2.0","id":1,"method":"notifications/elicitation/complete","params":{"elicitationId":"id"}}"#,
    ] {
        assert!(McpLegacyCompletionNotification::parse(bytes).is_err());
    }
    assert_eq!(notification(&"x".repeat(256)).elicitation_id().len(), 256);
    let oversized = serde_json::to_vec(&serde_json::json!({
        "jsonrpc":"2.0", "method":"notifications/elicitation/complete",
        "params":{"elicitationId":"x".repeat(257)}
    }))
    .unwrap();
    assert!(McpLegacyCompletionNotification::parse(&oversized).is_err());
    assert!(McpLegacyCompletionNotification::parse(&vec![b' '; 128 * 1024 + 1]).is_err());
    let duplicate_unknown = br#"{"jsonrpc":"2.0","method":"notifications/elicitation/complete","params":{"elicitationId":"id"},"unknown":{"x":1,"x":2}}"#;
    assert!(McpLegacyCompletionNotification::parse(duplicate_unknown).is_err());
}

#[test]
fn exact_source_allocation_and_all_ids_are_required_before_completion() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let foreign = McpCompletionSource::new();
    let now = Instant::now();
    let window = window(&registry, &source);
    let waiter = register(&window, &["first", "second"], now);
    assert_eq!(
        registry
            .observe(&foreign, &notification("first"), now)
            .unwrap(),
        McpCompletionRoute::default()
    );
    registry
        .observe(&source, &notification("second"), now)
        .unwrap();
    assert!(waiter.observation(now).unwrap().is_none());
    assert!(
        registry
            .observe(&source, &notification("second"), now)
            .unwrap()
            .duplicate
    );
    assert_eq!(
        registry
            .observe(&source, &notification("first"), now)
            .unwrap()
            .completed_waiters,
        1
    );
    let observed = waiter.observation(now).unwrap().unwrap();
    assert!(observed.belongs_to(&waiter));
    let another = register(&window, &["another"], now);
    assert!(!observed.belongs_to(&another));
    observed.revalidate(now).unwrap();
    drop(waiter);
    assert_eq!(observed.revalidate(now), Err(McpCompletionError::Cancelled));
}

#[test]
fn early_notifications_route_to_every_overlapping_window_not_the_first_match() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let now = Instant::now();
    assert_eq!(
        registry
            .observe(&source, &notification("unscoped"), now)
            .unwrap()
            .early_windows,
        0
    );
    let first = window(&registry, &source);
    let second = window(&registry, &source);
    assert_eq!(
        registry
            .observe(&source, &notification("a"), now)
            .unwrap()
            .early_windows,
        2
    );
    assert_eq!(
        registry
            .observe(&source, &notification("b"), now)
            .unwrap()
            .early_windows,
        2
    );
    let first_waiter = register(&first, &["a"], now);
    let second_waiter = register(&second, &["b"], now);
    assert!(first_waiter.observation(now).unwrap().is_some());
    assert!(second_waiter.observation(now).unwrap().is_some());
    assert!(matches!(
        second.register_ids(&["a"], now, now + Duration::from_secs(1)),
        Err(McpCompletionError::Duplicate)
    ));
}

#[test]
fn early_ttl_is_inclusive_and_duplicates_do_not_extend_it() {
    for (elapsed, complete) in [(600, true), (601, false)] {
        let registry = registry();
        let source = McpCompletionSource::new();
        let window = window(&registry, &source);
        let now = Instant::now();
        registry.observe(&source, &notification("id"), now).unwrap();
        assert!(
            registry
                .observe(&source, &notification("id"), now + Duration::from_secs(599))
                .unwrap()
                .duplicate
        );
        let at = now + Duration::from_secs(elapsed);
        let waiter = register(&window, &["id"], at);
        assert_eq!(waiter.observation(at).unwrap().is_some(), complete);
    }
}

#[test]
fn early_capacity_evicts_only_its_own_window_and_verified_ids_are_not_ttl_evicted() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    for id in 0..65 {
        registry
            .observe(&source, &notification(&id.to_string()), now)
            .unwrap();
    }
    assert_eq!(state::counts(&registry.0).3, 64);
    let evicted = register(&window, &["0"], now);
    let retained = register(&window, &["64"], now);
    assert!(evicted.observation(now).unwrap().is_none());
    assert!(
        retained
            .observation(now + Duration::from_secs(601))
            .unwrap()
            .is_some()
    );
}

#[test]
fn registration_is_atomic_and_source_wide_duplicate_tombstones_are_bounded() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    for ids in [vec![], vec!["a", "a"], vec![""], vec!["x"; 33]] {
        assert!(
            window
                .register_ids(&ids, now, now + Duration::from_secs(1))
                .is_err()
        );
        assert_eq!(state::counts(&registry.0), (1, 0, 0, 0));
        assert_eq!(
            registry.retained_bytes(),
            state::base_charge(registry.0.limits) + state::window_charge(registry.0.limits)
        );
    }
    for index in 0..1024 {
        let id = index.to_string();
        drop(register(&window, &[&id], now));
    }
    assert_eq!(state::counts(&registry.0).2, 1024);
    assert!(matches!(
        window.register_ids(&["overflow"], now, now + Duration::from_secs(1)),
        Err(McpCompletionError::Limit)
    ));
    registry.invalidate_source(&source);
    assert_eq!(state::counts(&registry.0), (0, 0, 0, 0));
    assert!(matches!(
        registry.open_window(source, CancellationToken::new()),
        Err(McpCompletionError::Cancelled)
    ));
    assert!(
        registry
            .open_window(McpCompletionSource::new(), CancellationToken::new())
            .is_ok()
    );
}

#[test]
fn windows_waiters_and_retained_bytes_have_independent_inclusive_limits() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let now = Instant::now();
    let windows: Vec<_> = (0..32).map(|_| window(&registry, &source)).collect();
    assert!(matches!(
        registry.open_window(source.clone(), CancellationToken::new()),
        Err(McpCompletionError::Limit)
    ));
    let waiters: Vec<_> = (0..32)
        .map(|i| register(&windows[i], &[&i.to_string()], now))
        .collect();
    assert!(matches!(
        windows[0].register_ids(&["extra"], now, now + Duration::from_secs(1)),
        Err(McpCompletionError::Limit)
    ));
    drop(waiters);
    drop(windows);
    assert_eq!(state::counts(&registry.0), (0, 0, 32, 0));
    registry.invalidate_source(&source);
    assert_eq!(
        registry.retained_bytes(),
        state::base_charge(registry.0.limits)
    );

    let limits = McpCompletionLimits::default();
    let budget = state::base_charge(limits) + state::window_charge(limits);
    let registry = McpCompletionRegistry::new(McpCompletionLimits {
        retained_bytes: budget,
        ..limits
    })
    .unwrap();
    let window = window(&registry, &McpCompletionSource::new());
    assert_eq!(registry.retained_bytes(), budget);
    assert!(matches!(
        window.register_ids(&["id"], now, now + Duration::from_secs(1)),
        Err(McpCompletionError::Limit)
    ));
    assert_eq!(state::counts(&registry.0), (1, 0, 0, 0));
    assert_eq!(registry.retained_bytes(), budget);
}

#[test]
fn outstanding_observation_keeps_its_retained_charge_after_source_removal() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let waiter = register(&window, &["id"], now);
    registry.observe(&source, &notification("id"), now).unwrap();
    let observed = waiter.observation(now).unwrap().unwrap();
    registry.invalidate_source(&source);
    drop(waiter);
    drop(window);
    assert_eq!(
        registry.retained_bytes(),
        state::base_charge(registry.0.limits)
            + state::window_charge(registry.0.limits)
            + 1024
            + 512
    );
    drop(observed);
    assert_eq!(
        registry.retained_bytes(),
        state::base_charge(registry.0.limits)
    );
}

struct Clock {
    now: Mutex<Instant>,
    reads: AtomicUsize,
    wake: futures_util::task::AtomicWaker,
}
impl Clock {
    fn new(now: Instant) -> Self {
        Self {
            now: Mutex::new(now),
            reads: AtomicUsize::new(0),
            wake: futures_util::task::AtomicWaker::new(),
        }
    }
    fn advance(&self, duration: Duration) {
        *self.now.lock().unwrap() += duration;
        self.wake.wake();
    }
}
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.reads.fetch_add(1, Ordering::SeqCst);
        *self.now.lock().unwrap()
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

#[test]
fn unpolled_wait_has_no_clock_read_and_exact_expiry_wakes_pending_wait() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let clock = Clock::new(now);
    let waiter = register(&window, &["id"], now);
    drop(waiter.wait(&clock));
    assert_eq!(clock.reads.load(Ordering::SeqCst), 0);
    let mut wait = waiter.wait(&clock);
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    clock.advance(Duration::from_secs(1800));
    assert!(matches!(block_on(wait), Err(McpCompletionError::Deadline)));
    assert_eq!(
        waiter
            .observation(now + Duration::from_secs(1800))
            .unwrap_err(),
        McpCompletionError::Deadline
    );
}

#[test]
fn a_waiter_admits_one_subscription_owner_and_dropping_it_restores_capacity() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let clock = Clock::new(now);
    let waiter = register(&window, &["id"], now);
    let mut first = waiter.wait(&clock);
    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(matches!(
        block_on(waiter.wait(&clock)),
        Err(McpCompletionError::Limit)
    ));
    drop(first);
    let mut next = waiter.wait(&clock);
    assert!(
        next.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    registry.observe(&source, &notification("id"), now).unwrap();
    assert!(block_on(next).unwrap().belongs_to(&waiter));
}

#[test]
fn expired_journal_entries_release_only_their_charge_not_retained_container_capacity() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let baseline = state::base_charge(registry.0.limits) + state::window_charge(registry.0.limits);
    for index in 0..64 {
        registry
            .observe(&source, &notification(&index.to_string()), now)
            .unwrap();
    }
    assert_eq!(registry.retained_bytes(), baseline + 64 * 1024);
    drop(register(
        &window,
        &["fresh"],
        now + Duration::from_secs(601),
    ));
    assert_eq!(state::counts(&registry.0), (1, 0, 1, 0));
    assert_eq!(registry.retained_bytes(), baseline + 1024);
    registry.invalidate_source(&source);
    assert_eq!(registry.retained_bytes(), baseline);
    drop(window);
    assert_eq!(
        registry.retained_bytes(),
        state::base_charge(registry.0.limits)
    );
}

#[test]
fn window_waiter_source_registry_and_operation_cancellation_wake_waits() {
    for kind in 0..5 {
        let registry = registry();
        let source = McpCompletionSource::new();
        let cancellation = CancellationToken::new();
        let window = registry
            .open_window(source.clone(), cancellation.clone())
            .unwrap();
        let now = Instant::now();
        let clock = Clock::new(now);
        let waiter = register(&window, &["id"], now);
        let mut wait = waiter.wait(&clock);
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        match kind {
            0 => drop(window),
            1 => drop(waiter),
            2 => registry.invalidate_source(&source),
            3 => registry.close(),
            _ => {
                cancellation.cancel();
            }
        }
        assert!(matches!(block_on(wait), Err(McpCompletionError::Cancelled)));
    }
}

#[test]
fn notifications_and_invalidation_wake_outside_registry_lock() {
    use machine_god_reentrant_waker_test::{Callback, new};
    for invalidate in [false, true] {
        for callback in [Callback::Clone, Callback::Wake, Callback::Drop] {
            let registry = registry();
            let source = McpCompletionSource::new();
            let window = window(&registry, &source);
            let now = Instant::now();
            let clock = Clock::new(now);
            let waiter = register(&window, &["id"], now);
            let mut wait = waiter.wait(&clock);
            let inner = registry.0.clone();
            let (waker, calls) = new(callback, move || {
                assert!(inner.state.try_lock().is_ok());
            });
            assert!(
                wait.as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            if invalidate {
                registry.invalidate_source(&source);
            } else {
                registry.observe(&source, &notification("id"), now).unwrap();
            }
            let result = block_on(wait);
            assert_eq!(result.is_ok(), !invalidate);
            drop(waker);
            assert!(calls.calls() > 0);
        }
    }
}

#[test]
fn sequence_exhaustion_and_rejected_limits_do_not_mutate_existing_owners() {
    assert!(
        McpCompletionRegistry::new(McpCompletionLimits {
            windows: 0,
            ..McpCompletionLimits::default()
        })
        .is_err()
    );
    assert!(
        McpCompletionRegistry::new(McpCompletionLimits {
            candidates: 1025,
            ..McpCompletionLimits::default()
        })
        .is_err()
    );
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    state::exhaust_sequence(&registry.0);
    assert!(matches!(
        registry.open_window(source, CancellationToken::new()),
        Err(McpCompletionError::Limit)
    ));
    let now = Instant::now();
    assert!(matches!(
        window.register_ids(&["id"], now, now + Duration::from_secs(1)),
        Err(McpCompletionError::Limit)
    ));
    assert_eq!(state::counts(&registry.0), (1, 0, 0, 0));
}

#[test]
fn debug_and_errors_do_not_include_notification_data() {
    let value = notification("private-completion-id");
    assert!(!format!("{value:?}").contains("private-completion-id"));
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let waiter = register(&window, &["private-completion-id"], now);
    registry.observe(&source, &value, now).unwrap();
    let observed = waiter.observation(now).unwrap().unwrap();
    assert!(
        !format!("{registry:?} {source:?} {window:?} {waiter:?} {observed:?}")
            .contains("private-completion-id")
    );
}
