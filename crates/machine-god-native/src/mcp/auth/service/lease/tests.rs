use super::*;
use crate::mcp::{
    auth::codec::{Credentials, Registration, Secret},
    http::McpHttpClock,
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

struct Clock {
    current: Mutex<(Instant, i64, Option<Waker>)>,
    reads: AtomicUsize,
    timers: AtomicUsize,
}

impl Clock {
    fn new(now: Instant, wall: i64) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new((now, wall, None)),
            reads: AtomicUsize::new(0),
            timers: AtomicUsize::new(0),
        })
    }
    fn set(&self, now: Instant, wall: i64) {
        let wake = {
            let mut state = self.current.lock().unwrap();
            state.0 = now;
            state.1 = wall;
            state.2.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}

impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.current.lock().unwrap().0
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        self.timers.fetch_add(1, Ordering::Relaxed);
        Box::pin(std::future::poll_fn(move |cx| {
            let mut state = self.current.lock().unwrap();
            if state.0 >= deadline {
                Poll::Ready(())
            } else {
                state.2 = Some(cx.waker().clone());
                Poll::Pending
            }
        }))
    }
}

impl McpAuthClock for Clock {
    fn unix_millis(&self) -> i64 {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.current.lock().unwrap().1
    }
}

fn lease(clock: &Arc<Clock>, expires_ms: i64) -> McpAuthLease {
    let credentials = Credentials {
        identity: McpAuthIdentity {
            endpoint: "https://example.test/mcp".into(),
            selection: "a".repeat(64).into(),
        },
        resource: "https://example.test/mcp".into(),
        issuer: "https://example.test".into(),
        registration: Registration {
            id: Secret::new(b"client").unwrap(),
            secret: None,
            method: "none".into(),
        },
        access: Secret::new(b"private-access").unwrap(),
        refresh: None,
        scope: "read".into(),
        expires_ms,
        authorization_endpoint: "https://example.test/authorize".into(),
        token_endpoint: "https://example.test/token".into(),
        revocation_endpoint: None,
    };
    credentials.validate().unwrap();
    McpAuthLease {
        credentials: Arc::new(credentials),
        generation: CancellationToken::new(),
        profile: None,
        lifetime: Lifetime::new(clock.clone(), expires_ms).unwrap(),
    }
}

fn poll(future: &mut BoxFuture<'static, ()>) -> Poll<()> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn exact_skew_and_expiry_boundaries_survive_wall_clock_rollback() {
    let now = Instant::now();
    let clock = Clock::new(now, 1_000_000);
    let lease = lease(&clock, 1_120_000);
    assert_eq!(lease.expires_at(), Some(now + Duration::from_secs(120)));
    clock.set(now + Duration::from_millis(59_999), 0);
    assert_eq!(lease.refresh_due(), Ok(false));
    clock.set(now + Duration::from_secs(60), 0);
    assert_eq!(lease.refresh_due(), Ok(true));
    assert!(lease.access_token().is_ok());
    clock.set(now + Duration::from_millis(119_999), i64::MIN);
    assert!(lease.access_token().is_ok());
    clock.set(now + Duration::from_secs(120), i64::MIN);
    assert_eq!(lease.access_token(), Err(McpAuthError::Rejected));
    assert_eq!(lease.refresh_due(), Ok(true));
    assert!(!lease.generation().is_cancelled());
}

#[test]
fn wall_clock_forward_observations_narrow_skew_and_expiry() {
    let now = Instant::now();
    let clock = Clock::new(now, 1_000_000);
    let lease = lease(&clock, 1_120_000);
    clock.set(now, 1_059_999);
    assert_eq!(lease.refresh_due(), Ok(false));
    clock.set(now, 1_060_000);
    assert_eq!(lease.refresh_due(), Ok(true));
    assert!(lease.access_token().is_ok());
    clock.set(now, 1_120_000);
    assert_eq!(lease.access_token(), Err(McpAuthError::Rejected));
    assert!(poll(&mut lease.cancelled_owned()).is_ready());
    assert_eq!(clock.timers.load(Ordering::Relaxed), 0);
}

#[test]
fn no_expiry_does_not_read_or_schedule_a_clock() {
    let now = Instant::now();
    let clock = Clock::new(now, i64::MAX);
    let lease = lease(&clock, i64::MAX);
    assert_eq!(lease.expires_at(), None);
    assert_eq!(lease.refresh_due(), Ok(false));
    assert!(lease.access_token().is_ok());
    let mut stopped = lease.cancelled_owned();
    assert!(poll(&mut stopped).is_pending());
    assert_eq!(clock.reads.load(Ordering::Relaxed), 0);
    assert_eq!(clock.timers.load(Ordering::Relaxed), 0);
    lease.generation().cancel();
    assert!(poll(&mut stopped).is_ready());
}

#[test]
fn observer_is_inert_until_poll_and_retains_exact_expiry_after_lease_drop() {
    let now = Instant::now();
    let clock = Clock::new(now, 1_000_000);
    let lease = lease(&clock, 1_120_000);
    let reads = clock.reads.load(Ordering::Relaxed);
    drop(lease.cancelled_owned());
    let mut stopped = lease.cancelled_owned();
    assert_eq!(clock.reads.load(Ordering::Relaxed), reads);
    assert_eq!(clock.timers.load(Ordering::Relaxed), 0);
    let original = lease.generation();
    drop(lease);
    assert!(poll(&mut stopped).is_pending());
    assert_eq!(clock.timers.load(Ordering::Relaxed), 1);
    clock.set(now + Duration::from_secs(120), 0);
    assert!(poll(&mut stopped).is_ready());
    assert!(!original.is_cancelled());
}

#[test]
fn original_generation_cutoff_precedes_refresh_and_expiry_observations() {
    let now = Instant::now();
    let clock = Clock::new(now, 0);
    let original = lease(&clock, 120_000);
    let replacement = lease(&clock, 120_000);
    let mut stopped = original.cancelled_owned();
    assert!(poll(&mut stopped).is_pending());
    original.generation().cancel();
    clock.set(now + Duration::from_secs(120), 120_000);
    assert_eq!(original.access_token(), Err(McpAuthError::Conflict));
    assert_eq!(original.refresh_due(), Err(McpAuthError::Conflict));
    assert!(poll(&mut stopped).is_ready());
    assert!(!replacement.generation().is_cancelled());
    assert_eq!(replacement.refresh_due(), Ok(true));
}

#[test]
fn already_expired_mapping_has_no_negative_duration_or_underflow() {
    let now = Instant::now();
    let clock = Clock::new(now, i64::MAX);
    let lease = lease(&clock, i64::MIN);
    assert_eq!(lease.expires_at(), Some(now));
    assert_eq!(lease.refresh_due(), Ok(true));
    assert_eq!(lease.access_token(), Err(McpAuthError::Rejected));
}

#[test]
fn unrepresentable_finite_deadline_is_a_limit_not_unbounded_authority() {
    let base = Instant::now();
    let (mut low, mut high) = (0, u64::MAX);
    while low < high {
        let mid = low + (high - low) / 2 + 1;
        if base.checked_add(Duration::from_secs(mid)).is_some() {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let last_second = base.checked_add(Duration::from_secs(low)).unwrap();
    let clock = Clock::new(last_second, 0);
    assert!(matches!(
        Lifetime::new(clock, 120_000),
        Err(McpAuthError::Limit)
    ));
}
