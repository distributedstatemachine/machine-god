use super::*;

#[test]
fn early_and_between_stage_notifications_keep_the_complete_set_until_human_binding() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    registry
        .observe(&source, &notification("early"), now)
        .unwrap();
    let ticket = window
        .register_candidates(&["early", "between"], now)
        .unwrap();
    assert!(ticket.waiter.data.deadline.get().is_none());
    assert_eq!(ticket.waiter.data.status.load(Ordering::Acquire), PENDING);
    registry
        .observe(
            &source,
            &notification("between"),
            now + Duration::from_secs(1),
        )
        .unwrap();
    // Verified candidates are not subject to the unverified journal's TTL.
    let human_start = now + Duration::from_secs(86_400);
    let deadline = human_start + Duration::from_secs(1800);
    let exact = Arc::downgrade(&ticket.waiter.data);
    let waiter = ticket.bind(human_start, deadline).unwrap();
    assert!(Arc::ptr_eq(&exact.upgrade().unwrap(), &waiter.data));
    assert_eq!(state::counts(&registry.0), (1, 1, 2, 0));
    let observed = waiter
        .observation(deadline.checked_sub(Duration::from_nanos(1)).unwrap())
        .unwrap()
        .unwrap();
    assert!(observed.belongs_to(&waiter));
    assert_eq!(
        observed.revalidate(deadline),
        Err(McpCompletionError::Deadline)
    );
    assert_eq!(
        waiter.data.deadline.set(deadline + Duration::from_secs(1)),
        Err(deadline + Duration::from_secs(1))
    );
}

#[test]
fn registration_does_not_spend_the_human_budget_and_consent_does_not_reset_it() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let clock = Clock::new(now);
    let ticket = window.register_candidates(&["id"], now).unwrap();
    assert_eq!(clock.reads.load(Ordering::SeqCst), 0);
    clock.advance(Duration::from_secs(7200));
    let human_start = clock.now();
    let deadline = human_start + Duration::from_secs(1800);
    let waiter = ticket.bind(human_start, deadline).unwrap();
    // An actual owner binds before consent; the same budget covers both phases.
    clock.advance(Duration::from_secs(600));
    let mut waiting = waiter.wait(&clock);
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    clock.advance(Duration::from_secs(1199));
    registry
        .observe(&source, &notification("id"), clock.now())
        .unwrap();
    let observation = block_on(waiting).unwrap();
    assert!(observation.belongs_to(&waiter));
    clock.advance(Duration::from_secs(1));
    assert_eq!(
        observation.revalidate(clock.now()),
        Err(McpCompletionError::Deadline)
    );
}

#[test]
fn ticket_early_promotion_preserves_inclusive_ttl_and_nonextending_duplicates() {
    for (seconds, status) in [(600, COMPLETE), (601, PENDING)] {
        let registry = registry();
        let source = McpCompletionSource::new();
        let window = window(&registry, &source);
        let now = Instant::now();
        registry.observe(&source, &notification("id"), now).unwrap();
        registry
            .observe(&source, &notification("id"), now + Duration::from_secs(599))
            .unwrap();
        let at = now + Duration::from_secs(seconds);
        let ticket = window.register_candidates(&["id"], at).unwrap();
        assert_eq!(ticket.waiter.data.status.load(Ordering::Acquire), status);
        let waiter = ticket
            .bind(at + EARLY_TTL, at + EARLY_TTL + Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            waiter.observation(at + EARLY_TTL).unwrap().is_some(),
            status == COMPLETE
        );
    }
}

#[test]
fn ticket_invalid_final_id_and_partial_charge_failure_are_atomic() {
    let limits = McpCompletionLimits::default();
    let baseline = state::base_charge(limits) + state::window_charge(limits);
    let registry = McpCompletionRegistry::new(McpCompletionLimits {
        // Early journal + waiter + all but the final byte of two candidates.
        retained_bytes: baseline + 1024 + (1024 + 2 * 512) + 2 * 1024 - 1,
        ..limits
    })
    .unwrap();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    registry
        .observe(&source, &notification("early"), now)
        .unwrap();
    let oversized = "x".repeat(257);
    for ids in [
        vec!["early", ""],
        vec!["early", &oversized],
        vec!["early", "early"],
        vec!["early", "last"],
    ] {
        assert!(window.register_candidates(&ids, now).is_err());
        assert_eq!(state::counts(&registry.0), (1, 0, 0, 1));
        assert_eq!(registry.retained_bytes(), baseline + 1024);
    }
    let ticket = window.register_candidates(&["early"], now).unwrap();
    assert_eq!(ticket.waiter.data.status.load(Ordering::Acquire), COMPLETE);
}

#[test]
fn unbound_tickets_share_waiter_and_candidate_limits_with_public_waiters() {
    let registry = McpCompletionRegistry::new(McpCompletionLimits {
        waiters: 2,
        candidates: 2,
        ..McpCompletionLimits::default()
    })
    .unwrap();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let first = window.register_candidates(&["first"], now).unwrap();
    let second = register(&window, &["second"], now);
    assert!(matches!(
        window.register_candidates(&["overflow"], now),
        Err(McpCompletionError::Limit)
    ));
    drop(first);
    drop(second);
    assert_eq!(state::counts(&registry.0), (1, 0, 2, 0));
    assert!(matches!(
        window.register_candidates(&["overflow"], now),
        Err(McpCompletionError::Limit)
    ));
    assert!(
        registry
            .observe(&source, &notification("first"), now)
            .unwrap()
            .duplicate
    );
}

#[test]
fn maximum_candidate_set_is_atomic_and_waiter_slots_are_released_on_abandonment() {
    let registry = McpCompletionRegistry::new(McpCompletionLimits {
        waiters: 1,
        ..McpCompletionLimits::default()
    })
    .unwrap();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let ids: Vec<_> = (0..32).map(|index| format!("{index:0256}")).collect();
    let borrowed: Vec<_> = ids.iter().map(String::as_str).collect();
    let ticket = window.register_candidates(&borrowed, now).unwrap();
    assert_eq!(state::counts(&registry.0), (1, 1, 32, 0));
    assert!(matches!(
        window.register_candidates(&["next"], now),
        Err(McpCompletionError::Limit)
    ));
    drop(ticket);
    let next = window.register_candidates(&["next"], now).unwrap();
    assert_eq!(state::counts(&registry.0), (1, 1, 33, 0));
    drop(next);
    assert!(matches!(
        window.register_candidates(&borrowed, now),
        Err(McpCompletionError::Duplicate)
    ));
}

#[test]
fn stale_ticket_binding_and_failed_binding_never_rebind_or_release_retained_charge_early() {
    for kind in 0..5 {
        let registry = registry();
        let source = McpCompletionSource::new();
        let cancel = CancellationToken::new();
        let window = registry
            .open_window(source.clone(), cancel.clone())
            .unwrap();
        let now = Instant::now();
        let ticket = window.register_candidates(&["id"], now).unwrap();
        match kind {
            0 => drop(window),
            1 => registry.invalidate_source(&source),
            2 => registry.close(),
            3 => {
                cancel.cancel();
            }
            _ => {}
        }
        assert!(registry.retained_bytes() >= state::window_charge(registry.0.limits) + 1536);
        let deadline = if kind == 4 {
            now
        } else {
            now + Duration::from_secs(1)
        };
        assert_eq!(
            ticket.bind(now, deadline).unwrap_err(),
            if kind == 4 {
                McpCompletionError::Deadline
            } else {
                McpCompletionError::Cancelled
            }
        );
        assert_eq!(state::counts(&registry.0).1, 0);
    }
}

#[test]
fn expired_public_registration_has_no_ticket_or_tombstone_effects() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    registry.observe(&source, &notification("id"), now).unwrap();
    let bytes = registry.retained_bytes();
    assert!(matches!(
        window.register_ids(&["id"], now, now),
        Err(McpCompletionError::Deadline)
    ));
    assert_eq!(state::counts(&registry.0), (1, 0, 0, 1));
    assert_eq!(registry.retained_bytes(), bytes);
    let ticket = window.register_candidates(&["id"], now).unwrap();
    assert!(!format!("{ticket:?}").contains("\"id\""));
    let waiter = ticket.bind(now, now + Duration::from_secs(1)).unwrap();
    assert!(waiter.observation(now).unwrap().is_some());
}

#[test]
fn unbound_handle_retains_exact_charge_after_source_and_window_removal() {
    let registry = registry();
    let source = McpCompletionSource::new();
    let window = window(&registry, &source);
    let now = Instant::now();
    let ticket = window.register_candidates(&["id"], now).unwrap();
    registry.invalidate_source(&source);
    drop(window);
    assert_eq!(
        registry.retained_bytes(),
        state::base_charge(registry.0.limits) + state::window_charge(registry.0.limits) + 1536
    );
    assert_eq!(state::counts(&registry.0), (0, 0, 0, 0));
    assert_eq!(
        ticket.bind(now, now + Duration::from_secs(1)).unwrap_err(),
        McpCompletionError::Cancelled
    );
    assert_eq!(
        registry.retained_bytes(),
        state::base_charge(registry.0.limits)
    );
}
