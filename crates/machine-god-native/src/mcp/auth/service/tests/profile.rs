use super::*;
use crate::mcp::store::{
    McpConfigMutation, NativeMcpConfigCommit, NativeMcpConfigStore, NativeMcpConfigStoreError,
};
use futures_util::future::{Either, select};
use std::{io::Write, os::unix::fs::OpenOptionsExt};

struct Selected {
    store: Arc<NativeMcpConfigStore>,
    profile: Arc<NativeMcpAuthProfile>,
    cancellation: CancellationToken,
}
impl Selected {
    fn new(fixture: &Fixture) -> Self {
        let store = Arc::new(NativeMcpConfigStore::new(fixture.directory.join("profile")).unwrap());
        let server = McpServerConfig::decode(
            "srv",
            br#"{"type":"http","url":"http://127.0.0.1:34567/mcp"}"#,
        )
        .unwrap();
        futures_executor::block_on(
            store.apply(&store.load().unwrap(), &McpConfigMutation::Insert(server)),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let profile = Arc::new(NativeMcpAuthProfile::new(
            store.clone(),
            Arc::new(store.load().unwrap()),
            CancellationToken::new(),
            cancellation.clone(),
        ));
        Self {
            store,
            profile,
            cancellation,
        }
    }
    async fn commit(
        &self,
        fixture: &Fixture,
        cancellation: &CancellationToken,
    ) -> Result<McpAuthLease> {
        let mut operation =
            fixture
                .service
                .inner
                .begin(fixture.config.identity(), cancellation, deadline())?;
        operation.profile = Some(self.profile.clone());
        let snapshot = operation.load(cancellation, deadline()).await?;
        operation
            .commit(
                snapshot,
                fixture.credentials(b"replacement"),
                cancellation,
                deadline(),
            )
            .await
    }
    fn replace(&self) -> std::result::Result<NativeMcpConfigCommit, NativeMcpConfigStoreError> {
        futures_executor::block_on(self.store.apply(
            &self.store.load().unwrap(),
            &McpConfigMutation::Remove("srv".into()),
        ))
    }
}

#[test]
fn profile_selection_is_inert_and_cached_lease_observes_source_cutoff() {
    let fixture = Fixture::new();
    fixture.seed();
    let selected = Selected::new(&fixture);
    let cancellation = CancellationToken::new();
    drop(fixture.service.access_token_for_profile(
        fixture.config.identity(),
        selected.profile.clone(),
        &cancellation,
        deadline(),
    ));
    assert_eq!(fixture.service.cleanup_status().pending_operations, 0);
    run(async {
        let lease = fixture
            .service
            .access_token_for_profile(
                fixture.config.identity(),
                selected.profile.clone(),
                &cancellation,
                deadline(),
            )
            .await
            .unwrap();
        assert_eq!(lease.access_token().unwrap(), b"old");
        assert_eq!(lease.refresh_due(), Ok(false));
        let stopped = lease.cancelled_owned();
        selected.cancellation.cancel();
        stopped.await;
        assert_eq!(lease.access_token(), Err(McpAuthError::Conflict));
        assert_eq!(lease.refresh_due(), Err(McpAuthError::Conflict));
        assert!(matches!(
            fixture
                .service
                .access_token_for_profile(
                    fixture.config.identity(),
                    selected.profile.clone(),
                    &cancellation,
                    deadline(),
                )
                .await,
            Err(McpAuthError::Conflict)
        ));
    });
}

#[test]
fn changed_profile_before_credential_admission_preserves_old_credentials() {
    let fixture = Fixture::new();
    fixture.seed();
    let selected = Selected::new(&fixture);
    let pause = Pause::install(&fixture.service.inner.hooks.before_commit);
    run(async {
        let cancellation = CancellationToken::new();
        let mut commit = Box::pin(selected.commit(&fixture, &cancellation));
        assert!(matches!(
            select(commit.as_mut(), Box::pin(pause.entered())).await,
            Either::Right(_)
        ));
        selected.replace().unwrap();
        pause.release();
        assert!(matches!(commit.await, Err(McpAuthError::Conflict)));
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
fn profile_lock_excludes_cooperative_edits_through_actual_publication() {
    let fixture = Fixture::new();
    fixture.seed();
    let selected = Selected::new(&fixture);
    let pause = Pause::install(&fixture.service.inner.hooks.admitted_commit);
    run(async {
        let cancellation = CancellationToken::new();
        let old = fixture
            .service
            .access_token(fixture.config.identity(), &cancellation, deadline())
            .await
            .unwrap();
        let mut commit = Box::pin(selected.commit(&fixture, &cancellation));
        assert!(matches!(
            select(commit.as_mut(), Box::pin(pause.entered())).await,
            Either::Right(_)
        ));
        assert!(matches!(
            selected.replace(),
            Err(NativeMcpConfigStoreError::Busy)
        ));
        selected.cancellation.cancel();
        pause.release();
        let lease = commit.await.unwrap();
        assert_eq!(lease.access_token(), Err(McpAuthError::Conflict));
        assert_eq!(old.access_token(), Err(McpAuthError::Conflict));
        selected.replace().unwrap();
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
fn equal_byte_source_replacement_conflicts_without_credential_write() {
    let fixture = Fixture::new();
    fixture.seed();
    let selected = Selected::new(&fixture);
    let before = std::fs::read(selected.store.path()).unwrap();
    let replacement = fixture.directory.join("replacement");
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&replacement)
        .unwrap();
    file.write_all(&before).unwrap();
    std::fs::rename(replacement, selected.store.path()).unwrap();
    run(async {
        assert!(matches!(
            fixture
                .service
                .access_token_for_profile(
                    fixture.config.identity(),
                    selected.profile.clone(),
                    &CancellationToken::new(),
                    deadline(),
                )
                .await,
            Err(McpAuthError::Conflict)
        ));
    });
}

#[test]
fn noncooperative_edit_after_admission_keeps_ambiguous_publication_evidence() {
    let fixture = Fixture::new();
    fixture.seed();
    let selected = Selected::new(&fixture);
    let pause = Pause::install(&fixture.service.inner.hooks.admitted_commit);
    run(async {
        let cancellation = CancellationToken::new();
        let mut commit = Box::pin(selected.commit(&fixture, &cancellation));
        assert!(matches!(
            select(commit.as_mut(), Box::pin(pause.entered())).await,
            Either::Right(_)
        ));
        std::fs::write(selected.store.path(), b"{}").unwrap();
        pause.release();
        assert!(matches!(
            commit.await,
            Err(McpAuthError::AmbiguousPublication)
        ));
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
