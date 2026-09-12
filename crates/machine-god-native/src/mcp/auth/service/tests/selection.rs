use super::*;
use futures_util::future::{Either, select};

#[test]
fn simultaneous_same_identity_drops_cannot_miss_last_selection_cutoff() {
    let fixture = Fixture::new();
    for _ in 0..32 {
        let first = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        let second = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                drop(first);
            });
            scope.spawn(|| {
                barrier.wait();
                drop(second);
            });
        });
        let _ = fixture.service.cleanup_status();
        assert!(lock(&fixture.service.inner.state).entries.is_empty());
    }
}

#[test]
fn overlapping_same_identity_selections_retire_only_after_last_owner() {
    let fixture = Fixture::new();
    fixture.seed();
    run(async {
        let first = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        let second = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        let lease = fixture
            .service
            .access_token(
                fixture.config.identity(),
                &CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        drop(first);
        assert!(lease.access_token().is_ok());
        drop(second);
        assert!(lease.access_token().is_err());
        let _ = fixture.service.cleanup_status();
        assert!(lock(&fixture.service.inner.state).entries.is_empty());
        assert!(
            fixture
                .service
                .inner
                .store
                .load()
                .unwrap()
                .get(fixture.config.identity())
                .is_some()
        );
    });
}

#[test]
fn selection_retirement_matches_exact_service_and_identity() {
    let first = Fixture::new();
    let second = Fixture::new();
    first.seed();
    second.seed();
    let other = config("http://127.0.0.1:34568/other");
    run(async {
        let selected = first
            .service
            .retain_identity(first.config.identity())
            .unwrap();
        let unrelated = first.service.retain_identity(other.identity()).unwrap();
        let foreign = second
            .service
            .retain_identity(second.config.identity())
            .unwrap();
        let first_lease = first
            .service
            .access_token(
                first.config.identity(),
                &CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let second_lease = second
            .service
            .access_token(
                second.config.identity(),
                &CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        drop(unrelated);
        assert!(first_lease.access_token().is_ok());
        drop(selected);
        assert!(first_lease.access_token().is_err());
        assert!(second_lease.access_token().is_ok());
        drop(foreign);
        assert!(second_lease.access_token().is_err());
    });
}

#[test]
fn missing_reservation_is_released_without_writing_credentials() {
    let fixture = Fixture::new();
    run(async {
        let owner = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        assert!(matches!(
            fixture
                .service
                .access_token(
                    fixture.config.identity(),
                    &CancellationToken::new(),
                    deadline()
                )
                .await,
            Err(McpAuthError::Missing)
        ));
        assert_eq!(lock(&fixture.service.inner.state).entries.len(), 1);
        drop(owner);
        let _ = fixture.service.cleanup_status();
        assert!(lock(&fixture.service.inner.state).entries.is_empty());
        assert!(!fixture.service.inner.store.path().exists());
    });
}

#[test]
fn selected_identity_overlap_has_a_finite_reusable_cap() {
    let fixture = Fixture::new();
    let configs = (0..129)
        .map(|index| config(&format!("http://127.0.0.1:34567/{index}")))
        .collect::<Vec<_>>();
    let mut owners = configs[..128]
        .iter()
        .map(|config| fixture.service.retain_identity(config.identity()).unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        fixture.service.retain_identity(configs[128].identity()),
        Err(McpAuthError::Limit)
    ));
    drop(owners.pop());
    owners.push(
        fixture
            .service
            .retain_identity(configs[128].identity())
            .unwrap(),
    );
    assert_eq!(lock(&fixture.service.inner.state).entries.len(), 128);
    drop(owners);
    let _ = fixture.service.cleanup_status();
    assert!(lock(&fixture.service.inner.state).entries.is_empty());
    assert!(!fixture.service.inner.store.path().exists());
}

#[test]
fn same_identity_owner_count_is_bounded_separately_from_distinct_entries() {
    let fixture = Fixture::new();
    let mut owners = (0..128)
        .map(|_| {
            fixture
                .service
                .retain_identity(fixture.config.identity())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(lock(&fixture.service.inner.state).entries.len(), 1);
    assert!(matches!(
        fixture.service.retain_identity(fixture.config.identity()),
        Err(McpAuthError::Limit)
    ));
    drop(owners.pop());
    owners.push(
        fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap(),
    );
    drop(owners);
    let _ = fixture.service.cleanup_status();
    assert!(lock(&fixture.service.inner.state).entries.is_empty());
}

#[test]
fn final_selection_retirement_preserves_admitted_publication_receipt_and_charge() {
    let fixture = Fixture::new();
    fixture.seed();
    let pause = Pause::install(&fixture.service.inner.hooks.admitted_commit);
    run(async {
        let owner = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        let cancellation = CancellationToken::new();
        let mut commit = Box::pin(fixture.commit(&cancellation));
        assert!(matches!(
            select(commit.as_mut(), Box::pin(pause.entered())).await,
            Either::Right(_)
        ));
        drop(owner);
        let _ = fixture.service.cleanup_status();
        assert_eq!(lock(&fixture.service.inner.state).entries.len(), 1);
        assert!(matches!(
            fixture.service.retain_identity(fixture.config.identity()),
            Err(McpAuthError::Busy)
        ));
        pause.release();
        let receipt = commit.await.unwrap();
        assert!(receipt.access_token().is_err());
        let _ = fixture.service.cleanup_status();
        assert!(lock(&fixture.service.inner.state).entries.is_empty());
        assert_eq!(
            fixture
                .service
                .inner
                .store
                .load()
                .unwrap()
                .get(fixture.config.identity())
                .unwrap()
                .access
                .bytes(),
            b"replacement"
        );
        let replacement = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        assert!(
            fixture
                .service
                .access_token(fixture.config.identity(), &cancellation, deadline())
                .await
                .unwrap()
                .access_token()
                .is_ok()
        );
        drop(replacement);
    });
}

#[test]
fn explicit_retirement_does_not_lose_other_selection_owners_on_slot_renewal() {
    let fixture = Fixture::new();
    fixture.seed();
    run(async {
        let active = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        let command = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        fixture
            .service
            .retire(
                fixture.config.identity(),
                &CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        let lease = fixture
            .service
            .access_token(
                fixture.config.identity(),
                &CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        drop(command);
        assert!(lease.access_token().is_ok());
        drop(active);
        assert!(lease.access_token().is_err());
    });
}

#[test]
fn close_rejects_selection_and_invalidates_owned_lease() {
    let fixture = Fixture::new();
    fixture.seed();
    run(async {
        let owner = fixture
            .service
            .retain_identity(fixture.config.identity())
            .unwrap();
        let lease = fixture
            .service
            .access_token(
                fixture.config.identity(),
                &CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        fixture.service.close();
        assert!(lease.access_token().is_err());
        assert!(matches!(
            fixture.service.retain_identity(fixture.config.identity()),
            Err(McpAuthError::Unavailable)
        ));
        drop(owner);
        assert_eq!(fixture.service.cleanup_status().pending_workers, 0);
    });
}
