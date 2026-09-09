use super::*;
use machine_god_core::{SessionIncarnationId, SessionRecord, SessionRevision};
use machine_god_native::{
    NativeSessionCleanupReport, NativeSessionCleanupStatus, NativeSessionMigration,
    NativeSessionRecovery,
};
use std::cell::{Cell, RefCell};

struct Host {
    result: RefCell<Option<Result<Receipt, Failure>>>,
    actions: RefCell<Vec<Action>>,
    calls: Cell<usize>,
}

impl Host {
    fn new(result: Result<Receipt, Failure>) -> Self {
        Self {
            result: RefCell::new(Some(result)),
            actions: RefCell::new(Vec::new()),
            calls: Cell::new(0),
        }
    }
}

impl MaintenanceCommandHost for Host {
    fn execute(&self, action: &Action) -> Result<Receipt, Failure> {
        self.calls.set(self.calls.get() + 1);
        self.actions.borrow_mut().push(action.clone());
        self.result
            .borrow_mut()
            .take()
            .expect("exactly one execution")
    }
}

fn record(id: &str) -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new(id).unwrap(),
        SessionIncarnationId::new("inc").unwrap(),
    );
    record.revision = SessionRevision(1);
    record
        .metadata
        .insert("secret".into(), serde_json::json!("never displayed"));
    record
}

fn invoke(args: &[&str], host: &Host) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = crate::run_with_hosts(
        args.iter().map(OsString::from),
        &mut stdout,
        &mut stderr,
        crate::CommandHosts {
            maintenance: host,
            ..Default::default()
        },
    );
    (
        exit,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn invalid_grammar_never_calls_the_maintenance_host() {
    for args in [
        vec!["session", "migrate"],
        vec!["session", "recover"],
        vec!["session", "migrate", "last"],
        vec!["session", "migrate", "saved", "--allow-large"],
        vec!["session", "recover", "saved", "--json", "--json"],
        vec!["session", "recover", "--json", "saved"],
        vec!["doctor", "cleanup", "--apply", "--apply"],
        vec!["doctor", "cleanup", "--json", "--json"],
        vec!["doctor", "cleanup", "arbitrary/path"],
        vec!["session", "migrate", "saved", "--record"],
        vec!["--add-dir=shared", "doctor", "cleanup", "--apply"],
    ] {
        let host = Host::new(Err(Failure::Unavailable));
        let (exit, stdout, stderr) = invoke(&args, &host);
        assert_eq!(exit, 2, "{args:?}");
        assert_eq!(host.calls.get(), 0);
        assert!(stdout.is_empty());
        assert_eq!(stderr, crate::INVALID_ARGUMENTS);
    }
}

#[test]
fn migration_preserves_native_current_and_changed_receipts_without_content() {
    for current in [false, true] {
        let receipt = if current {
            NativeSessionMigration::AlreadyCurrent(record("saved"))
        } else {
            NativeSessionMigration::Migrated(record("saved"))
        };
        let host = Host::new(Ok(Receipt::Migration(receipt)));
        let (exit, stdout, stderr) = invoke(&["session", "migrate", "saved", "--json"], &host);
        assert_eq!(exit, 0);
        assert!(stderr.is_empty());
        let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(
            value["status"],
            if current {
                "already_current"
            } else {
                "migrated"
            }
        );
        assert_eq!(value["id"], "saved");
        assert!(!stdout.contains("secret"));
        assert_eq!(
            host.actions.borrow().as_slice(),
            &[Action::Migrate(SessionId::new("saved").unwrap())]
        );
    }
}

#[test]
fn recovery_reports_the_separate_identity_and_unknown_effects() {
    let host = Host::new(Ok(Receipt::Recovery(NativeSessionRecovery {
        record: record("copy"),
        truncated_source: true,
        unknown_tool_results: 2,
    })));
    let (exit, stdout, stderr) = invoke(&["session", "recover", "saved", "--json"], &host);
    assert_eq!(exit, 0);
    assert!(stderr.is_empty());
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["source_id"], "saved");
    assert_eq!(value["id"], "copy");
    assert_eq!(value["source_unchanged"], true);
    assert_eq!(value["unknown_tool_results"], 2);
    assert_eq!(value["truncated_source"], true);
    assert!(!stdout.contains("secret"));
}

#[test]
fn cleanup_is_report_only_unless_explicitly_applied_and_reports_skips() {
    use NativeSessionCleanupStatus::{
        ActiveWriter, Completed, Indeterminate, ReportOnly, Untrusted,
    };
    for (apply, statuses, scan_complete, expected_exit) in [
        (false, vec![ReportOnly, ActiveWriter, Untrusted], true, 0),
        (true, vec![Completed], true, 0),
        (true, vec![Completed, ActiveWriter, Untrusted], true, 1),
        (true, vec![Indeterminate], true, 1),
        (false, vec![ReportOnly], false, 1),
    ] {
        let host = Host::new(Ok(Receipt::Cleanup(NativeSessionCleanupReport {
            outcomes: statuses,
            scan_complete,
        })));
        let args = if apply {
            vec!["doctor", "cleanup", "--json", "--apply"]
        } else {
            vec!["doctor", "cleanup", "--json"]
        };
        let (exit, stdout, stderr) = invoke(&args, &host);
        assert_eq!(exit, expected_exit);
        assert!(stderr.is_empty());
        let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["apply"], apply);
        assert_eq!(value["scan_complete"], scan_complete);
        assert_eq!(
            host.actions.borrow().as_slice(),
            &[Action::Cleanup { apply }]
        );
    }
}

#[test]
fn mismatched_receipts_and_impossible_report_only_deletions_are_not_success() {
    for receipt in [
        Receipt::Migration(NativeSessionMigration::Migrated(record("other"))),
        Receipt::Recovery(NativeSessionRecovery {
            record: record("copy"),
            truncated_source: false,
            unknown_tool_results: 0,
        }),
    ] {
        let host = Host::new(Ok(receipt));
        let (exit, stdout, stderr) = invoke(&["session", "migrate", "saved"], &host);
        assert_eq!(exit, 1);
        assert!(stdout.is_empty());
        assert!(stderr.contains("Unavailable"));
        assert_eq!(host.calls.get(), 1);
    }
    let host = Host::new(Ok(Receipt::Cleanup(NativeSessionCleanupReport {
        outcomes: vec![NativeSessionCleanupStatus::Completed],
        scan_complete: true,
    })));
    assert_eq!(invoke(&["doctor", "cleanup"], &host).0, 1);
}

#[test]
fn indeterminate_failure_is_redacted_and_never_retried() {
    for json in [false, true] {
        let host = Host::new(Err(Failure::Indeterminate));
        let args = if json {
            vec!["session", "migrate", "saved", "--json"]
        } else {
            vec!["session", "migrate", "saved"]
        };
        let (exit, stdout, stderr) = invoke(&args, &host);
        assert_eq!(exit, 1);
        assert_eq!(host.calls.get(), 1);
        let visible = if json {
            assert!(stderr.is_empty());
            stdout
        } else {
            assert!(stdout.is_empty());
            stderr
        };
        assert!(visible.contains("Indeterminate"));
        assert!(visible.contains("reload before retrying uncertain effects"));
    }
}

#[test]
fn uncertain_recovery_retains_the_exact_copy_to_reload_without_success_claims() {
    let host = Host::new(Ok(Receipt::RecoveryIndeterminate {
        session_id: SessionId::new("uncertain-copy").unwrap(),
    }));
    let (exit, stdout, stderr) = invoke(&["session", "recover", "source", "--json"], &host);
    assert_eq!(exit, 1);
    assert!(stderr.is_empty());
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["status"], "indeterminate");
    assert_eq!(value["id"], "uncertain-copy");
    assert_eq!(value["source_id"], "source");
    assert!(value.get("revision").is_none());
    assert_eq!(host.calls.get(), 1);
}

#[test]
fn output_failure_happens_after_the_single_native_receipt_and_does_not_retry() {
    struct FailedOutput;
    impl io::Write for FailedOutput {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let host = Host::new(Ok(Receipt::Migration(NativeSessionMigration::Migrated(
        record("saved"),
    ))));
    let options = parse_session(false, ["saved", "--json"].map(OsString::from)).unwrap();
    let mut stderr = Vec::new();
    assert_eq!(run(&host, &options, &mut FailedOutput, &mut stderr), 1);
    assert_eq!(host.calls.get(), 1);
    assert_eq!(stderr, crate::OUTPUT_FAILURE.as_bytes());
}

#[test]
fn oversized_cleanup_receipts_fail_before_publishing_partial_output() {
    let host = Host::new(Ok(Receipt::Cleanup(NativeSessionCleanupReport {
        outcomes: vec![NativeSessionCleanupStatus::Untrusted; 1025],
        scan_complete: false,
    })));
    let (exit, stdout, stderr) = invoke(&["doctor", "cleanup", "--json"], &host);
    assert_eq!(exit, 1);
    assert!(stderr.is_empty());
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["code"], "Oversized");
    assert!(value.get("completed").is_none());
}
