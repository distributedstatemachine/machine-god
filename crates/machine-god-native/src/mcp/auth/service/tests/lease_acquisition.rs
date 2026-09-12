use super::*;
use std::sync::atomic::AtomicI64;

struct SelectedClock {
    base: Instant,
    elapsed_ms: AtomicU64,
    wall_ms: AtomicI64,
}
impl SelectedClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            base: Instant::now(),
            elapsed_ms: AtomicU64::new(0),
            wall_ms: AtomicI64::new(1_000_000),
        })
    }
    fn set(&self, elapsed_ms: u64, wall_ms: i64) {
        self.elapsed_ms.store(elapsed_ms, Ordering::Relaxed);
        self.wall_ms.store(wall_ms, Ordering::Relaxed);
    }
    fn deadline(&self) -> Instant {
        self.base + Duration::from_secs(600)
    }
}
impl McpHttpClock for SelectedClock {
    fn now(&self) -> Instant {
        self.base + Duration::from_millis(self.elapsed_ms.load(Ordering::Relaxed))
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        // Selected time changes only between completed operations in this fixture.
        Box::pin(std::future::pending())
    }
}
impl McpAuthClock for SelectedClock {
    fn unix_millis(&self) -> i64 {
        self.wall_ms.load(Ordering::Relaxed)
    }
}

fn seed(fixture: &Fixture, expires_ms: i64, access: &[u8]) {
    let mut credentials = fixture.credentials(access);
    credentials.expires_ms = expires_ms;
    let store = &fixture.service.inner.store;
    store
        .publish(
            &store.load().unwrap(),
            fixture.config.identity(),
            Some(&credentials),
        )
        .unwrap();
}

#[test]
fn repeated_service_acquisition_cannot_remap_expiry_after_wall_rollback() {
    let clock = SelectedClock::new();
    let fixture = Fixture::with_clock(clock.clone());
    seed(&fixture, 1_120_000, b"old");
    run(async {
        let caller = CancellationToken::new();
        let first = fixture
            .service
            .access_token(fixture.config.identity(), &caller, clock.deadline())
            .await
            .unwrap();
        let original_expiry = first.expires_at();
        assert_eq!(original_expiry, Some(clock.base + Duration::from_secs(120)));
        for elapsed in [10_000, 30_000, 59_999] {
            clock.set(elapsed, 0);
            let next = fixture
                .service
                .access_token(fixture.config.identity(), &caller, clock.deadline())
                .await
                .unwrap();
            assert_eq!(next.expires_at(), original_expiry);
            assert_eq!(next.refresh_due(), Ok(false));
            assert!(!first.generation().is_cancelled());
        }
        for elapsed in [60_000, 120_000, 180_000] {
            clock.set(elapsed, 0);
            assert_eq!(first.refresh_due(), Ok(true));
            // No refresh token: reacquisition must require auth instead of
            // returning the old bearer under a newly extended deadline.
            assert!(matches!(
                fixture
                    .service
                    .access_token(fixture.config.identity(), &caller, clock.deadline(),)
                    .await,
                Err(McpAuthError::Unavailable)
            ));
        }
        assert_eq!(first.access_token(), Err(McpAuthError::Rejected));
    });
}

#[test]
fn changed_stored_credentials_select_one_new_issuance_and_retire_the_old() {
    let clock = SelectedClock::new();
    let fixture = Fixture::with_clock(clock.clone());
    seed(&fixture, 1_120_000, b"old");
    run(async {
        let caller = CancellationToken::new();
        let old = fixture
            .service
            .access_token(fixture.config.identity(), &caller, clock.deadline())
            .await
            .unwrap();
        clock.set(10_000, 1_010_000);
        seed(&fixture, 1_240_000, b"new");
        let replacement = fixture
            .service
            .access_token(fixture.config.identity(), &caller, clock.deadline())
            .await
            .unwrap();
        assert_eq!(old.access_token(), Err(McpAuthError::Conflict));
        assert_eq!(replacement.access_token().unwrap(), b"new");
        assert_eq!(
            replacement.expires_at(),
            Some(clock.base + Duration::from_secs(240))
        );
        clock.set(20_000, 0);
        let reacquired = fixture
            .service
            .access_token(fixture.config.identity(), &caller, clock.deadline())
            .await
            .unwrap();
        assert_eq!(reacquired.expires_at(), replacement.expires_at());
        assert!(replacement.access_token().is_ok());
    });
}

#[test]
fn successful_publication_retains_its_issuance_for_later_acquisition() {
    let clock = SelectedClock::new();
    let fixture = Fixture::with_clock(clock.clone());
    run(async {
        let caller = CancellationToken::new();
        let operation = fixture
            .service
            .inner
            .begin(fixture.config.identity(), &caller, clock.deadline())
            .unwrap();
        let snapshot = operation.load(&caller, clock.deadline()).await.unwrap();
        let mut credentials = fixture.credentials(b"committed");
        credentials.expires_ms = 1_120_000;
        let committed = operation
            .commit(snapshot, credentials, &caller, clock.deadline())
            .await
            .unwrap();
        drop(operation);
        clock.set(30_000, 0);
        let reacquired = fixture
            .service
            .access_token(fixture.config.identity(), &caller, clock.deadline())
            .await
            .unwrap();
        assert_eq!(reacquired.expires_at(), committed.expires_at());
        assert_eq!(reacquired.access_token().unwrap(), b"committed");
        assert!(committed.access_token().is_ok());
    });
}
