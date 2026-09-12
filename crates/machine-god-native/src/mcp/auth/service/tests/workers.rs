use super::*;
use futures_util::future::{Either, join, select};
use std::{future::Future, pin::Pin};

async fn paused<T>(future: Pin<&mut impl Future<Output = T>>, pause: &Pause) {
    assert!(matches!(
        select(future, Box::pin(pause.entered())).await,
        Either::Right(_)
    ));
}

#[test]
fn cancelled_revocation_preserves_already_published_local_removal() {
    let fixture = Fixture::new();
    let store = &fixture.service.inner.store;
    let mut credentials = fixture.credentials(b"old");
    credentials.revocation_endpoint = Some("http://127.0.0.1:34567/revoke".into());
    store
        .publish(
            &store.load().unwrap(),
            fixture.config.identity(),
            Some(&credentials),
        )
        .unwrap();
    let pause = Pause::install(&fixture.service.inner.hooks.after_remove);
    run(async {
        let cancellation = CancellationToken::new();
        let mut logout = Box::pin(fixture.service.logout(
            fixture.config.identity(),
            &cancellation,
            deadline(),
        ));
        paused(logout.as_mut(), &pause).await;
        cancellation.cancel();
        pause.release();
        assert_eq!(
            logout.await.unwrap(),
            McpAuthLogoutReceipt {
                local: McpAuthLocalRemoval::Removed,
                remote: McpAuthRemoteRevocation::NotAttempted,
            }
        );
    });
    assert!(
        store
            .load()
            .unwrap()
            .get(fixture.config.identity())
            .is_none()
    );
}

#[test]
fn blocked_load_never_blocks_poll_and_dropped_owner_keeps_reservation() {
    let fixture = Fixture::new();
    fixture.seed();
    let pause = Pause::install(&fixture.service.inner.hooks.before_load);
    run(async {
        let cancellation = CancellationToken::new();
        let mut pending = Box::pin(fixture.service.access_token(
            fixture.config.identity(),
            &cancellation,
            deadline(),
        ));
        paused(pending.as_mut(), &pause).await;
        let other = config("http://127.0.0.1:34568/other");
        assert!(
            !fixture
                .service
                .status_owned(other.identity(), &CancellationToken::new(), deadline())
                .await
                .unwrap()
        );
        drop(pending);
        assert!(matches!(
            fixture
                .service
                .access_token(
                    fixture.config.identity(),
                    &CancellationToken::new(),
                    deadline()
                )
                .await,
            Err(McpAuthError::Busy)
        ));
        assert_eq!(fixture.service.cleanup_status().pending_workers, 1);
        fixture.service.close();
        assert!(matches!(
            fixture
                .service
                .settle(
                    Instant::now() + Duration::from_millis(5),
                    CancellationToken::new()
                )
                .await,
            Err(McpAuthError::Deadline)
        ));
        assert!(!fixture.service.cleanup_status().complete);
        pause.release();
        assert!(
            fixture
                .service
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
        assert!(
            fixture.workers.run(|| true).await.unwrap(),
            "auth settlement must not close shared host workers"
        );
    });
}

#[test]
fn cancellation_after_durability_keeps_publication_and_exact_lease() {
    let fixture = Fixture::new();
    fixture.seed();
    let pause = Pause::install(&fixture.service.inner.hooks.after_commit);
    run(async {
        let cancellation = CancellationToken::new();
        let mut commit = Box::pin(fixture.commit(&cancellation));
        paused(commit.as_mut(), &pause).await;
        cancellation.cancel();
        pause.release();
        let lease = commit.await.unwrap();
        assert_eq!(lease.access_token().unwrap(), b"replacement");
    });
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
}

#[test]
fn retirement_before_publication_admission_prevents_write() {
    let fixture = Fixture::new();
    fixture.seed();
    let pause = Pause::install(&fixture.service.inner.hooks.before_commit);
    run(async {
        let cancellation = CancellationToken::new();
        let mut commit = Box::pin(fixture.commit(&cancellation));
        paused(commit.as_mut(), &pause).await;
        let retirement_cancel = CancellationToken::new();
        let mut retirement = Box::pin(fixture.service.retire(
            fixture.config.identity(),
            &retirement_cancel,
            deadline(),
        ));
        assert!(futures_util::poll!(retirement.as_mut()).is_pending());
        assert!(matches!(
            fixture
                .service
                .access_token(
                    fixture.config.identity(),
                    &CancellationToken::new(),
                    deadline()
                )
                .await,
            Err(McpAuthError::Busy)
        ));
        pause.release();
        let (commit, retirement) = join(commit, retirement).await;
        assert!(matches!(commit, Err(McpAuthError::Conflict)));
        retirement.unwrap();
    });
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
        b"old"
    );
}

#[test]
fn admitted_publication_finishes_before_retirement_ack_without_live_lease() {
    let fixture = Fixture::new();
    fixture.seed();
    let pause = Pause::install(&fixture.service.inner.hooks.admitted_commit);
    run(async {
        let cancellation = CancellationToken::new();
        let old = fixture
            .service
            .access_token(fixture.config.identity(), &cancellation, deadline())
            .await
            .unwrap();
        let mut commit = Box::pin(fixture.commit(&cancellation));
        paused(commit.as_mut(), &pause).await;
        let retirement_cancel = CancellationToken::new();
        let mut retirement = Box::pin(fixture.service.retire(
            fixture.config.identity(),
            &retirement_cancel,
            deadline(),
        ));
        assert!(futures_util::poll!(retirement.as_mut()).is_pending());
        assert!(
            old.access_token().is_err(),
            "first retirement poll cuts off old live authority"
        );
        assert!(matches!(
            fixture
                .service
                .access_token(
                    fixture.config.identity(),
                    &CancellationToken::new(),
                    deadline()
                )
                .await,
            Err(McpAuthError::Busy)
        ));
        pause.release();
        let (commit, retirement) = join(commit, retirement).await;
        assert!(commit.unwrap().access_token().is_err());
        retirement.unwrap();
        assert_eq!(fixture.service.cleanup_status().pending_workers, 0);
    });
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
}

#[test]
fn logout_waits_admitted_publication_then_deletes_without_late_republication() {
    let fixture = Fixture::new();
    fixture.seed();
    let pause = Pause::install(&fixture.service.inner.hooks.admitted_commit);
    run(async {
        let cancellation = CancellationToken::new();
        let mut commit = Box::pin(fixture.commit(&cancellation));
        paused(commit.as_mut(), &pause).await;
        let logout_cancel = CancellationToken::new();
        let mut logout = Box::pin(fixture.service.logout(
            fixture.config.identity(),
            &logout_cancel,
            deadline(),
        ));
        assert!(futures_util::poll!(logout.as_mut()).is_pending());
        pause.release();
        let (commit, logout) = join(commit, logout).await;
        assert!(commit.unwrap().access_token().is_err());
        assert_eq!(
            logout.unwrap(),
            McpAuthLogoutReceipt {
                local: McpAuthLocalRemoval::Removed,
                remote: McpAuthRemoteRevocation::Ambiguous
            }
        );
        assert!(
            fixture
                .service
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
    assert!(
        fixture
            .service
            .inner
            .store
            .load()
            .unwrap()
            .get(fixture.config.identity())
            .is_none()
    );
}

#[test]
fn dropped_durable_publication_remains_owned_through_close_and_settle() {
    let fixture = Fixture::new();
    fixture.seed();
    let pause = Pause::install(&fixture.service.inner.hooks.after_commit);
    run(async {
        let cancellation = CancellationToken::new();
        let mut commit = Box::pin(fixture.commit(&cancellation));
        paused(commit.as_mut(), &pause).await;
        drop(commit);
        fixture.service.close();
        assert!(!fixture.service.cleanup_status().complete);
        assert!(matches!(
            fixture
                .service
                .settle(
                    Instant::now() + Duration::from_millis(5),
                    CancellationToken::new()
                )
                .await,
            Err(McpAuthError::Deadline)
        ));
        pause.release();
        assert!(
            fixture
                .service
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
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
}

#[test]
fn retired_reservations_and_unpolled_operations_are_bounded_and_inert() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    drop(
        fixture
            .service
            .access_token(fixture.config.identity(), &cancellation, deadline()),
    );
    drop(
        fixture
            .service
            .retire(fixture.config.identity(), &cancellation, deadline()),
    );
    assert_eq!(fixture.service.cleanup_status().pending_operations, 0);
    assert!(!fixture.service.inner.store.path().exists());
    let mut retained = Vec::new();
    for index in 0..64 {
        let selected = config(&format!("http://127.0.0.1:34567/mcp?identity={index}"));
        retained.push(
            fixture
                .service
                .inner
                .begin(selected.identity(), &cancellation, deadline())
                .unwrap(),
        );
        retained.push(
            fixture
                .service
                .inner
                .cutoff(selected.identity(), &cancellation, deadline())
                .unwrap()
                .0,
        );
    }
    assert_eq!(fixture.service.cleanup_status().pending_operations, 128);
    assert!(matches!(
        fixture
            .service
            .inner
            .begin(fixture.config.identity(), &cancellation, deadline()),
        Err(McpAuthError::Limit)
    ));
    drop(retained);
    run(async {
        assert!(
            fixture
                .service
                .settle(deadline(), CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
}
