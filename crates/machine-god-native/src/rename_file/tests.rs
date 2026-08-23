#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(label: &str) -> Self {
        for suffix in 0..1_000_u64 {
            let path = std::env::temp_dir().join(format!(
                "machine-god-rename-private-{label}-{}-{suffix}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create private rename test directory: {error}"),
            }
        }
        panic!("allocate private rename test directory")
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Open(RenamePhase, RenameOpenSite, usize),
    Fstat(RenamePhase, RenameFstatSite, usize),
    Statat(RenamePhase, RenameStatatSite, usize),
    Rename,
    Sync(RenameSyncSide, usize),
}

struct TraceEvidence {
    operations: Vec<Operation>,
    checkpoints: Vec<RenameCheckpoint>,
    fault: Option<Operation>,
    cancellation: Option<CancellationToken>,
    cancel_checkpoint: Option<RenameCheckpoint>,
    cancel_on_fault: bool,
    rename_calls: usize,
}

impl TraceEvidence {
    fn new() -> Self {
        Self {
            operations: Vec::new(),
            checkpoints: Vec::new(),
            fault: None,
            cancellation: None,
            cancel_checkpoint: None,
            cancel_on_fault: false,
            rename_calls: 0,
        }
    }

    fn record(&mut self, operation: Operation) -> Result<(), rustix::io::Errno> {
        self.operations.push(operation);
        if self.fault == Some(operation) {
            if self.cancel_on_fault {
                assert!(
                    self.cancellation
                        .as_ref()
                        .expect("fault cancellation token")
                        .cancel()
                );
            }
            return Err(rustix::io::Errno::IO);
        }
        Ok(())
    }
}

impl RenameFileEvidence for TraceEvidence {
    fn checkpoint(&mut self, checkpoint: RenameCheckpoint, _cancellation: &CancellationToken) {
        self.checkpoints.push(checkpoint);
        if self.cancel_checkpoint == Some(checkpoint) {
            let _ = self
                .cancellation
                .as_ref()
                .expect("checkpoint cancellation token")
                .cancel();
        }
    }

    fn open_walk(
        &mut self,
        phase: RenamePhase,
        site: RenameOpenSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        self.record(Operation::Open(phase, site, ordinal))?;
        rustix::fs::openat(parent, component, open_flags(site), Mode::empty())
    }

    fn fstat(
        &mut self,
        phase: RenamePhase,
        site: RenameFstatSite,
        ordinal: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        self.record(Operation::Fstat(phase, site, ordinal))?;
        rustix::fs::fstat(descriptor)
    }

    fn statat(
        &mut self,
        phase: RenamePhase,
        site: RenameStatatSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        self.record(Operation::Statat(phase, site, ordinal))?;
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn rename(
        &mut self,
        source_parent: BorrowedFd<'_>,
        source_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
    ) -> Result<(), rustix::io::Errno> {
        self.record(Operation::Rename)?;
        self.rename_calls += 1;
        rustix::fs::renameat_with(
            source_parent,
            source_name,
            destination_parent,
            destination_name,
            RenameFlags::NOREPLACE,
        )
    }

    fn sync_parent(
        &mut self,
        side: RenameSyncSide,
        attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.record(Operation::Sync(side, attempt))?;
        rustix::fs::fsync(parent)
    }
}

fn nested_fixture(label: &str) -> (TempDirectory, RenameFileTool) {
    let temporary = TempDirectory::new(label);
    fs::create_dir(temporary.path().join("old-parent")).unwrap();
    fs::create_dir(temporary.path().join("new-parent")).unwrap();
    fs::write(temporary.path().join("old-parent/source"), b"source").unwrap();
    let tool = RenameFileTool::open(temporary.path()).unwrap();
    (temporary, tool)
}

fn execute_with(
    tool: &RenameFileTool,
    cancellation: &CancellationToken,
    evidence: &mut TraceEvidence,
) -> Result<ToolOutput, ToolError> {
    tool.execute_supported_with_evidence(
        "old-parent/source",
        "new-parent/destination",
        cancellation,
        evidence,
    )
}

fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("invoke mkfifo for private rename evidence");
    assert!(status.success(), "mkfifo failed with {status}");
}

fn assert_cancelled(error: &ToolError) {
    assert_eq!(error.kind, ToolErrorKind::Cancelled);
    assert_eq!(error.code, "rename_file_cancelled");
    assert_eq!(error.message, "rename_file execution was cancelled");
    assert!(!error.retryable);
}

#[test]
fn serialized_argument_and_result_guards_accept_exact_and_reject_one_over() {
    for limit in [
        MAX_RENAME_FILE_SERIALIZED_ARGUMENT_BYTES,
        MAX_RENAME_FILE_SERIALIZED_RESULT_BYTES,
    ] {
        let overhead = serde_json::to_vec(&json!({"padding": ""})).unwrap().len();
        let exact = json!({"padding": "x".repeat(limit - overhead)});
        assert_eq!(serde_json::to_vec(&exact).unwrap().len(), limit);
        assert!(serialized_value_fits(&exact, limit));
        let over = json!({"padding": "x".repeat(limit - overhead + 1)});
        assert_eq!(serde_json::to_vec(&over).unwrap().len(), limit + 1);
        assert!(!serialized_value_fits(&over, limit));
    }

    let output = build_success_output("old", "new").unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!({"old_path": "old", "new_path": "new"}))
    );
}

#[test]
fn real_pipeline_trace_covers_both_endpoints_phases_one_rename_and_ordered_syncs() {
    let (temporary, tool) = nested_fixture("trace");
    let mut evidence = TraceEvidence::new();
    let output = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap();

    assert_eq!(
        output,
        ToolOutput::success(json!({
            "old_path": "old-parent/source",
            "new_path": "new-parent/destination"
        }))
    );
    assert_eq!(evidence.rename_calls, 1);
    assert_eq!(
        evidence
            .operations
            .iter()
            .copied()
            .filter(|operation| matches!(operation, Operation::Sync(_, _)))
            .collect::<Vec<_>>(),
        [
            Operation::Sync(RenameSyncSide::Source, 0),
            Operation::Sync(RenameSyncSide::Destination, 0)
        ]
    );
    for phase in [RenamePhase::Initial, RenamePhase::Revalidate] {
        for endpoint in [RenameEndpoint::Source, RenameEndpoint::Destination] {
            assert!(evidence.operations.iter().any(|operation| matches!(
                operation,
                Operation::Open(candidate_phase, RenameOpenSite::Root(candidate_endpoint), _)
                    if *candidate_phase == phase && *candidate_endpoint == endpoint
            )));
            assert!(evidence.operations.iter().any(|operation| matches!(
                operation,
                Operation::Fstat(candidate_phase, RenameFstatSite::FinalParent(candidate_endpoint), _)
                    if *candidate_phase == phase && *candidate_endpoint == endpoint
            )));
        }
    }
    assert!(
        evidence
            .checkpoints
            .contains(&RenameCheckpoint::FinalPreRename)
    );
    assert!(
        evidence
            .checkpoints
            .contains(&RenameCheckpoint::AfterRename)
    );
    assert!(!temporary.path().join("old-parent/source").exists());
    assert_eq!(
        fs::read(temporary.path().join("new-parent/destination")).unwrap(),
        b"source"
    );
}

#[test]
fn every_real_precommit_checkpoint_honors_cancellation_without_rename() {
    let (_trace_directory, trace_tool) = nested_fixture("checkpoint-trace");
    let mut trace = TraceEvidence::new();
    execute_with(&trace_tool, &CancellationToken::new(), &mut trace).unwrap();
    let final_index = trace
        .checkpoints
        .iter()
        .position(|checkpoint| *checkpoint == RenameCheckpoint::FinalPreRename)
        .unwrap();
    let checkpoints = trace.checkpoints[..=final_index].to_vec();

    for (index, checkpoint) in checkpoints.into_iter().enumerate() {
        let (temporary, tool) = nested_fixture(&format!("cancel-{index}"));
        let cancellation = CancellationToken::new();
        let mut evidence = TraceEvidence::new();
        evidence.cancellation = Some(cancellation.clone());
        evidence.cancel_checkpoint = Some(checkpoint);
        let error = execute_with(&tool, &cancellation, &mut evidence).unwrap_err();
        assert_cancelled(&error);
        assert_eq!(evidence.rename_calls, 0, "checkpoint {checkpoint:?}");
        assert_eq!(
            fs::read(temporary.path().join("old-parent/source")).unwrap(),
            b"source"
        );
        assert!(!temporary.path().join("new-parent/destination").exists());
    }
}

#[test]
fn operation_faults_are_precommit_and_same_call_cancellation_wins() {
    let (_trace_directory, trace_tool) = nested_fixture("operation-trace");
    let mut trace = TraceEvidence::new();
    execute_with(&trace_tool, &CancellationToken::new(), &mut trace).unwrap();
    let operations = trace
        .operations
        .into_iter()
        .take_while(|operation| *operation != Operation::Rename)
        .collect::<Vec<_>>();

    for (index, operation) in operations.into_iter().enumerate() {
        let (temporary, tool) = nested_fixture(&format!("fault-{index}"));
        let mut fault = TraceEvidence::new();
        fault.fault = Some(operation);
        let error = execute_with(&tool, &CancellationToken::new(), &mut fault).unwrap_err();
        assert_ne!(error.kind, ToolErrorKind::Cancelled);
        assert_eq!(fault.rename_calls, 0, "operation {operation:?}");
        assert!(temporary.path().join("old-parent/source").exists());

        let cancellation = CancellationToken::new();
        let mut cancelled_fault = TraceEvidence::new();
        cancelled_fault.fault = Some(operation);
        cancelled_fault.cancel_on_fault = true;
        cancelled_fault.cancellation = Some(cancellation.clone());
        let error = execute_with(&tool, &cancellation, &mut cancelled_fault).unwrap_err();
        assert_cancelled(&error);
        assert_eq!(cancelled_fault.rename_calls, 0);
    }
}

enum InitialRace {
    ReplaceSource,
    CreateDestination,
}

struct InitialRaceEvidence {
    race: InitialRace,
}

impl RenameFileEvidence for InitialRaceEvidence {
    #[allow(clippy::too_many_arguments)]
    fn after_validation(
        &mut self,
        phase: RenamePhase,
        source_parent: BorrowedFd<'_>,
        source_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        if phase != RenamePhase::Initial {
            return Ok(());
        }
        match self.race {
            InitialRace::ReplaceSource => {
                rustix::fs::renameat(source_parent, source_name, source_parent, "displaced")
                    .unwrap();
                rustix::fs::renameat(source_parent, "replacement", source_parent, source_name)
                    .unwrap();
            }
            InitialRace::CreateDestination => {
                rustix::fs::renameat(
                    destination_parent,
                    "intruder",
                    destination_parent,
                    destination_name,
                )
                .unwrap();
            }
        }
        Ok(())
    }
}

#[test]
fn source_identity_and_destination_absence_are_revalidated_before_rename() {
    for race in [InitialRace::ReplaceSource, InitialRace::CreateDestination] {
        let (temporary, tool) = nested_fixture("revalidation");
        fs::write(
            temporary.path().join("old-parent/replacement"),
            b"replacement",
        )
        .unwrap();
        fs::write(temporary.path().join("new-parent/intruder"), b"intruder").unwrap();
        let mut evidence = InitialRaceEvidence { race };
        let error = tool
            .execute_supported_with_evidence(
                "old-parent/source",
                "new-parent/destination",
                &CancellationToken::new(),
                &mut evidence,
            )
            .unwrap_err();
        assert_eq!(error.code, "rename_file_target_changed");
        assert!(
            !temporary.path().join("new-parent/destination").is_file()
                || fs::read(temporary.path().join("new-parent/destination")).unwrap()
                    == b"intruder"
        );
    }
}

#[derive(Clone, Copy)]
enum TerminalRenameBehavior {
    Native,
    Error(rustix::io::Errno),
    CommitThenError(rustix::io::Errno),
}

#[derive(Clone, Copy)]
enum PublishedBehavior {
    Native,
    Error(rustix::io::Errno),
    Alternate(&'static str),
}

struct TerminalEvidence {
    rename_behavior: TerminalRenameBehavior,
    published_behavior: PublishedBehavior,
    cancel_after_rename: Option<CancellationToken>,
    checkpoint_cancellation: Option<CancellationToken>,
    cancel_checkpoint: Option<RenameCheckpoint>,
    source_interruptions: usize,
    destination_interruptions: usize,
    source_sync_error: Option<rustix::io::Errno>,
    destination_sync_error: Option<rustix::io::Errno>,
    rename_calls: usize,
    sync_calls: Vec<(RenameSyncSide, usize)>,
    published_checks: usize,
}

impl TerminalEvidence {
    fn new(rename_behavior: TerminalRenameBehavior) -> Self {
        Self {
            rename_behavior,
            published_behavior: PublishedBehavior::Native,
            cancel_after_rename: None,
            checkpoint_cancellation: None,
            cancel_checkpoint: None,
            source_interruptions: 0,
            destination_interruptions: 0,
            source_sync_error: None,
            destination_sync_error: None,
            rename_calls: 0,
            sync_calls: Vec::new(),
            published_checks: 0,
        }
    }
}

impl RenameFileEvidence for TerminalEvidence {
    fn checkpoint(&mut self, checkpoint: RenameCheckpoint, _cancellation: &CancellationToken) {
        if self.cancel_checkpoint == Some(checkpoint) {
            let _ = self
                .checkpoint_cancellation
                .as_ref()
                .expect("checkpoint cancellation token")
                .cancel();
        }
    }

    fn statat(
        &mut self,
        _phase: RenamePhase,
        site: RenameStatatSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        if site == RenameStatatSite::PublishedDestination {
            self.published_checks += 1;
            return match self.published_behavior {
                PublishedBehavior::Native => {
                    rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                }
                PublishedBehavior::Error(error) => Err(error),
                PublishedBehavior::Alternate(alternate) => {
                    rustix::fs::statat(parent, alternate, AtFlags::SYMLINK_NOFOLLOW)
                }
            };
        }
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn rename(
        &mut self,
        source_parent: BorrowedFd<'_>,
        source_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
    ) -> Result<(), rustix::io::Errno> {
        self.rename_calls += 1;
        let native = || {
            rustix::fs::renameat_with(
                source_parent,
                source_name,
                destination_parent,
                destination_name,
                RenameFlags::NOREPLACE,
            )
        };
        match self.rename_behavior {
            TerminalRenameBehavior::Native => native(),
            TerminalRenameBehavior::Error(error) => Err(error),
            TerminalRenameBehavior::CommitThenError(error) => {
                native()?;
                Err(error)
            }
        }
    }

    fn after_rename(
        &mut self,
        _retained_source: BorrowedFd<'_>,
        _source_parent: BorrowedFd<'_>,
        _source_name: &str,
        _destination_parent: BorrowedFd<'_>,
        _destination_name: &str,
        _outcome: Result<(), rustix::io::Errno>,
        _cancellation: &CancellationToken,
    ) -> Result<(), rustix::io::Errno> {
        if let Some(cancellation) = &self.cancel_after_rename {
            let _ = cancellation.cancel();
        }
        Ok(())
    }

    fn sync_parent(
        &mut self,
        side: RenameSyncSide,
        attempt: usize,
        _parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.sync_calls.push((side, attempt));
        let (interruptions, terminal_error) = match side {
            RenameSyncSide::Source => (self.source_interruptions, self.source_sync_error),
            RenameSyncSide::Destination => {
                (self.destination_interruptions, self.destination_sync_error)
            }
        };
        if attempt < interruptions {
            Err(rustix::io::Errno::INTR)
        } else if let Some(error) = terminal_error {
            Err(error)
        } else {
            Ok(())
        }
    }
}

fn execute_terminal(
    tool: &RenameFileTool,
    cancellation: &CancellationToken,
    evidence: &mut TerminalEvidence,
) -> Result<ToolOutput, ToolError> {
    tool.execute_supported_with_evidence(
        "old-parent/source",
        "new-parent/destination",
        cancellation,
        evidence,
    )
}

fn assert_commit_ambiguous(error: &ToolError) {
    assert_eq!(error.kind, ToolErrorKind::Execution);
    assert_eq!(error.code, "rename_file_commit_ambiguous");
    assert_eq!(error.message, "requested file rename status is uncertain");
    assert!(!error.retryable);
}

#[test]
fn interrupted_rename_is_once_ambiguous_syncs_both_parents_and_ignores_late_cancel() {
    for committed in [false, true] {
        let (temporary, tool) = nested_fixture(if committed {
            "committed-eintr"
        } else {
            "uncommitted-eintr"
        });
        let cancellation = CancellationToken::new();
        let behavior = if committed {
            TerminalRenameBehavior::CommitThenError(rustix::io::Errno::INTR)
        } else {
            TerminalRenameBehavior::Error(rustix::io::Errno::INTR)
        };
        let mut evidence = TerminalEvidence::new(behavior);
        evidence.checkpoint_cancellation = Some(cancellation.clone());
        evidence.cancel_checkpoint = Some(RenameCheckpoint::AfterRename);

        let error = execute_terminal(&tool, &cancellation, &mut evidence).unwrap_err();

        assert_commit_ambiguous(&error);
        assert!(cancellation.is_cancelled());
        assert_eq!(evidence.rename_calls, 1);
        assert_eq!(evidence.published_checks, 0);
        assert_eq!(
            evidence.sync_calls,
            [
                (RenameSyncSide::Source, 0),
                (RenameSyncSide::Destination, 0)
            ]
        );
        assert_eq!(
            temporary.path().join("old-parent/source").exists(),
            !committed
        );
        assert_eq!(
            temporary.path().join("new-parent/destination").exists(),
            committed
        );
    }
}

#[test]
fn definitive_rename_errors_map_once_without_sync_and_same_call_cancel_wins() {
    let cases = [
        (
            rustix::io::Errno::EXIST,
            ToolErrorKind::Execution,
            "rename_file_destination_exists",
            false,
        ),
        (
            rustix::io::Errno::ACCESS,
            ToolErrorKind::PermissionDenied,
            "rename_file_permission_denied",
            false,
        ),
        (
            rustix::io::Errno::PERM,
            ToolErrorKind::PermissionDenied,
            "rename_file_permission_denied",
            false,
        ),
        (
            rustix::io::Errno::ROFS,
            ToolErrorKind::PermissionDenied,
            "rename_file_permission_denied",
            false,
        ),
        (
            rustix::io::Errno::XDEV,
            ToolErrorKind::Unavailable,
            "rename_file_unsupported_filesystem",
            false,
        ),
        (
            rustix::io::Errno::NOSYS,
            ToolErrorKind::Unavailable,
            "rename_file_unsupported_filesystem",
            false,
        ),
        (
            rustix::io::Errno::NOTSUP,
            ToolErrorKind::Unavailable,
            "rename_file_unsupported_filesystem",
            false,
        ),
        (
            rustix::io::Errno::INVAL,
            ToolErrorKind::Unavailable,
            "rename_file_unsupported_filesystem",
            false,
        ),
        (
            rustix::io::Errno::NOENT,
            ToolErrorKind::Execution,
            "rename_file_target_changed",
            true,
        ),
        (
            rustix::io::Errno::LOOP,
            ToolErrorKind::Execution,
            "rename_file_target_changed",
            true,
        ),
        (
            rustix::io::Errno::NOTDIR,
            ToolErrorKind::Execution,
            "rename_file_target_changed",
            true,
        ),
        (
            rustix::io::Errno::ISDIR,
            ToolErrorKind::Execution,
            "rename_file_target_changed",
            true,
        ),
        (
            rustix::io::Errno::IO,
            ToolErrorKind::Execution,
            "rename_file_rename_failed",
            true,
        ),
    ];
    for (index, (os_error, kind, code, retryable)) in cases.into_iter().enumerate() {
        let (temporary, tool) = nested_fixture(&format!("rename-error-{index}"));
        let mut evidence = TerminalEvidence::new(TerminalRenameBehavior::Error(os_error));
        let error = execute_terminal(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.code, code);
        assert_eq!(error.retryable, retryable);
        assert_eq!(evidence.rename_calls, 1);
        assert!(evidence.sync_calls.is_empty());
        assert!(temporary.path().join("old-parent/source").exists());
        assert!(!temporary.path().join("new-parent/destination").exists());
    }

    let (_temporary, tool) = nested_fixture("rename-error-cancelled");
    let cancellation = CancellationToken::new();
    let mut evidence = TerminalEvidence::new(TerminalRenameBehavior::Error(rustix::io::Errno::IO));
    evidence.cancel_after_rename = Some(cancellation.clone());
    let error = execute_terminal(&tool, &cancellation, &mut evidence).unwrap_err();
    assert_cancelled(&error);
    assert_eq!(evidence.rename_calls, 1);
    assert!(evidence.sync_calls.is_empty());
}

#[test]
fn postcommit_identity_failures_still_attempt_both_parent_syncs() {
    for (label, published_behavior) in [
        (
            "published-missing",
            PublishedBehavior::Error(rustix::io::Errno::NOENT),
        ),
        ("published-wrong", PublishedBehavior::Alternate("intruder")),
    ] {
        let (temporary, tool) = nested_fixture(label);
        fs::write(
            temporary.path().join("new-parent/intruder"),
            b"different inode",
        )
        .unwrap();
        let mut evidence = TerminalEvidence::new(TerminalRenameBehavior::Native);
        evidence.published_behavior = published_behavior;
        let error = execute_terminal(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
        assert_commit_ambiguous(&error);
        assert_eq!(evidence.rename_calls, 1);
        assert_eq!(evidence.published_checks, 1);
        assert_eq!(
            evidence.sync_calls,
            [
                (RenameSyncSide::Source, 0),
                (RenameSyncSide::Destination, 0)
            ]
        );
        assert_eq!(
            fs::read(temporary.path().join("new-parent/destination")).unwrap(),
            b"source"
        );
    }
}

#[test]
fn late_cancel_after_success_is_ignored_while_identity_and_sync_complete() {
    let (temporary, tool) = nested_fixture("late-cancel-success");
    let cancellation = CancellationToken::new();
    let mut evidence = TerminalEvidence::new(TerminalRenameBehavior::Native);
    evidence.checkpoint_cancellation = Some(cancellation.clone());
    evidence.cancel_checkpoint = Some(RenameCheckpoint::AfterRename);

    let output = execute_terminal(&tool, &cancellation, &mut evidence).unwrap();

    assert_eq!(
        output,
        ToolOutput::success(json!({
            "old_path": "old-parent/source",
            "new_path": "new-parent/destination"
        }))
    );
    assert!(cancellation.is_cancelled());
    assert_eq!(evidence.published_checks, 1);
    assert_eq!(
        evidence.sync_calls,
        [
            (RenameSyncSide::Source, 0),
            (RenameSyncSide::Destination, 0)
        ]
    );
    assert_eq!(
        fs::read(temporary.path().join("new-parent/destination")).unwrap(),
        b"source"
    );
}

#[test]
fn same_parent_syncs_once_and_distinct_parent_sync_failure_still_attempts_second() {
    let temporary = TempDirectory::new("same-parent-sync");
    fs::write(temporary.path().join("source"), b"same parent").unwrap();
    let tool = RenameFileTool::open(temporary.path()).unwrap();
    let mut evidence = TerminalEvidence::new(TerminalRenameBehavior::Native);
    tool.execute_supported_with_evidence(
        "source",
        "destination",
        &CancellationToken::new(),
        &mut evidence,
    )
    .unwrap();
    assert_eq!(evidence.sync_calls, [(RenameSyncSide::Source, 0)]);

    let temporary = TempDirectory::new("same-parent-sync-bound");
    fs::write(temporary.path().join("source"), b"same parent bound").unwrap();
    let tool = RenameFileTool::open(temporary.path()).unwrap();
    let mut evidence = TerminalEvidence::new(TerminalRenameBehavior::Native);
    evidence.source_interruptions = 16;
    let error = tool
        .execute_supported_with_evidence(
            "source",
            "destination",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err();
    assert_commit_ambiguous(&error);
    assert_eq!(
        evidence.sync_calls,
        (0..16)
            .map(|attempt| (RenameSyncSide::Source, attempt))
            .collect::<Vec<_>>()
    );

    let (temporary, tool) = nested_fixture("cross-parent-sync-failure");
    let mut evidence = TerminalEvidence::new(TerminalRenameBehavior::Native);
    evidence.source_sync_error = Some(rustix::io::Errno::IO);
    let error = execute_terminal(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
    assert_commit_ambiguous(&error);
    assert_eq!(
        evidence.sync_calls,
        [
            (RenameSyncSide::Source, 0),
            (RenameSyncSide::Destination, 0)
        ]
    );
    assert_eq!(
        fs::read(temporary.path().join("new-parent/destination")).unwrap(),
        b"source"
    );
}

#[test]
fn each_parent_sync_accepts_fifteen_interruptions_and_bounds_the_sixteenth() {
    for side in [RenameSyncSide::Source, RenameSyncSide::Destination] {
        for (interruptions, succeeds) in [(15_usize, true), (16, false)] {
            let (temporary, tool) = nested_fixture(&format!("sync-bound-{side:?}-{interruptions}"));
            let mut evidence = TerminalEvidence::new(TerminalRenameBehavior::Native);
            match side {
                RenameSyncSide::Source => evidence.source_interruptions = interruptions,
                RenameSyncSide::Destination => {
                    evidence.destination_interruptions = interruptions;
                }
            }
            let result = execute_terminal(&tool, &CancellationToken::new(), &mut evidence);
            if succeeds {
                result.unwrap();
            } else {
                assert_commit_ambiguous(&result.unwrap_err());
            }
            assert_eq!(
                evidence
                    .sync_calls
                    .iter()
                    .filter(|(candidate, _)| *candidate == side)
                    .count(),
                16
            );
            assert!(
                evidence
                    .sync_calls
                    .iter()
                    .any(|(candidate, _)| *candidate != side),
                "the other distinct parent must still be attempted"
            );
            assert_eq!(
                fs::read(temporary.path().join("new-parent/destination")).unwrap(),
                b"source"
            );
        }
    }
}

enum FinalWindowRace {
    ReplaceSource(&'static str),
    UnlinkSourceThenReplace,
    CreateDestination,
}

struct FinalWindowEvidence {
    race: FinalWindowRace,
    rename_calls: usize,
    sync_calls: Vec<(RenameSyncSide, usize)>,
    retained_source_was_unlinked: bool,
}

impl RenameFileEvidence for FinalWindowEvidence {
    fn final_pre_rename(
        &mut self,
        source_parent: BorrowedFd<'_>,
        source_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        match self.race {
            FinalWindowRace::ReplaceSource(replacement) => {
                rustix::fs::renameat(source_parent, source_name, source_parent, "displaced")
                    .unwrap();
                rustix::fs::renameat(source_parent, replacement, source_parent, source_name)
                    .unwrap();
            }
            FinalWindowRace::UnlinkSourceThenReplace => {
                rustix::fs::renameat(source_parent, source_name, source_parent, "displaced")
                    .unwrap();
                rustix::fs::unlinkat(source_parent, "displaced", AtFlags::empty()).unwrap();
                rustix::fs::renameat(
                    source_parent,
                    "replacement-file",
                    source_parent,
                    source_name,
                )
                .unwrap();
            }
            FinalWindowRace::CreateDestination => {
                rustix::fs::renameat(
                    destination_parent,
                    "intruder",
                    destination_parent,
                    destination_name,
                )
                .unwrap();
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn after_rename(
        &mut self,
        retained_source: BorrowedFd<'_>,
        _source_parent: BorrowedFd<'_>,
        _source_name: &str,
        _destination_parent: BorrowedFd<'_>,
        _destination_name: &str,
        _outcome: Result<(), rustix::io::Errno>,
        _cancellation: &CancellationToken,
    ) -> Result<(), rustix::io::Errno> {
        if matches!(self.race, FinalWindowRace::UnlinkSourceThenReplace) {
            let metadata = rustix::fs::fstat(retained_source)?;
            self.retained_source_was_unlinked = metadata.st_nlink == 0;
        }
        Ok(())
    }

    fn rename(
        &mut self,
        source_parent: BorrowedFd<'_>,
        source_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
    ) -> Result<(), rustix::io::Errno> {
        self.rename_calls += 1;
        rustix::fs::renameat_with(
            source_parent,
            source_name,
            destination_parent,
            destination_name,
            RenameFlags::NOREPLACE,
        )
    }

    fn sync_parent(
        &mut self,
        side: RenameSyncSide,
        attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.sync_calls.push((side, attempt));
        rustix::fs::fsync(parent)
    }
}

fn assert_final_source_replacement(replacement: &'static str) {
    let (temporary, tool) = nested_fixture(replacement);
    let outside = TempDirectory::new(&format!("outside-{replacement}"));
    let outside_referent = outside.path().join("referent");
    fs::write(&outside_referent, b"outside referent").unwrap();
    fs::write(
        temporary.path().join("old-parent/replacement-file"),
        b"replacement",
    )
    .unwrap();
    symlink(
        &outside_referent,
        temporary.path().join("old-parent/replacement-link"),
    )
    .unwrap();
    create_fifo(&temporary.path().join("old-parent/replacement-fifo"));
    fs::create_dir(temporary.path().join("old-parent/replacement-directory")).unwrap();
    fs::write(
        temporary
            .path()
            .join("old-parent/replacement-directory/child"),
        b"child",
    )
    .unwrap();
    let mut evidence = FinalWindowEvidence {
        race: FinalWindowRace::ReplaceSource(replacement),
        rename_calls: 0,
        sync_calls: Vec::new(),
        retained_source_was_unlinked: false,
    };
    let error = tool
        .execute_supported_with_evidence(
            "old-parent/source",
            "new-parent/destination",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err();
    assert_commit_ambiguous(&error);
    assert_eq!(evidence.rename_calls, 1);
    assert_eq!(
        evidence.sync_calls,
        [
            (RenameSyncSide::Source, 0),
            (RenameSyncSide::Destination, 0)
        ]
    );
    assert!(temporary.path().join("old-parent/displaced").is_file());
    assert!(!temporary.path().join("old-parent/source").exists());
    let destination = temporary.path().join("new-parent/destination");
    match replacement {
        "replacement-file" => assert_eq!(fs::read(destination).unwrap(), b"replacement"),
        "replacement-link" => {
            assert!(
                fs::symlink_metadata(&destination)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(fs::read(&outside_referent).unwrap(), b"outside referent");
        }
        "replacement-fifo" => {
            assert!(
                fs::symlink_metadata(&destination)
                    .unwrap()
                    .file_type()
                    .is_fifo()
            );
            assert_eq!(fs::read(&outside_referent).unwrap(), b"outside referent");
        }
        "replacement-directory" => {
            assert_eq!(fs::read(destination.join("child")).unwrap(), b"child");
        }
        _ => unreachable!(),
    }
}

#[test]
fn retained_source_descriptor_prevents_inode_reuse_false_success() {
    let (temporary, tool) = nested_fixture("retained-source");
    fs::write(
        temporary.path().join("old-parent/replacement-file"),
        b"replacement",
    )
    .unwrap();
    let mut evidence = FinalWindowEvidence {
        race: FinalWindowRace::UnlinkSourceThenReplace,
        rename_calls: 0,
        sync_calls: Vec::new(),
        retained_source_was_unlinked: false,
    };

    let error = tool
        .execute_supported_with_evidence(
            "old-parent/source",
            "new-parent/destination",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err();

    assert_commit_ambiguous(&error);
    assert_eq!(evidence.rename_calls, 1);
    assert!(evidence.retained_source_was_unlinked);
    assert_eq!(
        fs::read(temporary.path().join("new-parent/destination")).unwrap(),
        b"replacement"
    );
}

#[test]
fn final_window_destination_is_preserved_and_source_replacements_are_disclosed() {
    let (temporary, tool) = nested_fixture("destination-final-race");
    fs::write(temporary.path().join("new-parent/intruder"), b"intruder").unwrap();
    let mut evidence = FinalWindowEvidence {
        race: FinalWindowRace::CreateDestination,
        rename_calls: 0,
        sync_calls: Vec::new(),
        retained_source_was_unlinked: false,
    };
    let error = tool
        .execute_supported_with_evidence(
            "old-parent/source",
            "new-parent/destination",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err();
    assert_eq!(error.code, "rename_file_destination_exists");
    assert_eq!(evidence.rename_calls, 1);
    assert!(evidence.sync_calls.is_empty());
    assert_eq!(
        fs::read(temporary.path().join("new-parent/destination")).unwrap(),
        b"intruder"
    );
    assert_eq!(
        fs::read(temporary.path().join("old-parent/source")).unwrap(),
        b"source"
    );

    for replacement in [
        "replacement-file",
        "replacement-link",
        "replacement-fifo",
        "replacement-directory",
    ] {
        assert_final_source_replacement(replacement);
    }
}

struct MovedParentsEvidence {
    root: PathBuf,
}

#[derive(Clone, Copy)]
enum ParentReplacementSide {
    Source,
    Destination,
}

struct ParentReplacementEvidence {
    root: PathBuf,
    side: ParentReplacementSide,
    rename_calls: usize,
}

struct LinkedRootRemovalEvidence {
    root: PathBuf,
    moved: PathBuf,
    rename_calls: usize,
}

impl RenameFileEvidence for LinkedRootRemovalEvidence {
    #[allow(clippy::too_many_arguments)]
    fn after_validation(
        &mut self,
        phase: RenamePhase,
        _source_parent: BorrowedFd<'_>,
        _source_name: &str,
        _destination_parent: BorrowedFd<'_>,
        _destination_name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        if phase != RenamePhase::Initial {
            return Ok(());
        }
        fs::rename(&self.root, &self.moved).unwrap();
        fs::remove_file(self.moved.join("old-parent/source")).unwrap();
        fs::remove_dir(self.moved.join("old-parent")).unwrap();
        fs::remove_dir(self.moved.join("new-parent")).unwrap();
        fs::remove_dir(&self.moved).unwrap();
        fs::create_dir(&self.root).unwrap();
        fs::create_dir(self.root.join("old-parent")).unwrap();
        fs::create_dir(self.root.join("new-parent")).unwrap();
        fs::write(self.root.join("old-parent/source"), b"replacement root").unwrap();
        Ok(())
    }

    fn rename(
        &mut self,
        source_parent: BorrowedFd<'_>,
        source_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
    ) -> Result<(), rustix::io::Errno> {
        self.rename_calls += 1;
        rustix::fs::renameat_with(
            source_parent,
            source_name,
            destination_parent,
            destination_name,
            RenameFlags::NOREPLACE,
        )
    }
}

impl RenameFileEvidence for ParentReplacementEvidence {
    #[allow(clippy::too_many_arguments)]
    fn after_validation(
        &mut self,
        phase: RenamePhase,
        _source_parent: BorrowedFd<'_>,
        _source_name: &str,
        _destination_parent: BorrowedFd<'_>,
        _destination_name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        if phase != RenamePhase::Initial {
            return Ok(());
        }
        match self.side {
            ParentReplacementSide::Source => {
                fs::rename(
                    self.root.join("old-parent"),
                    self.root.join("initial-old-parent"),
                )
                .unwrap();
                fs::create_dir(self.root.join("old-parent")).unwrap();
                fs::write(self.root.join("old-parent/source"), b"source decoy").unwrap();
            }
            ParentReplacementSide::Destination => {
                fs::rename(
                    self.root.join("new-parent"),
                    self.root.join("initial-new-parent"),
                )
                .unwrap();
                fs::create_dir(self.root.join("new-parent")).unwrap();
            }
        }
        Ok(())
    }

    fn rename(
        &mut self,
        source_parent: BorrowedFd<'_>,
        source_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
    ) -> Result<(), rustix::io::Errno> {
        self.rename_calls += 1;
        rustix::fs::renameat_with(
            source_parent,
            source_name,
            destination_parent,
            destination_name,
            RenameFlags::NOREPLACE,
        )
    }
}

#[test]
fn parent_identity_replacements_between_passes_fail_before_rename() {
    for side in [
        ParentReplacementSide::Source,
        ParentReplacementSide::Destination,
    ] {
        let (temporary, tool) = nested_fixture("parent-identity-replacement");
        let mut evidence = ParentReplacementEvidence {
            root: temporary.path().to_owned(),
            side,
            rename_calls: 0,
        };
        let error = tool
            .execute_supported_with_evidence(
                "old-parent/source",
                "new-parent/destination",
                &CancellationToken::new(),
                &mut evidence,
            )
            .unwrap_err();
        assert_eq!(error.code, "rename_file_target_changed");
        assert_eq!(evidence.rename_calls, 0);
        assert!(!temporary.path().join("new-parent/destination").exists());
        match side {
            ParentReplacementSide::Source => {
                assert_eq!(
                    fs::read(temporary.path().join("old-parent/source")).unwrap(),
                    b"source decoy"
                );
                assert_eq!(
                    fs::read(temporary.path().join("initial-old-parent/source")).unwrap(),
                    b"source"
                );
            }
            ParentReplacementSide::Destination => {
                assert_eq!(
                    fs::read(temporary.path().join("old-parent/source")).unwrap(),
                    b"source"
                );
                assert!(temporary.path().join("initial-new-parent").is_dir());
            }
        }
    }
}

#[test]
fn linked_root_removal_between_passes_cannot_redirect_final_validation() {
    let (temporary, tool) = nested_fixture("linked-root-removal");
    let root = temporary.path().to_owned();
    let moved = root.with_file_name(format!(
        "{}-moved",
        root.file_name().unwrap().to_string_lossy()
    ));
    let mut evidence = LinkedRootRemovalEvidence {
        root: root.clone(),
        moved,
        rename_calls: 0,
    };
    let error = tool
        .execute_supported_with_evidence(
            "old-parent/source",
            "new-parent/destination",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err();
    assert_eq!(error.code, "rename_file_target_changed");
    assert_eq!(evidence.rename_calls, 0);
    assert_eq!(
        fs::read(root.join("old-parent/source")).unwrap(),
        b"replacement root"
    );
    assert!(!root.join("new-parent/destination").exists());
}

impl RenameFileEvidence for MovedParentsEvidence {
    fn final_pre_rename(
        &mut self,
        _source_parent: BorrowedFd<'_>,
        _source_name: &str,
        _destination_parent: BorrowedFd<'_>,
        _destination_name: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        fs::rename(self.root.join("old-parent"), self.root.join("moved-old")).unwrap();
        fs::rename(self.root.join("new-parent"), self.root.join("moved-new")).unwrap();
        fs::create_dir(self.root.join("old-parent")).unwrap();
        fs::create_dir(self.root.join("new-parent")).unwrap();
        fs::write(self.root.join("old-parent/source"), b"public decoy").unwrap();
        fs::write(
            self.root.join("new-parent/destination"),
            b"destination decoy",
        )
        .unwrap();
        Ok(())
    }
}

#[test]
fn final_window_moved_retained_parents_receive_only_the_validated_rename() {
    let (temporary, tool) = nested_fixture("moved-retained-parents");
    let mut evidence = MovedParentsEvidence {
        root: temporary.path().to_owned(),
    };
    let output = tool
        .execute_supported_with_evidence(
            "old-parent/source",
            "new-parent/destination",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap();
    assert_eq!(
        output,
        ToolOutput::success(json!({
            "old_path": "old-parent/source",
            "new_path": "new-parent/destination"
        }))
    );
    assert_eq!(
        fs::read(temporary.path().join("old-parent/source")).unwrap(),
        b"public decoy"
    );
    assert_eq!(
        fs::read(temporary.path().join("new-parent/destination")).unwrap(),
        b"destination decoy"
    );
    assert!(!temporary.path().join("moved-old/source").exists());
    assert_eq!(
        fs::read(temporary.path().join("moved-new/destination")).unwrap(),
        b"source"
    );
}
