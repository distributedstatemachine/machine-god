//! Complete owner-scoped durable catalog projection on the blocking owner.
//!
//! Listing does not admit recovered history to the sixteen-slot live registry,
//! probe saved PIDs, or construct a backend. Only one disk history is decoded at
//! a time; the result retains compact public facts, not commands or screen cells.

use crate::terminal_catalog::{TerminalCatalog, TerminalCatalogError, owner_name};
use crate::terminal_history::{TerminalHistory, TerminalHistoryError};
use crate::terminal_journal::{TerminalJournal, TerminalJournalError};
use crate::terminal_profile::{TerminalProfileBudget, TerminalProfileMutationContext};
use crate::terminal_profile_store::{TerminalProfileStore, TerminalProfileStoreError};
use crate::terminal_registry::{TerminalRegistry, TerminalRegistryError};
use crate::terminal_session::{
    TerminalRecoveredSession, TerminalSessionBackend, TerminalSessionError,
};
use crate::terminal_session_record::TerminalSessionFacts;
use machine_god_core::{
    BackgroundOutputOwner, MAX_TERMINAL_ACTION_RESULTS, TerminalActionResult, TerminalActorRole,
    TerminalAllowedControls, TerminalListFilters, TerminalSessionFacts as PublicFacts,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalCatalogViewError {
    Cancelled,
    Invalid,
    ResourceLimit,
    Catalog(TerminalCatalogError),
    Profile(TerminalProfileStoreError),
    Journal(TerminalJournalError),
    History(TerminalHistoryError),
    Session(TerminalSessionError),
    Registry(TerminalRegistryError),
}
type Result<T> = std::result::Result<T, TerminalCatalogViewError>;

/// Exact injected owner authority precedes catalog/profile effects. Filters are
/// predicates only: they cannot select another owner or suppress corrupt rows.
/// The pinned list contract has no pagination or truncation flag. An overfull
/// union fails instead of silently returning a resident-only or partial result.
#[allow(
    clippy::too_many_arguments,
    reason = "explicit owner, profile and projection authority"
)]
#[cfg(test)]
pub(crate) fn list_with<B: TerminalSessionBackend>(
    registry: &mut TerminalRegistry<B>,
    store: &TerminalProfileStore,
    catalog: &TerminalCatalog,
    budget: TerminalProfileBudget,
    owner: &BackgroundOutputOwner,
    actor: TerminalActorRole,
    controls: &TerminalAllowedControls,
    filters: &TerminalListFilters,
    now_ms: i64,
) -> Result<TerminalActionResult> {
    list_selected_with(
        registry,
        store,
        catalog,
        budget,
        owner,
        actor,
        controls,
        filters,
        now_ms,
        owner,
        |_| true,
    )
}

/// Access filtering is supplied by the retained host, never provider arguments.
#[allow(
    clippy::too_many_arguments,
    reason = "separate immutable storage and current access principals"
)]
pub(crate) fn list_selected_with<B: TerminalSessionBackend>(
    registry: &mut TerminalRegistry<B>,
    store: &TerminalProfileStore,
    catalog: &TerminalCatalog,
    budget: TerminalProfileBudget,
    owner: &BackgroundOutputOwner,
    actor: TerminalActorRole,
    controls: &TerminalAllowedControls,
    filters: &TerminalListFilters,
    now_ms: i64,
    access_owner: &BackgroundOutputOwner,
    visible: impl Fn(&machine_god_core::TerminalSessionId) -> bool,
) -> Result<TerminalActionResult> {
    filters
        .validate()
        .map_err(|_| TerminalCatalogViewError::Invalid)?;
    let namespace = owner_name(registry.workspace(), owner);
    if catalog.namespace_key() != namespace || now_ms < registry.minimum_time_ms() {
        return Err(TerminalCatalogViewError::Invalid);
    }
    let mut transaction = store
        .transaction()
        .map_err(TerminalCatalogViewError::Profile)?;
    transaction
        .validate_catalog(catalog)
        .map_err(TerminalCatalogViewError::Profile)?;
    let residents = registry.owner_ids(owner);
    let snapshot = catalog
        .snapshot()
        .map_err(TerminalCatalogViewError::Catalog)?;
    let mut ids: Vec<_> = snapshot.ids().cloned().collect();
    ids.extend(residents.iter().cloned());
    ids.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
    ids.dedup();
    ids.retain(visible);
    if ids.len() > MAX_TERMINAL_ACTION_RESULTS {
        return Err(TerminalCatalogViewError::ResourceLimit);
    }
    let mut sessions = Vec::with_capacity(ids.len());
    for id in ids {
        let public = if residents.contains(&id) {
            // Validate the namespace again before any checkpoint re-anchoring.
            // Drop the metadata clone before building the compact projection.
            validate_facts(
                &registry
                    .inspect(owner, &id)
                    .map_err(TerminalCatalogViewError::Registry)?,
                owner,
                &namespace,
            )?;
            let mut persistence =
                TerminalProfileMutationContext::new(&mut transaction, budget, &namespace);
            registry
                .project_facts_with(&mut persistence, owner, &id, actor, controls)
                .map_err(TerminalCatalogViewError::Registry)?
        } else {
            let directory = snapshot
                .open(&id)
                .map_err(TerminalCatalogViewError::Catalog)?;
            // Read stored, validated limits; listing does not replace retention
            // policy with a caller-supplied guess or promote history to live.
            let journal = TerminalJournal::open_for_retention(directory, &id)
                .map_err(TerminalCatalogViewError::Journal)?;
            let history =
                TerminalHistory::recover(journal).map_err(TerminalCatalogViewError::History)?;
            {
                let state = history
                    .load_state()
                    .map_err(TerminalCatalogViewError::History)?
                    .ok_or(TerminalCatalogViewError::Invalid)?;
                let (facts, _) = TerminalSessionFacts::decode(&state.bytes, &id, &state.source)
                    .map_err(|_| TerminalCatalogViewError::Invalid)?;
                validate_facts(&facts, owner, &namespace)?;
            }
            let mut persistence =
                TerminalProfileMutationContext::new(&mut transaction, budget, &namespace);
            let mut recovered =
                TerminalRecoveredSession::recover_with(&mut persistence, history, owner, now_ms)
                    .map_err(TerminalCatalogViewError::Session)?;
            recovered
                .prepare_public_facts_with(&mut persistence, owner)
                .map_err(TerminalCatalogViewError::Session)?;
            recovered
                .public_facts(owner, actor, controls)
                .map_err(TerminalCatalogViewError::Session)?
        };
        if matches_filters(&public, registry.workspace(), access_owner, filters) {
            sessions.push(public);
        }
    }
    snapshot
        .validate()
        .map_err(TerminalCatalogViewError::Catalog)?;
    transaction
        .validate_catalog(catalog)
        .map_err(TerminalCatalogViewError::Profile)?;
    let result = TerminalActionResult::List { sessions };
    result
        .validate()
        .map_err(|_| TerminalCatalogViewError::Invalid)?;
    Ok(result)
}

fn validate_facts(
    facts: &TerminalSessionFacts,
    owner: &BackgroundOutputOwner,
    namespace: &str,
) -> Result<()> {
    if !facts.owned_by(owner) {
        return Err(TerminalCatalogViewError::Invalid);
    }
    facts
        .validate_profile_binding(namespace)
        .map_err(|_| TerminalCatalogViewError::Invalid)
}

fn matches_filters(
    facts: &PublicFacts,
    workspace: &str,
    owner: &BackgroundOutputOwner,
    filters: &TerminalListFilters,
) -> bool {
    filters
        .task_id
        .as_ref()
        .is_none_or(|task| task == owner.session_id().as_str())
        && filters
            .workspace_root
            .as_ref()
            .is_none_or(|root| root == workspace)
        && filters
            .lifecycle
            .is_none_or(|lifecycle| lifecycle == facts.lifecycle)
        && filters
            .backend
            .is_none_or(|backend| backend == facts.backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_input::BackgroundInputReceipt;
    use crate::terminal_journal::TerminalJournalLimits;
    use crate::terminal_monitor::{TerminalMonitorContext, TerminalMonitorSet};
    use crate::terminal_pty::{TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
    use crate::terminal_session::TerminalSession;
    use machine_god_core::{
        SessionId, SessionIncarnationId, TerminalBackend, TerminalCursor, TerminalDimensions,
        TerminalLifecycle, TerminalSessionId, TerminalSignal,
    };
    use rustix::fd::OwnedFd;
    use rustix::fs::{Mode, OFlags};
    use std::fs::DirBuilder;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Backend(Arc<AtomicUsize>);
    impl TerminalSessionBackend for Backend {
        fn read(&mut self, _: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(())
        }
        fn write(&mut self, _: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(())
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(TerminalPtyStatus::Running)
        }
        fn resize(&mut self, _: &TerminalDimensions) -> std::result::Result<(), ()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(())
        }
        fn signal(&mut self, _: TerminalSignal) -> std::result::Result<(), ()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(())
        }
        fn signal_may_discard_output(&self) -> bool {
            false
        }
        fn close(
            &mut self,
            _: bool,
            _: &mut dyn FnMut(&[u8]),
        ) -> std::result::Result<TerminalPtyClose, ()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(TerminalPtyClose {
                status: TerminalPtyStatus::Exited(0),
                output_incomplete: false,
            })
        }
    }

    struct Fixture {
        path: PathBuf,
        store: TerminalProfileStore,
        catalog: TerminalCatalog,
        registry: TerminalRegistry<Backend>,
        calls: Arc<AtomicUsize>,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-catalog-view-{:032x}",
                u128::from_le_bytes(random)
            ));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            let root = rustix::fs::open(
                &path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .unwrap();
            let store = TerminalProfileStore::prepare(root).unwrap();
            let catalog = store
                .transaction()
                .unwrap()
                .prepare_catalog("/workspace".into(), owner("incarnation"))
                .unwrap();
            Self {
                path,
                store,
                catalog,
                registry: TerminalRegistry::new("/workspace".into()).unwrap(),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
        fn directory(&mut self, id: &TerminalSessionId) -> OwnedFd {
            self.store
                .transaction()
                .unwrap()
                .create_session(&mut self.catalog, id)
                .unwrap()
        }
        fn disk(&mut self, name: &str, lifecycle: TerminalLifecycle, backend: TerminalBackend) {
            self.disk_with_owner(
                name,
                lifecycle,
                backend,
                &owner("incarnation"),
                Some("/workspace"),
            );
        }
        fn disk_with_owner(
            &mut self,
            name: &str,
            lifecycle: TerminalLifecycle,
            backend: TerminalBackend,
            record_owner: &BackgroundOutputOwner,
            workspace: Option<&str>,
        ) {
            let id = id(name);
            let mut journal = TerminalJournal::create(
                self.directory(&id),
                id.clone(),
                TerminalJournalLimits::default(),
            )
            .unwrap();
            let context = TerminalMonitorContext {
                now_ms: 0,
                cursor: TerminalCursor::new(1, 0).unwrap(),
                lifecycle,
            };
            let monitors = TerminalMonitorSet::new(id.clone(), context.clone()).unwrap();
            let mut facts =
                TerminalSessionFacts::new(id, record_owner, context, 0, 0, None).unwrap();
            facts.metadata = workspace.map(|workspace| {
                let mut metadata = crate::terminal_session_record::test_metadata();
                metadata.workspace = workspace.into();
                metadata.backend = backend;
                metadata.command = Some("x".repeat(64 * 1024));
                metadata
            });
            journal
                .publish_state(journal.latest(), &facts.encode(&monitors).unwrap())
                .unwrap();
        }
        fn live(&mut self, name: &str) {
            let id = id(name);
            let directory = self.directory(&id);
            self.live_in(id, directory);
        }
        fn live_in(&mut self, id: TerminalSessionId, directory: OwnedFd) {
            let journal =
                TerminalJournal::create(directory, id.clone(), TerminalJournalLimits::default())
                    .unwrap();
            let history =
                TerminalHistory::create(journal, &TerminalDimensions::new(3, 20).unwrap()).unwrap();
            let mut session = TerminalSession::new(
                Backend(Arc::clone(&self.calls)),
                history,
                owner("incarnation"),
                id.clone(),
                crate::terminal_session_record::test_metadata(),
                0,
            )
            .unwrap();
            session.shell_ready(0).unwrap();
            self.registry
                .start(owner("incarnation"), id, || Ok(session))
                .unwrap();
        }
        fn list(&mut self, filters: &TerminalListFilters) -> Result<Vec<PublicFacts>> {
            let controls = TerminalAllowedControls {
                read: true,
                screen: true,
                write: true,
                wait: true,
                monitor: true,
                inspect: true,
                list: true,
                resize: true,
                signal: true,
                close: true,
            };
            let TerminalActionResult::List { sessions } = list_with(
                &mut self.registry,
                &self.store,
                &self.catalog,
                budget(),
                &owner("incarnation"),
                TerminalActorRole::Agent,
                &controls,
                filters,
                1,
            )?
            else {
                panic!("list result")
            };
            Ok(sessions)
        }
        fn session_path(&self, name: &str) -> PathBuf {
            self.path
                .join("terminal-v1")
                .join(self.catalog.namespace_key())
                .join("sessions")
                .join(name)
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
        TerminalProfileBudget::new(crate::terminal_profile::TerminalProfileLimits::default())
            .unwrap()
    }

    #[test]
    fn disk_catalog_exceeds_residency_and_live_overlay_never_probes_backend() {
        let mut fixture = Fixture::new();
        for n in (0..20).rev() {
            fixture.disk(
                &format!("disk-{n:03}"),
                TerminalLifecycle::Closed,
                TerminalBackend::Native,
            );
        }
        fixture.live("live");
        let before = fixture.calls.load(Ordering::Relaxed);
        let facts = fixture.list(&TerminalListFilters::default()).unwrap();
        assert_eq!(facts.len(), 21);
        assert!(
            facts
                .windows(2)
                .all(|pair| pair[0].session_id.as_str() < pair[1].session_id.as_str())
        );
        assert_eq!(facts.last().unwrap().lifecycle, TerminalLifecycle::Running);
        assert!(facts.last().unwrap().next_actions.write);
        assert_eq!(fixture.registry.owner_ids(&owner("incarnation")).len(), 1);
        assert_eq!(fixture.calls.load(Ordering::Relaxed), before);
        assert!(
            fixture.store.transaction().is_ok(),
            "reply releases profile lock"
        );
        assert_eq!(
            fixture.list(&TerminalListFilters::default()).unwrap().len(),
            21
        );
    }

    #[test]
    fn exact_filters_cannot_expand_owner_scope() {
        let mut fixture = Fixture::new();
        fixture.disk("native", TerminalLifecycle::Closed, TerminalBackend::Native);
        fixture.disk("tmux", TerminalLifecycle::Lost, TerminalBackend::Tmux);
        let filters = TerminalListFilters {
            task_id: Some("owner".into()),
            workspace_root: Some("/workspace".into()),
            lifecycle: Some(TerminalLifecycle::Lost),
            backend: Some(TerminalBackend::Tmux),
        };
        assert_eq!(fixture.list(&filters).unwrap()[0].session_id, id("tmux"));
        assert!(
            fixture
                .list(&TerminalListFilters {
                    task_id: Some("other".into()),
                    ..filters.clone()
                })
                .unwrap()
                .is_empty()
        );
        assert!(
            fixture
                .list(&TerminalListFilters {
                    workspace_root: Some("/elsewhere".into()),
                    ..filters
                })
                .unwrap()
                .is_empty()
        );
        let transaction = fixture.store.transaction().unwrap();
        let error = list_with(
            &mut fixture.registry,
            &fixture.store,
            &fixture.catalog,
            budget(),
            &owner("other-incarnation"),
            TerminalActorRole::Agent,
            &TerminalAllowedControls::default(),
            &TerminalListFilters::default(),
            1,
        )
        .unwrap_err();
        assert_eq!(
            error,
            TerminalCatalogViewError::Invalid,
            "owner mismatch rejected before busy profile"
        );
        drop(transaction);
    }

    #[test]
    fn nonresident_formerly_live_is_lost_without_native_authority() {
        let mut fixture = Fixture::new();
        fixture.disk("old", TerminalLifecycle::Running, TerminalBackend::Native);
        let facts = fixture.list(&TerminalListFilters::default()).unwrap();
        assert_eq!(facts[0].lifecycle, TerminalLifecycle::Lost);
        assert!(
            !facts[0].next_actions.write
                && !facts[0].next_actions.signal
                && !facts[0].next_actions.resize
        );
        assert!(fixture.registry.owner_ids(&owner("incarnation")).is_empty());
        assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            fixture.list(&TerminalListFilters::default()).unwrap()[0].lifecycle,
            TerminalLifecycle::Lost
        );
    }

    #[test]
    fn malformed_and_missing_metadata_are_not_silently_filtered() {
        for workspace in [None, Some("/different")] {
            let mut fixture = Fixture::new();
            fixture.disk_with_owner(
                "bad",
                TerminalLifecycle::Running,
                TerminalBackend::Native,
                &owner("incarnation"),
                workspace,
            );
            assert_eq!(
                fixture
                    .list(&TerminalListFilters {
                        task_id: Some("nonmatching".into()),
                        ..TerminalListFilters::default()
                    })
                    .unwrap_err(),
                TerminalCatalogViewError::Invalid
            );
            let journal = TerminalJournal::open_for_retention(
                fixture.catalog.open(&id("bad")).unwrap(),
                &id("bad"),
            )
            .unwrap();
            let state = journal.load_state().unwrap().unwrap();
            let (facts, _) =
                TerminalSessionFacts::decode(&state.bytes, &id("bad"), &state.source).unwrap();
            assert_eq!(
                facts.context.lifecycle,
                TerminalLifecycle::Running,
                "invalid namespace cannot publish recovery"
            );
        }
    }

    #[test]
    fn wrong_record_owner_and_wrong_profile_fail_closed() {
        let mut fixture = Fixture::new();
        fixture.disk_with_owner(
            "bad",
            TerminalLifecycle::Running,
            TerminalBackend::Native,
            &owner("different"),
            Some("/workspace"),
        );
        assert_eq!(
            fixture.list(&TerminalListFilters::default()).unwrap_err(),
            TerminalCatalogViewError::Invalid
        );
        let other = Fixture::new();
        let result = list_with(
            &mut fixture.registry,
            &other.store,
            &fixture.catalog,
            budget(),
            &owner("incarnation"),
            TerminalActorRole::Agent,
            &TerminalAllowedControls::default(),
            &TerminalListFilters::default(),
            1,
        );
        assert_eq!(
            result.unwrap_err(),
            TerminalCatalogViewError::Profile(TerminalProfileStoreError::Invalid)
        );
    }

    #[test]
    fn corrupt_journal_is_error_even_when_filter_excludes_it() {
        let mut fixture = Fixture::new();
        fixture.disk("bad", TerminalLifecycle::Closed, TerminalBackend::Native);
        let path = fixture.session_path("bad").join("tj-meta");
        assert!(path.exists());
        std::fs::write(path, b"corrupt").unwrap();
        assert!(matches!(
            fixture.list(&TerminalListFilters {
                lifecycle: Some(TerminalLifecycle::Running),
                ..TerminalListFilters::default()
            }),
            Err(TerminalCatalogViewError::Journal(
                TerminalJournalError::Corrupt
            ))
        ));
    }

    #[test]
    fn snapshot_pins_exact_membership_and_directory_identity() {
        let mut fixture = Fixture::new();
        fixture.disk("exact", TerminalLifecycle::Closed, TerminalBackend::Native);
        let snapshot = fixture.catalog.snapshot().unwrap();
        assert!(matches!(
            snapshot.open(&id("Exact")),
            Err(TerminalCatalogError::NotFound)
        ));
        let path = fixture.session_path("exact");
        std::fs::rename(&path, fixture.path.join("old-directory")).unwrap();
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        assert!(matches!(
            snapshot.open(&id("exact")),
            Err(TerminalCatalogError::Corrupt)
        ));
        assert_eq!(
            snapshot.validate().unwrap_err(),
            TerminalCatalogError::Corrupt
        );
    }

    #[test]
    fn complete_256_row_catalog_is_not_truncated_or_admitted_to_residency() {
        let mut fixture = Fixture::new();
        for n in 0..256 {
            fixture.disk(
                &format!("history-{n:03}"),
                TerminalLifecycle::Closed,
                TerminalBackend::Native,
            );
        }
        assert_eq!(
            fixture.list(&TerminalListFilters::default()).unwrap().len(),
            256
        );
        assert!(fixture.registry.owner_ids(&owner("incarnation")).is_empty());
        assert!(fixture.directory_overflow_rejected());
        let outside = fixture.path.join("resident-outside-catalog");
        DirBuilder::new().mode(0o700).create(&outside).unwrap();
        let directory = rustix::fs::open(
            &outside,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .unwrap();
        fixture.live_in(id("extra-resident"), directory);
        assert_eq!(
            fixture.list(&TerminalListFilters::default()).unwrap_err(),
            TerminalCatalogViewError::ResourceLimit
        );
        assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
    }
    impl Fixture {
        fn directory_overflow_rejected(&mut self) -> bool {
            matches!(
                self.store
                    .transaction()
                    .unwrap()
                    .create_session(&mut self.catalog, &id("overflow")),
                Err(TerminalProfileStoreError::ResourceLimit)
            )
        }
    }
}
