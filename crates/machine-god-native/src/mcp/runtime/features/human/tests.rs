//! Pure guard cutoffs: no browser or process fixture is constructed here.

use super::*;

struct Clock(Instant);
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.0
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

#[test]
fn browser_guard_retains_every_original_authority_cutoff_and_exact_round() {
    for cutoff in 0..6 {
        let command = CancellationToken::new();
        let operation = CancellationToken::new();
        let server = CancellationToken::new();
        let credential = CancellationToken::new();
        let retired = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(true));
        let now = Instant::now();
        let guard = FeatureUrlGuard {
            authority: McpFeatureControlAuthority::for_human(
                command.clone(),
                operation.clone(),
                server.clone(),
                retired.clone(),
                Arc::from([credential.clone()]),
            )
            .unwrap(),
            clock: Arc::new(Clock(now)),
            deadline: now + Duration::from_secs(1),
            active: active.clone(),
        };
        guard.check().unwrap();
        match cutoff {
            0 => {
                command.cancel();
            }
            1 => {
                operation.cancel();
            }
            2 => {
                server.cancel();
            }
            3 => {
                credential.cancel();
            }
            4 => retired.store(true, Ordering::Release),
            5 => active.store(false, Ordering::Release),
            _ => unreachable!(),
        }
        assert!(matches!(
            guard.check(),
            Err(NativeBackgroundOpenError::Cancelled)
        ));
    }
}

#[test]
fn browser_guard_rejects_at_the_original_human_deadline() {
    let deadline = Instant::now();
    let guard = FeatureUrlGuard {
        authority: McpFeatureControlAuthority::for_human(
            CancellationToken::new(),
            CancellationToken::new(),
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
            Arc::from([]),
        )
        .unwrap(),
        clock: Arc::new(Clock(deadline)),
        deadline,
        active: Arc::new(AtomicBool::new(true)),
    };
    assert!(matches!(
        guard.check(),
        Err(NativeBackgroundOpenError::TimedOut)
    ));
}
