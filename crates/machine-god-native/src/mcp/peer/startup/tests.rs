use super::*;
mod lifetime;

#[test]
fn every_configured_restart_retains_a_fresh_maximum_attempt_budget() {
    let timeout = Duration::from_millis(u64::from(u32::MAX));
    let startup = Startup {
        deadline: None,
        timeout,
        observer: Some(Arc::new(|_| false)),
    };
    let origin = Instant::now();
    let mut now = origin;
    for _ in 0..=u8::MAX {
        assert_eq!(startup.attempt_deadline(now).unwrap(), now + timeout);
        now += timeout;
    }
    assert!(now > origin + timeout + timeout);
}

#[test]
fn configured_deadlines_are_checked_bounded_and_independent_of_legacy_defaults() {
    let now = Instant::now();
    let maximum = Duration::from_millis(u64::from(u32::MAX));
    let mut startup = Startup {
        deadline: Some(now + maximum),
        timeout: maximum,
        observer: Some(Arc::new(|_| false)),
    };
    startup.validate(now).unwrap();
    assert_eq!(
        Some(startup.attempt_deadline(now).unwrap()),
        startup.deadline
    );
    startup.timeout += Duration::from_millis(1);
    assert!(startup.validate(now).is_err());
    startup.timeout = Duration::ZERO;
    assert!(startup.validate(now).is_err());
    startup.timeout = Duration::from_secs(1);
    assert_eq!(
        startup.attempt_deadline(now).unwrap(),
        now + startup.timeout
    );
    startup.deadline = Some(now + Duration::from_millis(500));
    assert_eq!(
        Some(startup.attempt_deadline(now).unwrap()),
        startup.deadline
    );
    startup.observer = None;
    startup.deadline = Some(now + Duration::from_secs(301));
    assert!(startup.validate(now).is_err());
    startup.deadline = Some(now + Duration::from_secs(300));
    startup.validate(now).unwrap();
    assert_eq!(
        Some(startup.attempt_deadline(now).unwrap()),
        startup.deadline
    );
}

#[test]
fn configured_attempts_keep_the_full_timeout_without_an_overall_cap() {
    let origin = Instant::now();
    let maximum = Duration::from_millis(u64::from(u32::MAX));
    let startup = Startup {
        deadline: None,
        timeout: maximum,
        observer: Some(Arc::new(|_| false)),
    };
    startup.validate(origin).unwrap();
    assert_eq!(startup.attempt_deadline(origin).unwrap(), origin + maximum);
    let after_previous_attempt = origin + maximum + Duration::from_secs(30);
    assert_eq!(
        startup.attempt_deadline(after_previous_attempt).unwrap(),
        after_previous_attempt + maximum
    );
    assert_eq!(
        startup.cleanup_deadline(origin).unwrap(),
        origin + Duration::from_secs(30)
    );
    let bounded = Startup {
        deadline: Some(origin),
        ..startup
    };
    assert_eq!(
        bounded.cleanup_deadline(after_previous_attempt).unwrap(),
        origin
    );
    assert_eq!(
        bounded.attempt_deadline(after_previous_attempt).unwrap(),
        origin
    );
}

#[test]
fn configured_owner_cancellation_precedes_any_factory_effect() {
    struct Timer;
    impl McpPeerTimer for Timer {
        fn sleep_until(&self, _: Instant) -> machine_god_core::BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }
    let host = NativeOwnedWorkerScope::new();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut factory = || panic!("cancelled startup cannot acquire a launch");
    let future = McpStdioPeer::connect_configured_observed(
        &mut factory,
        host.clone(),
        Arc::new(Timer),
        cancellation,
        Duration::from_millis(u64::from(u32::MAX)),
        Arc::new(|_| panic!("cancelled startup cannot observe")),
    );
    assert!(matches!(
        futures_executor::block_on(future),
        Err(McpPeerError::Cancelled)
    ));
    host.close();
    assert!(host.completion().is_complete());
}

#[test]
fn configured_peer_is_inert_and_never_retries_factory_failure() {
    struct Timer;
    impl McpPeerTimer for Timer {
        fn sleep_until(&self, _: Instant) -> machine_god_core::BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }
    let mut calls = 0;
    let mut factory = || {
        calls += 1;
        Err(McpStdioError::Process)
    };
    let host = NativeOwnedWorkerScope::new();
    let deadline = Instant::now() + Duration::from_secs(600);
    drop(McpStdioPeer::connect_observed(
        &mut factory,
        host.clone(),
        Arc::new(Timer),
        CancellationToken::new(),
        deadline,
        Duration::from_secs(600),
        Arc::new(|_| panic!("no transport")),
    ));
    assert!(matches!(
        futures_executor::block_on(McpStdioPeer::connect_observed(
            &mut factory,
            host.clone(),
            Arc::new(Timer),
            CancellationToken::new(),
            deadline,
            Duration::from_secs(600),
            Arc::new(|_| panic!("no transport"))
        )),
        Err(McpPeerError::Transport(McpStdioError::Process))
    ));
    assert_eq!(calls, 1);
    host.close();
    assert!(host.completion().is_complete());
}

#[test]
fn selected_clock_expiry_precedes_factory_and_observer() {
    struct Elapsed(Instant);
    impl McpPeerTimer for Elapsed {
        fn now(&self) -> Instant {
            self.0
        }
        fn sleep_until(&self, _: Instant) -> machine_god_core::BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let host = NativeOwnedWorkerScope::new();
    assert!(matches!(
        futures_executor::block_on(McpStdioPeer::connect_observed(
            &mut || panic!("expired startup cannot acquire launch"),
            host.clone(),
            Arc::new(Elapsed(deadline)),
            CancellationToken::new(),
            deadline,
            Duration::from_secs(1),
            Arc::new(|_| panic!("expired startup cannot observe"))
        )),
        Err(McpPeerError::Deadline)
    ));
    host.close();
    assert!(host.completion().is_complete());
}
