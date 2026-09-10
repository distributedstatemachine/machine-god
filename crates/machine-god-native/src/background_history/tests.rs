use super::*;
use crate::background_inspection::{
    StoredBackgroundRecord, background_record_name, background_workspace_name,
};
use crate::terminal_journal::{TerminalJournal, TerminalJournalLimits};
use crate::terminal_monitor::{TerminalMonitorContext, TerminalMonitorSet};
use crate::terminal_profile_store::TerminalProfileStore;
use crate::terminal_session_record::{TerminalSessionFacts, test_metadata};
use crate::{NativeBackgroundInspectionErrorKind as Kind, NativeBackgroundState};
use machine_god_core::{BackgroundOutputOwner, SessionId, SessionIncarnationId};
use rustix::fs::{Mode, OFlags};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    state: PathBuf,
    journals: Vec<TerminalJournal>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "mg-bg-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let workspace = root.join("workspace");
        let state = root.join("state");
        private_dir(&workspace);
        private_dir(&state);
        Self {
            root,
            workspace,
            state,
            journals: Vec::new(),
        }
    }
    fn namespace(&self) -> PathBuf {
        self.state.join("machine-god")
    }
    fn environment(&self) -> NativeEnvironment {
        NativeEnvironment::new(None, Some(self.state.clone().into_os_string()), None)
    }
    fn legacy(&self, id: u64, updated_at_ms: u64) -> PathBuf {
        let namespace = self.namespace();
        let background = namespace.join("background-v1");
        let records = background.join(background_workspace_name(self.workspace.to_str().unwrap()));
        for path in [&namespace, &background, &records] {
            private_dir(path);
        }
        let record = StoredBackgroundRecord {
            version: 1,
            workspace: self.workspace.to_str().unwrap().to_owned(),
            id,
            started_at_ms: 0,
            updated_at_ms,
            command: format!("legacy-secret-{id}"),
            cwd: self.workspace.to_str().unwrap().to_owned(),
            state: NativeBackgroundState::Exited,
            pid: None,
            exit_code: Some(0),
            server_url: None,
            diagnostic: None,
        };
        let path = records.join(background_record_name(id));
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }
    fn terminal(&mut self, value: u64, timestamp: i64, command: Option<&str>) -> TerminalSessionId {
        private_dir(&self.namespace());
        let store = TerminalProfileStore::prepare(open_directory(&self.namespace())).unwrap();
        let owner = BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        );
        let id = TerminalSessionId::new(format!("terminal-{value:032x}")).unwrap();
        let mut catalog = store
            .transaction()
            .unwrap()
            .prepare_catalog(self.workspace.to_str().unwrap().into(), owner.clone())
            .unwrap();
        let directory = store
            .transaction()
            .unwrap()
            .create_session(&mut catalog, &id)
            .unwrap();
        let mut journal =
            TerminalJournal::create(directory, id.clone(), TerminalJournalLimits::default())
                .unwrap();
        journal.append(b"retained\0binary\x1b[31m output").unwrap();
        let context = TerminalMonitorContext {
            now_ms: timestamp,
            cursor: journal.latest(),
            lifecycle: TerminalLifecycle::Running,
        };
        let mut facts =
            TerminalSessionFacts::new(id.clone(), &owner, context.clone(), 0, timestamp, None)
                .unwrap();
        let mut metadata = test_metadata();
        metadata.workspace = self.workspace.to_str().unwrap().into();
        metadata.cwd = self.workspace.to_str().unwrap().into();
        metadata.command = command.map(str::to_owned);
        facts.metadata = Some(metadata);
        let monitors = TerminalMonitorSet::new(id.clone(), context).unwrap();
        journal
            .publish_state(journal.latest(), &facts.encode(&monitors).unwrap())
            .unwrap();
        self.journals.push(journal);
        id
    }
    fn inspect(
        &self,
        query: NativeBackgroundHistoryQuery,
    ) -> Result<NativeBackgroundHistoryInspection, NativeBackgroundInspectionError> {
        let mut future =
            inspect_native_background_history(self.environment(), self.workspace.clone(), query);
        let Poll::Ready(result) = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("inspection must finish on first poll");
        };
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.journals.clear();
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn open_directory(path: &Path) -> rustix::fd::OwnedFd {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap()
}
fn bytes_tree(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            result.extend(bytes_tree(&entry.path()));
        } else {
            result.insert(entry.path(), fs::read(entry.path()).unwrap());
        }
    }
    result
}

#[test]
fn missing_hierarchy_is_empty_without_creation_and_futures_are_inert() {
    let fixture = Fixture::new();
    let future = inspect_native_background_history(
        fixture.environment(),
        fixture.workspace.clone(),
        NativeBackgroundHistoryQuery::List,
    );
    drop(future);
    assert!(!fixture.namespace().exists());
    let NativeBackgroundHistoryInspection::List(list) =
        fixture.inspect(NativeBackgroundHistoryQuery::List).unwrap()
    else {
        panic!("list");
    };
    assert!(list.records().is_empty());
    assert!(!list.truncated());
    for query in [
        NativeBackgroundHistoryQuery::Last,
        NativeBackgroundHistoryQuery::Legacy(1),
        NativeBackgroundHistoryQuery::Terminal(
            TerminalSessionId::new(format!("terminal-{:032x}", 1)).unwrap(),
        ),
    ] {
        assert_eq!(fixture.inspect(query).unwrap_err().kind(), Kind::NotFound);
    }
    assert!(!fixture.namespace().exists());
}

#[test]
fn union_order_and_latest_compare_unsigned_and_signed_timestamps_losslessly() {
    let mut fixture = Fixture::new();
    let terminal = fixture.terminal(7, i64::MAX, Some("terminal-secret"));
    fixture.legacy(7, u64::MAX);
    fixture.legacy(8, i64::MAX.unsigned_abs());
    let NativeBackgroundHistoryInspection::List(list) =
        fixture.inspect(NativeBackgroundHistoryQuery::List).unwrap()
    else {
        panic!("list");
    };
    assert_eq!(list.records().len(), 3);
    assert_eq!(
        list.records()[0].id(),
        &NativeBackgroundHistoryId::Legacy(7)
    );
    assert_eq!(list.records()[0].updated_at_ms(), i128::from(u64::MAX));
    assert_eq!(
        list.records()[1].id(),
        &NativeBackgroundHistoryId::Terminal(terminal)
    );
    assert_eq!(
        list.records()[2].id(),
        &NativeBackgroundHistoryId::Legacy(8)
    );
    let NativeBackgroundHistoryInspection::Detail(NativeBackgroundHistoryDetail::Legacy(detail)) =
        fixture.inspect(NativeBackgroundHistoryQuery::Last).unwrap()
    else {
        panic!("legacy latest");
    };
    assert_eq!(detail.id(), 7);
    assert_eq!(detail.command(), "legacy-secret-7");
}

#[test]
fn terminal_latest_uses_same_observation_and_deterministic_identity_ties() {
    let mut fixture = Fixture::new();
    fixture.legacy(1, 10);
    fixture.terminal(1, 20, Some("older identity"));
    let last = fixture.terminal(2, 20, None);
    let before = bytes_tree(&fixture.root);
    let NativeBackgroundHistoryInspection::Detail(NativeBackgroundHistoryDetail::Terminal(detail)) =
        fixture.inspect(NativeBackgroundHistoryQuery::Last).unwrap()
    else {
        panic!("terminal latest");
    };
    assert_eq!(detail.id(), &last);
    assert_eq!(detail.command(), None);
    assert_eq!(detail.lifecycle(), TerminalLifecycle::Running);
    assert_eq!(detail.last_output_ms(), 20);
    assert_eq!(detail.facts_cursor(), detail.latest());
    assert_eq!(
        fixture
            .inspect(NativeBackgroundHistoryQuery::Terminal(last))
            .unwrap(),
        NativeBackgroundHistoryInspection::Detail(NativeBackgroundHistoryDetail::Terminal(detail))
    );
    assert_eq!(bytes_tree(&fixture.root), before);
}

#[test]
fn exact_queries_do_not_inspect_the_other_namespace() {
    let mut fixture = Fixture::new();
    let terminal = fixture.terminal(5, 20, Some("retained"));
    let record = fixture.legacy(5, 10);
    fs::write(&record, b"invalid legacy JSON").unwrap();
    assert!(
        fixture
            .inspect(NativeBackgroundHistoryQuery::Terminal(terminal))
            .is_ok()
    );
    assert_eq!(
        fixture
            .inspect(NativeBackgroundHistoryQuery::List)
            .unwrap_err()
            .kind(),
        Kind::Corrupt
    );
    fixture.legacy(5, 10);
    // A malformed child entry is rejected by terminal topology, not by exact legacy lookup.
    let malformed = fixture.namespace().join("terminal-v1/malformed");
    fs::write(&malformed, b"not a catalog").unwrap();
    assert!(
        fixture
            .inspect(NativeBackgroundHistoryQuery::Legacy(5))
            .is_ok()
    );
    assert!(fixture.inspect(NativeBackgroundHistoryQuery::List).is_err());
}

#[test]
fn last_fails_closed_on_legacy_truncation_and_list_reports_it() {
    let mut fixture = Fixture::new();
    fixture.terminal(1, 5000, Some("terminal"));
    for id in 1..=101 {
        fixture.legacy(id, id);
    }
    let NativeBackgroundHistoryInspection::List(list) =
        fixture.inspect(NativeBackgroundHistoryQuery::List).unwrap()
    else {
        panic!("list");
    };
    assert!(list.truncated());
    assert!(list.records().len() <= MAX_BACKGROUND_HISTORY_RECORDS);
    assert_eq!(
        fixture
            .inspect(NativeBackgroundHistoryQuery::Last)
            .unwrap_err()
            .kind(),
        Kind::ResourceLimit
    );
}

#[test]
fn retained_summary_and_selected_detail_mismatch_is_not_reread_or_reported_latest() {
    let summary = crate::NativeBackgroundRecordSummary::new(
        1,
        NativeBackgroundState::Exited,
        10,
        "first".into(),
        false,
    )
    .unwrap();
    let list = crate::NativeBackgroundList::new(vec![summary], false).unwrap();
    assert_eq!(
        supported::compose(&list, None, vec![], true)
            .unwrap_err()
            .kind(),
        Kind::Corrupt
    );
}

#[test]
fn public_debug_does_not_expose_record_contents_or_workspace() {
    let mut fixture = Fixture::new();
    let id = fixture.terminal(1, 20, Some("secret-command"));
    for query in [
        NativeBackgroundHistoryQuery::List,
        NativeBackgroundHistoryQuery::Terminal(id),
    ] {
        let inspection = fixture.inspect(query).unwrap();
        let text = format!("{inspection:#?}");
        assert!(!text.contains("secret-command"));
        assert!(!text.contains(fixture.workspace.to_str().unwrap()));
    }
}

#[test]
fn fresh_cli_inspects_producer_histories_without_recovery_or_live_control() {
    let mut fixture = Fixture::new();
    fixture.legacy(3, 10);
    let terminal = fixture.terminal(3, 20, Some("printf '\u{1b}[31msecret'"));
    let before = bytes_tree(&fixture.root);
    let binary = std::env::var_os("MACHINE_GOD_CLI_TEST_BINARY").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/machine-god"),
        PathBuf::from,
    );
    assert!(
        binary.is_file(),
        "build the fresh release CLI before background history scenarios"
    );
    for args in [
        vec!["background", "--json"],
        vec!["background", "last", "--json"],
        vec!["background", "--json", terminal.as_str()],
        vec!["background", "3", "--json"],
    ] {
        let output = std::process::Command::new(&binary)
            .current_dir(&fixture.workspace)
            .env_clear()
            .env("XDG_STATE_HOME", &fixture.state)
            .args(&args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty());
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if args.len() == 2 {
            assert_eq!(json["count"], 2);
        } else if args.contains(&"3") {
            assert_eq!(json["id"], 3);
        } else {
            assert_eq!(json["id"], terminal.as_str());
            assert_eq!(json["recorded_only"], true);
        }
        assert!(!output.stdout.contains(&0x1b));
    }
    assert_eq!(bytes_tree(&fixture.root), before);
}
