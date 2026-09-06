//! Worker-owned catalog leases and nonresident recovery for the terminal host.
//!
//! A saved session does not consume live-registry capacity or create a backend.
//! All filesystem work occurs in explicit calls on the terminal owner worker.

use crate::terminal_catalog::{TerminalCatalog, canonical_workspace, owner_name};
use crate::terminal_catalog_view::TerminalCatalogViewError;
use crate::terminal_history::TerminalHistory;
use crate::terminal_journal::TerminalJournal;
use crate::terminal_profile::{
    TerminalJournalPersistence, TerminalProfileBudget, TerminalProfileMutationContext,
};
use crate::terminal_profile_store::{MAX_PROFILE_OWNERS, TerminalProfileStore};
use crate::terminal_registry::TerminalRegistryError;
use crate::terminal_resident_dispatch::{
    TerminalResidentAuthority, read_terminal_action_page, resident_read_result,
};
use crate::terminal_session::{TerminalRecoveredSession, TerminalSessionError};
use machine_god_core::{
    BackgroundOutputOwner, CancellationToken, TerminalActionRequest, TerminalActionResult,
    TerminalSessionId,
};

type Result<T> = std::result::Result<T, TerminalCatalogViewError>;

/// Stored inside typed runtime state, after the registry in teardown order.
/// Catalog locks remain held between requests; no asynchronous future owns them.
pub(crate) struct TerminalHostCatalogs {
    workspace: String,
    catalogs: Vec<(BackgroundOutputOwner, TerminalCatalog)>,
}

impl TerminalHostCatalogs {
    /// The enclosing host tries resident dispatch first. Only historical
    /// observation actions may use this path; saved facts never grant mutation.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit profile and authorized request context"
    )]
    pub(crate) fn dispatch_history(
        &mut self,
        store: &TerminalProfileStore,
        budget: TerminalProfileBudget,
        authority: &TerminalResidentAuthority,
        request: &TerminalActionRequest,
        now_ms: i64,
        cancellation: &CancellationToken,
    ) -> Result<TerminalActionResult> {
        request
            .validate()
            .map_err(|_| TerminalCatalogViewError::Invalid)?;
        let (TerminalActionRequest::Read { session_id: id, .. }
        | TerminalActionRequest::Screen { session_id: id }
        | TerminalActionRequest::Inspect { session_id: id, .. }) = request
        else {
            return Err(TerminalCatalogViewError::Invalid);
        };
        let result = self.with_recovered(
            store,
            budget,
            &authority.owner,
            id,
            now_ms,
            cancellation,
            |session, persistence| {
                if let TerminalActionRequest::Inspect { events, .. } = request {
                    return session.inspect_result_with(
                        persistence,
                        &authority.owner,
                        authority.actor,
                        events,
                        &authority.controls,
                    );
                }
                session.prepare_public_facts_with(persistence, &authority.owner)?;
                let facts =
                    session.public_facts(&authority.owner, authority.actor, &authority.controls)?;
                match request {
                    TerminalActionRequest::Read { cursor, .. } => {
                        let page = read_terminal_action_page(cursor, |position, maximum| {
                            session
                                .read(&authority.owner, position, maximum)
                                .map_err(TerminalRegistryError::Session)
                        })
                        .map_err(registry_session_error)?;
                        resident_read_result(facts, page).map_err(registry_session_error)
                    }
                    TerminalActionRequest::Screen { .. } => Ok(TerminalActionResult::Screen {
                        session: facts,
                        snapshot: session.screen(&authority.owner)?,
                    }),
                    _ => unreachable!("historical action checked before effects"),
                }
            },
        )?;
        result
            .validate_for(request)
            .map_err(|_| TerminalCatalogViewError::Invalid)?;
        Ok(result)
    }

    /// Pure validation and empty allocation; does not prepare a profile or owner.
    pub(crate) fn new(workspace: String) -> Result<Self> {
        if !canonical_workspace(&workspace) {
            return Err(TerminalCatalogViewError::Invalid);
        }
        Ok(Self {
            workspace,
            catalogs: Vec::new(),
        })
    }

    /// Retains at most the profile's bounded owner count. Never evicts a catalog
    /// lease while live sessions may still depend on its exclusive ownership.
    pub(crate) fn catalog(
        &mut self,
        store: &TerminalProfileStore,
        owner: &BackgroundOutputOwner,
        cancellation: &CancellationToken,
    ) -> Result<&mut TerminalCatalog> {
        if cancellation.is_cancelled() {
            return Err(TerminalCatalogViewError::Cancelled);
        }
        let mut transaction = store
            .transaction()
            .map_err(TerminalCatalogViewError::Profile)?;
        if let Some(index) = self
            .catalogs
            .iter()
            .position(|(candidate, _)| candidate == owner)
        {
            transaction
                .validate_catalog(&self.catalogs[index].1)
                .map_err(TerminalCatalogViewError::Profile)?;
            return Ok(&mut self.catalogs[index].1);
        }
        if self.catalogs.len() == MAX_PROFILE_OWNERS {
            return Err(TerminalCatalogViewError::ResourceLimit);
        }
        let catalog = transaction
            .prepare_catalog(self.workspace.clone(), owner.clone())
            .map_err(TerminalCatalogViewError::Profile)?;
        self.catalogs.push((owner.clone(), catalog));
        Ok(&mut self.catalogs.last_mut().expect("catalog inserted").1)
    }

    /// Opens one cold history without promoting it to resident/native authority.
    /// The host checks the resident registry first. Its writer lock also prevents
    /// accidentally recovering a live session through this path.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit owner, profile, time and cancellation authority"
    )]
    pub(crate) fn with_recovered<T>(
        &mut self,
        store: &TerminalProfileStore,
        budget: TerminalProfileBudget,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        now_ms: i64,
        cancellation: &CancellationToken,
        operation: impl FnOnce(
            &mut TerminalRecoveredSession,
            &mut dyn TerminalJournalPersistence,
        ) -> std::result::Result<T, TerminalSessionError>,
    ) -> Result<T> {
        if now_ms < 0 {
            return Err(TerminalCatalogViewError::Invalid);
        }
        if cancellation.is_cancelled() {
            return Err(TerminalCatalogViewError::Cancelled);
        }
        let namespace = owner_name(&self.workspace, owner);
        let catalog = self.catalog(store, owner, cancellation)?;
        let mut transaction = store
            .transaction()
            .map_err(TerminalCatalogViewError::Profile)?;
        transaction
            .validate_catalog(catalog)
            .map_err(TerminalCatalogViewError::Profile)?;
        let directory = catalog
            .open(id)
            .map_err(TerminalCatalogViewError::Catalog)?;
        let journal = TerminalJournal::open_for_retention(directory, id)
            .map_err(TerminalCatalogViewError::Journal)?;
        let history =
            TerminalHistory::recover(journal).map_err(TerminalCatalogViewError::History)?;
        if cancellation.is_cancelled() {
            return Err(TerminalCatalogViewError::Cancelled);
        }
        let mut persistence =
            TerminalProfileMutationContext::new(&mut transaction, budget, &namespace);
        let mut recovered = TerminalRecoveredSession::recover_profile_with(
            &mut persistence,
            history,
            owner,
            &namespace,
            now_ms,
        )
        .map_err(TerminalCatalogViewError::Session)?;
        let result =
            operation(&mut recovered, &mut persistence).map_err(TerminalCatalogViewError::Session);
        // No process/probe factory is available here. All committed recovery and
        // inspect receipts finish before releasing authority and waking a caller.
        transaction
            .validate_catalog(catalog)
            .map_err(TerminalCatalogViewError::Profile)?;
        result
    }
}

fn registry_session_error(error: TerminalRegistryError) -> TerminalSessionError {
    match error {
        TerminalRegistryError::Session(error) => error,
        _ => TerminalSessionError::InvalidState,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_input::TerminalWriterId;
    use crate::terminal_journal::TerminalJournalLimits;
    use crate::terminal_monitor::{TerminalMonitorContext, TerminalMonitorSet};
    use crate::terminal_profile::TerminalProfileLimits;
    use crate::terminal_profile_store::TerminalProfileStoreError;
    use crate::terminal_session_record::TerminalSessionFacts;
    use machine_god_core::{
        SessionId, SessionIncarnationId, TerminalActorRole, TerminalAllowedControls,
        TerminalCursor, TerminalDimensions, TerminalEventQuery, TerminalLifecycle,
    };
    use rustix::fs::{Mode, OFlags};
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;

    struct Fixture {
        path: PathBuf,
        store: TerminalProfileStore,
        catalogs: TerminalHostCatalogs,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-host-catalog-{:032x}",
                u128::from_le_bytes(random)
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            let root = rustix::fs::open(
                &path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .unwrap();
            Self {
                path,
                store: TerminalProfileStore::prepare(root).unwrap(),
                catalogs: TerminalHostCatalogs::new("/workspace".into()).unwrap(),
            }
        }
        fn history(&mut self, name: &str, saved_owner: &BackgroundOutputOwner, workspace: &str) {
            self.history_bytes(name, saved_owner, workspace, b"saved\x1b[31m output\r\n");
        }
        fn history_bytes(
            &mut self,
            name: &str,
            saved_owner: &BackgroundOutputOwner,
            workspace: &str,
            output: &[u8],
        ) {
            let catalog = self
                .catalogs
                .catalog(&self.store, &owner("a"), &CancellationToken::new())
                .unwrap();
            let id = id(name);
            let directory = self
                .store
                .transaction()
                .unwrap()
                .create_session(catalog, &id)
                .unwrap();
            let journal =
                TerminalJournal::create(directory, id.clone(), TerminalJournalLimits::default())
                    .unwrap();
            let mut history =
                TerminalHistory::create(journal, &TerminalDimensions::new(3, 20).unwrap()).unwrap();
            for chunk in output.chunks(16 * 1024) {
                history.append(chunk).unwrap();
            }
            let context = TerminalMonitorContext {
                now_ms: 1,
                cursor: history.latest(),
                lifecycle: TerminalLifecycle::Running,
            };
            let monitors = TerminalMonitorSet::new(id.clone(), context.clone()).unwrap();
            let mut facts =
                TerminalSessionFacts::new(id, saved_owner, context, 0, 1, None).unwrap();
            let mut metadata = crate::terminal_session_record::test_metadata();
            metadata.workspace = workspace.into();
            facts.metadata = Some(metadata);
            history
                .publish_state(&facts.encode(&monitors).unwrap())
                .unwrap();
        }
        fn saved_state(&mut self, name: &str) -> Vec<u8> {
            let directory = self
                .catalogs
                .catalog(&self.store, &owner("a"), &CancellationToken::new())
                .unwrap()
                .open(&id(name))
                .unwrap();
            let journal = TerminalJournal::open_for_retention(directory, &id(name)).unwrap();
            TerminalHistory::recover(journal)
                .unwrap()
                .load_state()
                .unwrap()
                .unwrap()
                .bytes
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn owner(incarnation: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new(incarnation).unwrap(),
        )
    }
    fn id(name: &str) -> TerminalSessionId {
        TerminalSessionId::new(name).unwrap()
    }
    fn budget() -> TerminalProfileBudget {
        TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap()
    }

    #[test]
    fn historical_actions_share_full_read_pages_and_never_admit_native_mutations() {
        let mut fixture = Fixture::new();
        let maximum = machine_god_core::MAX_TERMINAL_ACTION_OUTPUT_BYTES;
        let output = vec![b'x'; maximum + 17];
        fixture.history_bytes("large", &owner("a"), "/workspace", &output);
        let authority = TerminalResidentAuthority {
            owner: owner("a"),
            actor: TerminalActorRole::Agent,
            writer: TerminalWriterId::new(std::num::NonZeroU64::new(1).unwrap()),
            controls: TerminalAllowedControls::default(),
            revoke_authorized: false,
        };
        let mut cursor = TerminalCursor::new(1, 0).unwrap();
        for expected in [&output[..maximum], &output[maximum..]] {
            let request = TerminalActionRequest::Read {
                session_id: id("large"),
                cursor,
            };
            let result = fixture
                .catalogs
                .dispatch_history(
                    &fixture.store,
                    budget(),
                    &authority,
                    &request,
                    2,
                    &CancellationToken::new(),
                )
                .unwrap();
            result.validate_for(&request).unwrap();
            let TerminalActionResult::Read {
                output,
                raw_range,
                session,
            } = result
            else {
                panic!("read result")
            };
            assert_eq!(output, expected);
            assert_eq!(session.lifecycle, TerminalLifecycle::Lost);
            cursor = raw_range.unwrap().end;
        }
        for request in [
            TerminalActionRequest::Screen {
                session_id: id("large"),
            },
            TerminalActionRequest::Inspect {
                session_id: id("large"),
                events: TerminalEventQuery {
                    after_event_id: 0,
                    acknowledge_event_id: None,
                    max_events: 256,
                },
            },
        ] {
            fixture
                .catalogs
                .dispatch_history(
                    &fixture.store,
                    budget(),
                    &authority,
                    &request,
                    2,
                    &CancellationToken::new(),
                )
                .unwrap()
                .validate_for(&request)
                .unwrap();
        }
        let before = fixture.saved_state("large");
        assert!(
            fixture
                .catalogs
                .dispatch_history(
                    &fixture.store,
                    budget(),
                    &authority,
                    &TerminalActionRequest::Signal {
                        session_id: id("large"),
                        signal: machine_god_core::TerminalSignal::Kill
                    },
                    2,
                    &CancellationToken::new()
                )
                .is_err()
        );
        assert_eq!(fixture.saved_state("large"), before);
    }

    #[test]
    fn constructor_and_precancelled_catalog_do_not_create_owner_namespaces() {
        assert!(TerminalHostCatalogs::new("relative".into()).is_err());
        let mut fixture = Fixture::new();
        let token = CancellationToken::new();
        token.cancel();
        assert!(
            fixture
                .catalogs
                .catalog(&fixture.store, &owner("a"), &token)
                .is_err()
        );
        assert_eq!(
            fixture
                .store
                .transaction()
                .unwrap()
                .inventory()
                .unwrap()
                .owner_count,
            0
        );
        assert!(fixture.catalogs.catalogs.is_empty());
    }

    #[test]
    fn owner_leases_are_reused_bound_to_profile_and_released_with_state() {
        let mut fixture = Fixture::new();
        let token = CancellationToken::new();
        let first = fixture
            .catalogs
            .catalog(&fixture.store, &owner("a"), &token)
            .unwrap()
            .namespace_key()
            .to_owned();
        assert_eq!(
            fixture
                .catalogs
                .catalog(&fixture.store, &owner("a"), &token)
                .unwrap()
                .namespace_key(),
            first
        );
        assert_eq!(fixture.catalogs.catalogs.len(), 1);
        let mut competing = TerminalHostCatalogs::new("/workspace".into()).unwrap();
        assert!(matches!(
            competing.catalog(&fixture.store, &owner("a"), &token),
            Err(TerminalCatalogViewError::Profile(
                TerminalProfileStoreError::Busy
            ))
        ));
        assert!(competing.catalogs.is_empty());
        let other = Fixture::new();
        assert!(
            fixture
                .catalogs
                .catalog(&other.store, &owner("a"), &token)
                .is_err()
        );
        let previous = std::mem::replace(
            &mut fixture.catalogs,
            TerminalHostCatalogs::new("/workspace".into()).unwrap(),
        );
        drop(previous);
        assert!(
            competing
                .catalog(&fixture.store, &owner("a"), &token)
                .is_ok()
        );
    }

    #[test]
    fn cold_history_recovery_is_durable_without_live_residency_or_backend() {
        let mut fixture = Fixture::new();
        for index in 0..20 {
            fixture.history(&format!("saved-{index}"), &owner("a"), "/workspace");
        }
        for index in 0..20 {
            let token = CancellationToken::new();
            fixture
                .catalogs
                .with_recovered(
                    &fixture.store,
                    budget(),
                    &owner("a"),
                    &id(&format!("saved-{index}")),
                    2,
                    &token,
                    |session, persistence| {
                        assert_eq!(
                            session.facts(&owner("a"))?.context.lifecycle,
                            TerminalLifecycle::Lost
                        );
                        let page =
                            session.read(&owner("a"), &TerminalCursor::new(1, 0).unwrap(), 1024)?;
                        assert_eq!(page.bytes, b"saved\x1b[31m output\r\n");
                        session.screen(&owner("a"))?.validate().unwrap();
                        let result = session.inspect_result_with(
                            persistence,
                            &owner("a"),
                            TerminalActorRole::Agent,
                            &TerminalEventQuery {
                                after_event_id: 0,
                                acknowledge_event_id: None,
                                max_events: 256,
                            },
                            &TerminalAllowedControls::default(),
                        )?;
                        result.validate().unwrap();
                        // Cancellation after recovery publication cannot discard the receipt.
                        token.cancel();
                        Ok(())
                    },
                )
                .unwrap();
            // The profile transaction ended before the callback result escaped.
            assert!(fixture.store.transaction().is_ok());
            fixture
                .catalogs
                .with_recovered(
                    &fixture.store,
                    budget(),
                    &owner("a"),
                    &id(&format!("saved-{index}")),
                    3,
                    &CancellationToken::new(),
                    |session, _| {
                        assert_eq!(
                            session.facts(&owner("a"))?.context.lifecycle,
                            TerminalLifecycle::Lost
                        );
                        Ok(())
                    },
                )
                .unwrap();
        }
        assert_eq!(fixture.catalogs.catalogs.len(), 1);
    }

    #[test]
    fn foreign_owner_workspace_clock_and_busy_writer_never_reach_callback() {
        let mut fixture = Fixture::new();
        fixture.history("wrong-owner", &owner("b"), "/workspace");
        fixture.history("wrong-workspace", &owner("a"), "/different");
        fixture.history("busy", &owner("a"), "/workspace");
        let owner_state = fixture.saved_state("wrong-owner");
        let workspace_state = fixture.saved_state("wrong-workspace");
        let token = CancellationToken::new();
        for name in ["wrong-owner", "wrong-workspace", "missing"] {
            assert!(
                fixture
                    .catalogs
                    .with_recovered(
                        &fixture.store,
                        budget(),
                        &owner("a"),
                        &id(name),
                        2,
                        &token,
                        |_, _| -> std::result::Result<(), TerminalSessionError> {
                            panic!("invalid recovery dispatched")
                        }
                    )
                    .is_err()
            );
        }
        assert_eq!(fixture.saved_state("wrong-owner"), owner_state);
        assert_eq!(fixture.saved_state("wrong-workspace"), workspace_state);
        assert!(
            fixture
                .catalogs
                .with_recovered(
                    &fixture.store,
                    budget(),
                    &owner("a"),
                    &id("busy"),
                    0,
                    &token,
                    |_, _| -> std::result::Result<(), TerminalSessionError> {
                        panic!("clock regression dispatched")
                    }
                )
                .is_err()
        );
        let directory = fixture
            .catalogs
            .catalog(&fixture.store, &owner("a"), &token)
            .unwrap()
            .open(&id("busy"))
            .unwrap();
        let _writer = TerminalJournal::open_for_retention(directory, &id("busy")).unwrap();
        assert!(
            fixture
                .catalogs
                .with_recovered(
                    &fixture.store,
                    budget(),
                    &owner("a"),
                    &id("busy"),
                    2,
                    &token,
                    |_, _| -> std::result::Result<(), TerminalSessionError> {
                        panic!("busy writer dispatched")
                    }
                )
                .is_err()
        );
    }
}
