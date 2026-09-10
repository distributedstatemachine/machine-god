use super::*;
use crate::terminal_catalog::{TerminalCatalog, owner_name};
use crate::terminal_journal::{TerminalJournal, TerminalJournalLimits};
use crate::terminal_monitor::{TerminalMonitorContext, TerminalMonitorSet};
use crate::terminal_profile_store::TerminalProfileStore;
use crate::terminal_session_record::{TerminalSessionFacts, test_metadata};
use machine_god_core::{SessionId, SessionIncarnationId};
use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const WORKSPACE: &str = "/workspace";

struct Fixture {
    path: PathBuf,
    store: TerminalProfileStore,
    catalogs: Vec<TerminalCatalog>,
    journals: Vec<TerminalJournal>,
    directories: Vec<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mg-background-inspect-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = TerminalProfileStore::prepare(open_directory(&path)).unwrap();
        Self {
            path,
            store,
            catalogs: Vec::new(),
            journals: Vec::new(),
            directories: Vec::new(),
        }
    }

    fn add(&mut self, label: &str, workspace: &str, last_output_ms: i64) -> usize {
        let owner = owner(label);
        let id = id(label);
        let mut catalog = self
            .store
            .transaction()
            .unwrap()
            .prepare_catalog(workspace.into(), owner.clone())
            .unwrap();
        let directory = self
            .store
            .transaction()
            .unwrap()
            .create_session(&mut catalog, &id)
            .unwrap();
        let mut journal =
            TerminalJournal::create(directory, id.clone(), TerminalJournalLimits::default())
                .unwrap();
        journal.append(b"retained output").unwrap();
        let context = TerminalMonitorContext {
            now_ms: last_output_ms,
            cursor: journal.latest(),
            lifecycle: TerminalLifecycle::Running,
        };
        let mut facts =
            TerminalSessionFacts::new(id.clone(), &owner, context.clone(), 1, last_output_ms, None)
                .unwrap();
        let mut metadata = test_metadata();
        metadata.workspace = workspace.into();
        metadata.cwd = workspace.into();
        metadata.command = Some(format!("printf secret-{label}"));
        facts.metadata = Some(metadata);
        let monitors = TerminalMonitorSet::new(id, context).unwrap();
        journal
            .publish_state(journal.latest(), &facts.encode(&monitors).unwrap())
            .unwrap();
        self.directories.push(
            self.path
                .join("terminal-v1")
                .join(owner_name(workspace, &owner))
                .join("sessions")
                .join(label),
        );
        self.catalogs.push(catalog);
        self.journals.push(journal);
        self.journals.len() - 1
    }

    fn inspect(
        &self,
        workspace: &str,
        exact: Option<&TerminalSessionId>,
    ) -> Result<Vec<NativeTerminalBackgroundHistoryRecord>> {
        inspect_terminal_background_history(
            open_directory(&self.path),
            workspace,
            exact,
            &CancellationToken::new(),
        )
    }

    fn artifact(&self, index: usize, prefix: &str) -> PathBuf {
        std::fs::read_dir(&self.directories[index])
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(prefix)
            })
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.journals.clear();
        self.catalogs.clear();
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

fn open_directory(path: &Path) -> OwnedFd {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap()
}

fn owner(label: &str) -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new(label).unwrap(),
        SessionIncarnationId::new("incarnation").unwrap(),
    )
}

fn id(label: &str) -> TerminalSessionId {
    TerminalSessionId::new(label).unwrap()
}

fn corrupt_byte(path: &Path, offset: u64) {
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(b"!").unwrap();
}

#[test]
fn producer_live_and_unlocked_histories_are_recorded_without_authority() {
    let mut fixture = Fixture::new();
    fixture.add("first", WORKSPACE, 5);
    fixture.add("second", WORKSPACE, 10);
    let records = fixture.inspect(WORKSPACE, None).unwrap();
    assert_eq!(records.len(), 2);
    let record = &records[0];
    assert_eq!(record.session_id, id("second"));
    assert_eq!(record.owner, owner("second"));
    assert_eq!(record.created_at_ms, 1);
    assert_eq!(record.last_output_ms, 10);
    assert_eq!(record.workspace, WORKSPACE);
    assert_eq!(record.cwd, WORKSPACE);
    assert_eq!(record.command.as_deref(), Some("printf secret-second"));
    assert_eq!(record.lifecycle, TerminalLifecycle::Running);
    assert_eq!(record.outcome, None);
    assert_eq!(record.earliest, TerminalCursor::new(1, 0).unwrap());
    assert_eq!(record.latest, fixture.journals[1].latest());
    assert_eq!(record.facts_cursor, record.latest);
    assert_eq!(
        format!("{record:?}"),
        "NativeTerminalBackgroundHistoryRecord { .. }"
    );
    fixture.journals.clear();
    fixture.catalogs.clear();
    assert_eq!(
        fixture
            .inspect(WORKSPACE, Some(&id("first")))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fixture.inspect(WORKSPACE, Some(&id("absent"))).unwrap_err(),
        Error::NotFound
    );
}

#[test]
fn absent_hierarchy_is_empty_without_creation_and_exact_is_not_found() {
    let fixture = Fixture::new();
    let root = fixture.path.join("unused");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let cancellation = CancellationToken::new();
    assert!(
        inspect_terminal_background_history(open_directory(&root), WORKSPACE, None, &cancellation)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        inspect_terminal_background_history(
            open_directory(&root),
            WORKSPACE,
            Some(&id("missing")),
            &cancellation
        )
        .unwrap_err(),
        Error::NotFound
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    assert_eq!(
        fixture.inspect("relative", None).unwrap_err(),
        Error::Invalid
    );
}

#[test]
fn other_workspaces_filter_only_after_namespace_identity_validation() {
    let mut fixture = Fixture::new();
    fixture.add("first", WORKSPACE, 5);
    fixture.add("second", "/other", 10);
    assert_eq!(fixture.inspect(WORKSPACE, None).unwrap().len(), 1);
    assert_eq!(
        fixture.inspect(WORKSPACE, Some(&id("second"))).unwrap_err(),
        Error::NotFound
    );
    let old_owner = fixture.directories[1].parent().unwrap().parent().unwrap();
    let wrong = fixture.path.join("terminal-v1").join("0".repeat(64));
    std::fs::rename(old_owner, wrong).unwrap();
    assert_eq!(
        fixture.inspect(WORKSPACE, None).unwrap_err(),
        Error::Corrupt
    );
}

#[test]
fn mismatched_workspace_and_session_and_missing_metadata_fail_closed() {
    for kind in ["workspace", "session", "metadata"] {
        let mut fixture = Fixture::new();
        fixture.add("saved", WORKSPACE, 5);
        let state = fixture.journals[0].load_state().unwrap().unwrap();
        let (mut facts, monitor_bytes) =
            TerminalSessionFacts::decode(&state.bytes, &id("saved"), &state.source).unwrap();
        let monitors = facts.restore_monitors(monitor_bytes).unwrap();
        match kind {
            "workspace" => facts.metadata.as_mut().unwrap().workspace = "/other".into(),
            "session" => {
                let replacement = fixture.directories[0].with_file_name("wrong-id");
                std::fs::rename(&fixture.directories[0], replacement).unwrap();
                assert_eq!(
                    fixture.inspect(WORKSPACE, None).unwrap_err(),
                    Error::Corrupt
                );
                continue;
            }
            _ => facts.metadata = None,
        }
        fixture.journals[0]
            .publish_state(state.source, &facts.encode(&monitors).unwrap())
            .unwrap();
        assert_eq!(
            fixture.inspect(WORKSPACE, None).unwrap_err(),
            Error::Corrupt
        );
    }
}

#[test]
fn corrupt_manifest_and_full_state_suffix_are_rejected() {
    for artifact in ["tj-meta", "tj-state-"] {
        let mut fixture = Fixture::new();
        fixture.add("saved", WORKSPACE, 5);
        let path = fixture.artifact(0, artifact);
        let offset = if artifact == "tj-meta" {
            0
        } else {
            std::fs::metadata(&path).unwrap().len() - 1
        };
        corrupt_byte(&path, offset);
        assert_eq!(
            fixture.inspect(WORKSPACE, None).unwrap_err(),
            Error::Corrupt
        );
    }
}

#[test]
fn missing_oversized_and_unsafe_selected_artifacts_fail_closed() {
    for kind in ["missing", "oversized", "symlink", "hardlink", "mode"] {
        let mut fixture = Fixture::new();
        fixture.add("saved", WORKSPACE, 5);
        let path = fixture.directories[0].join("tj-meta");
        match kind {
            "missing" => std::fs::remove_file(&path).unwrap(),
            "oversized" => std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(128 * 1024 + 1)
                .unwrap(),
            "symlink" => {
                let source = path.with_file_name("kept");
                std::fs::rename(&path, &source).unwrap();
                symlink(source, path).unwrap();
            }
            "hardlink" => std::fs::hard_link(&path, path.with_file_name("linked")).unwrap(),
            _ => std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap(),
        }
        assert_eq!(
            fixture.inspect(WORKSPACE, None).unwrap_err(),
            if kind == "oversized" {
                Error::ResourceLimit
            } else {
                Error::Corrupt
            }
        );
    }
}

#[test]
fn raw_suffix_and_orphan_are_unchanged_and_raw_checksum_is_not_claimed() {
    let mut fixture = Fixture::new();
    fixture.add("saved", WORKSPACE, 5);
    let raw = fixture.artifact(0, "tj-raw-");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&raw)
        .unwrap()
        .write_all(b"uncommitted suffix")
        .unwrap();
    let orphan = fixture.directories[0].join("tj-state-18446744073709551615");
    std::fs::write(&orphan, b"uncommitted orphan").unwrap();
    std::fs::set_permissions(&orphan, std::fs::Permissions::from_mode(0o600)).unwrap();
    // Metadata inspection checks raw lengths, not raw content checksums.
    corrupt_byte(&raw, 0);
    let before = snapshot(&fixture.path);
    let records = fixture.inspect(WORKSPACE, None).unwrap();
    assert_eq!(records[0].latest.offset(), b"retained output".len() as u64);
    assert_eq!(snapshot(&fixture.path), before);
}

#[derive(Debug, Eq, PartialEq)]
struct SnapshotEntry {
    path: PathBuf,
    inode: u64,
    len: u64,
    modified: i64,
    modified_ns: i64,
    contents: Vec<u8>,
}

fn snapshot(root: &Path) -> Vec<SnapshotEntry> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            result.extend(snapshot(&path));
        } else {
            result.push(SnapshotEntry {
                path: path.clone(),
                inode: meta.ino(),
                len: meta.len(),
                modified: meta.mtime(),
                modified_ns: meta.mtime_nsec(),
                contents: std::fs::read(path).unwrap(),
            });
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

#[test]
fn exclusive_profile_transaction_is_busy_and_inspection_releases_shared_lock() {
    let mut fixture = Fixture::new();
    fixture.add("saved", WORKSPACE, 5);
    let transaction = fixture.store.transaction().unwrap();
    assert_eq!(fixture.inspect(WORKSPACE, None).unwrap_err(), Error::Busy);
    drop(transaction);
    assert_eq!(fixture.inspect(WORKSPACE, None).unwrap().len(), 1);
    assert!(fixture.store.transaction().is_ok());
}

#[test]
fn replaced_profile_lock_or_session_topology_invalidates_observation() {
    for replace_lock in [true, false] {
        let mut fixture = Fixture::new();
        fixture.add("saved", WORKSPACE, 5);
        let target = if replace_lock {
            fixture.path.join("terminal-v1/profile-lock")
        } else {
            fixture.directories[0].clone()
        };
        BEFORE_FINAL_VALIDATION.with(|callback| {
            *callback.borrow_mut() = Some(Box::new(move || {
                std::fs::rename(&target, target.with_extension("old")).unwrap();
                if replace_lock {
                    std::fs::write(&target, []).unwrap();
                    std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                } else {
                    std::fs::create_dir(&target).unwrap();
                    std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                }
            }));
        });
        assert_eq!(
            fixture.inspect(WORKSPACE, None).unwrap_err(),
            Error::Corrupt
        );
    }
}

#[test]
fn cancellation_is_checked_before_io_and_before_success() {
    let mut fixture = Fixture::new();
    fixture.add("saved", WORKSPACE, 5);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        inspect_terminal_background_history(
            open_directory(&fixture.path),
            WORKSPACE,
            None,
            &cancellation
        )
        .unwrap_err(),
        Error::Cancelled
    );
    let cancellation = CancellationToken::new();
    let cancel_later = cancellation.clone();
    BEFORE_FINAL_VALIDATION.with(|callback| {
        *callback.borrow_mut() = Some(Box::new(move || {
            cancel_later.cancel();
        }));
    });
    assert_eq!(
        inspect_terminal_background_history(
            open_directory(&fixture.path),
            WORKSPACE,
            None,
            &cancellation
        )
        .unwrap_err(),
        Error::Cancelled
    );
    assert!(fixture.store.transaction().is_ok());
}

#[test]
fn aggregate_read_budget_is_exact_and_charged_before_state_read() {
    let mut fixture = Fixture::new();
    fixture.add("saved", WORKSPACE, 5);
    let meta = usize::try_from(
        std::fs::metadata(fixture.directories[0].join("tj-meta"))
            .unwrap()
            .len(),
    )
    .unwrap();
    let state = usize::try_from(
        std::fs::metadata(fixture.artifact(0, "tj-state-"))
            .unwrap()
            .len(),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    for remaining in [meta + state, meta + state - 1] {
        let mut budget = HistoryReadBudget {
            cancellation: &cancellation,
            remaining,
        };
        let result = crate::terminal_journal::inspection::read_history(
            open_directory(&fixture.directories[0]),
            &id("saved"),
            &mut budget,
        );
        if remaining == meta + state {
            assert!(result.is_ok());
            assert_eq!(budget.remaining, 0);
        } else {
            assert!(matches!(result, Err(Error::ResourceLimit)));
        }
    }
}

#[test]
fn row_bound_errors_instead_of_returning_partial_listing() {
    let mut fixture = Fixture::new();
    for index in 0..MAX_TERMINAL_BACKGROUND_RECORDS {
        fixture.add(&format!("session-{index}"), WORKSPACE, 5);
        fixture.journals.clear();
        fixture.catalogs.clear();
    }
    assert_eq!(
        fixture.inspect(WORKSPACE, None).unwrap().len(),
        MAX_TERMINAL_BACKGROUND_RECORDS
    );
    fixture.add("overflow", WORKSPACE, 5);
    assert_eq!(
        fixture.inspect(WORKSPACE, None).unwrap_err(),
        Error::ResourceLimit
    );
    assert_eq!(
        fixture
            .inspect(WORKSPACE, Some(&id("overflow")))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn retained_text_bound_rejects_whole_result_without_shortening_commands() {
    let mut fixture = Fixture::new();
    for index in 0..16 {
        let index = fixture.add(&format!("session-{index}"), WORKSPACE, 5);
        let state = fixture.journals[index].load_state().unwrap().unwrap();
        let (mut facts, monitors) = TerminalSessionFacts::decode(
            &state.bytes,
            &id(&format!("session-{index}")),
            &state.source,
        )
        .unwrap();
        let monitors = facts.restore_monitors(monitors).unwrap();
        facts.metadata.as_mut().unwrap().command = Some("x".repeat(64 * 1024));
        fixture.journals[index]
            .publish_state(state.source, &facts.encode(&monitors).unwrap())
            .unwrap();
        if index == 14 {
            let records = fixture.inspect(WORKSPACE, None).unwrap();
            assert_eq!(records.len(), 15);
            assert!(
                records
                    .iter()
                    .all(|record| record.command.as_ref().unwrap().len() == 64 * 1024)
            );
        }
    }
    assert_eq!(
        fixture.inspect(WORKSPACE, None).unwrap_err(),
        Error::ResourceLimit
    );
}

fn private_directory(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn owner_and_session_namespace_caps_are_complete_or_error() {
    let fixture = Fixture::new();
    let namespace = fixture.path.join("terminal-v1");
    for index in 0..256 {
        private_directory(&namespace.join(format!("{index:064x}")));
    }
    assert!(fixture.inspect(WORKSPACE, None).unwrap().is_empty());
    private_directory(&namespace.join(format!("{:064x}", 256)));
    assert_eq!(
        fixture.inspect(WORKSPACE, None).unwrap_err(),
        Error::ResourceLimit
    );

    let mut fixture = Fixture::new();
    fixture.add("saved", WORKSPACE, 5);
    let sessions = fixture.directories[0].parent().unwrap();
    for index in 0..256 {
        private_directory(&sessions.join(format!("extra-{index}")));
    }
    assert_eq!(
        fixture.inspect(WORKSPACE, None).unwrap_err(),
        Error::ResourceLimit
    );
}

#[test]
fn missing_profile_lock_is_never_created_and_namespace_symlink_is_rejected() {
    let mut fixture = Fixture::new();
    fixture.add("saved", WORKSPACE, 5);
    let lock = fixture.path.join("terminal-v1/profile-lock");
    std::fs::remove_file(&lock).unwrap();
    assert_eq!(
        fixture.inspect(WORKSPACE, None).unwrap_err(),
        Error::Corrupt
    );
    assert!(!lock.exists());

    let mut fixture = Fixture::new();
    fixture.add("saved", WORKSPACE, 5);
    let namespace = fixture.path.join("terminal-v1");
    let moved = fixture.path.join("moved");
    std::fs::rename(&namespace, &moved).unwrap();
    symlink(moved, namespace).unwrap();
    assert_eq!(
        fixture.inspect(WORKSPACE, None).unwrap_err(),
        Error::Corrupt
    );
}

#[test]
fn closed_outcome_and_facts_cursor_are_preserved_without_recovery() {
    let mut fixture = Fixture::new();
    fixture.add("saved", WORKSPACE, 5);
    let state = fixture.journals[0].load_state().unwrap().unwrap();
    let (mut facts, _) =
        TerminalSessionFacts::decode(&state.bytes, &id("saved"), &state.source).unwrap();
    facts.context.lifecycle = TerminalLifecycle::Closed;
    facts.outcome = Some(TerminalProcessOutcome::Exited(7));
    let monitors = TerminalMonitorSet::new(id("saved"), facts.context.clone()).unwrap();
    fixture.journals[0]
        .publish_state(state.source.clone(), &facts.encode(&monitors).unwrap())
        .unwrap();
    fixture.journals[0]
        .append(b"newer output committed before facts")
        .unwrap();
    let records = fixture.inspect(WORKSPACE, None).unwrap();
    assert_eq!(records[0].lifecycle, TerminalLifecycle::Closed);
    assert_eq!(records[0].outcome, Some(TerminalProcessOutcome::Exited(7)));
    assert_eq!(records[0].facts_cursor, state.source);
    assert!(records[0].latest > records[0].facts_cursor);
}
