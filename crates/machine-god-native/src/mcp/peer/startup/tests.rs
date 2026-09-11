use super::*;

#[test]
fn configured_deadlines_are_checked_bounded_and_independent_of_legacy_defaults() {
    let now = Instant::now();
    let maximum = Duration::from_millis(u64::from(u32::MAX));
    let mut startup = Startup {
        deadline: now + maximum,
        timeout: maximum,
        observer: Some(Arc::new(|_| false)),
    };
    startup.validate(now).unwrap();
    assert_eq!(startup.attempt_deadline(now).unwrap(), startup.deadline);
    startup.timeout += Duration::from_millis(1);
    assert!(startup.validate(now).is_err());
    startup.timeout = Duration::ZERO;
    assert!(startup.validate(now).is_err());
    startup.timeout = Duration::from_secs(1);
    assert_eq!(
        startup.attempt_deadline(now).unwrap(),
        now + startup.timeout
    );
    startup.deadline = now + Duration::from_millis(500);
    assert_eq!(startup.attempt_deadline(now).unwrap(), startup.deadline);
    startup.observer = None;
    startup.deadline = now + Duration::from_secs(301);
    assert!(startup.validate(now).is_err());
    startup.deadline = now + Duration::from_secs(300);
    startup.validate(now).unwrap();
    assert_eq!(startup.attempt_deadline(now).unwrap(), startup.deadline);
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
