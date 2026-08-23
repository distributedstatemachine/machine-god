#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
        rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
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
