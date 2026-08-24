use std::cell::{Cell, RefCell};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fd::AsFd;

use super::*;

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-copy-private-{label}-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create private copy test directory: {error}"),
            }
        }
        panic!("allocate private copy test directory");
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
enum PublishScript {
    Native,
    Interrupted,
    Failed(rustix::io::Errno),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AfterPublishMutation {
    None,
    Destination,
    Source,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckpointMutation {
    None,
    SourceContent,
    StageContent,
    StageNameReplacement,
    DestinationAppearance,
    MoveDestinationParent,
}

struct ScriptedEvidence {
    workspace: PathBuf,
    checkpoints: Vec<CopyCheckpoint>,
    cancel_at: Option<CopyCheckpoint>,
    cancellation: Option<CancellationToken>,
    mutate_at: Option<CopyCheckpoint>,
    checkpoint_mutation: CheckpointMutation,
    publish_script: PublishScript,
    after_publish_mutation: AfterPublishMutation,
    publish_calls: usize,
    stage_open_calls: usize,
    collide_all_stages: bool,
    staged_sync_script: Vec<Result<(), rustix::io::Errno>>,
    parent_sync_script: Vec<Result<(), rustix::io::Errno>>,
    staged_sync_calls: usize,
    parent_sync_calls: usize,
}

impl ScriptedEvidence {
    fn new(workspace: &Path) -> Self {
        Self {
            workspace: workspace.to_owned(),
            checkpoints: Vec::new(),
            cancel_at: None,
            cancellation: None,
            mutate_at: None,
            checkpoint_mutation: CheckpointMutation::None,
            publish_script: PublishScript::Native,
            after_publish_mutation: AfterPublishMutation::None,
            publish_calls: 0,
            stage_open_calls: 0,
            collide_all_stages: false,
            staged_sync_script: Vec::new(),
            parent_sync_script: Vec::new(),
            staged_sync_calls: 0,
            parent_sync_calls: 0,
        }
    }

    fn scripted_sync(
        script: &[Result<(), rustix::io::Errno>],
        attempt: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        script
            .get(attempt)
            .copied()
            .unwrap_or_else(|| rustix::fs::fsync(descriptor))
    }
}

impl CopyFileEvidence for ScriptedEvidence {
    fn checkpoint(&mut self, checkpoint: CopyCheckpoint, _cancellation: &CancellationToken) {
        self.checkpoints.push(checkpoint);
        if self.mutate_at == Some(checkpoint) {
            let staged = self.workspace.join(format!("{TEMP_NAME_PREFIX}{:032x}", 0));
            match self.checkpoint_mutation {
                CheckpointMutation::None => {}
                CheckpointMutation::SourceContent => {
                    fs::write(self.workspace.join("source"), b"changed!").unwrap();
                }
                CheckpointMutation::StageContent => {
                    fs::write(staged, b"tampered").unwrap();
                }
                CheckpointMutation::StageNameReplacement => {
                    fs::rename(&staged, self.workspace.join("moved-stage")).unwrap();
                    fs::write(staged, b"intruder").unwrap();
                }
                CheckpointMutation::DestinationAppearance => {
                    fs::write(self.workspace.join("destination"), b"raced-in").unwrap();
                }
                CheckpointMutation::MoveDestinationParent => {
                    fs::rename(
                        self.workspace.join("destination-parent"),
                        self.workspace.join("moved-parent"),
                    )
                    .unwrap();
                    fs::create_dir(self.workspace.join("destination-parent")).unwrap();
                }
            }
        }
        if self.cancel_at == Some(checkpoint) {
            let _ = self
                .cancellation
                .as_ref()
                .expect("checkpoint cancellation token")
                .cancel();
        }
    }

    fn next_temp_name(
        &mut self,
        attempt: usize,
        _cancellation: &CancellationToken,
    ) -> Result<String, ToolError> {
        Ok(format!("{TEMP_NAME_PREFIX}{attempt:032x}"))
    }

    fn open_stage(
        &mut self,
        parent: BorrowedFd<'_>,
        name: &str,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        self.stage_open_calls += 1;
        if self.collide_all_stages {
            return Err(rustix::io::Errno::EXIST);
        }
        rustix::fs::openat(
            parent,
            name,
            OFlags::RDWR
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        )
    }

    fn publish(
        &mut self,
        staged_parent: BorrowedFd<'_>,
        staged_name: &str,
        destination_parent: BorrowedFd<'_>,
        destination_name: &str,
    ) -> Result<(), rustix::io::Errno> {
        self.publish_calls += 1;
        match self.publish_script {
            PublishScript::Native => rustix::fs::renameat_with(
                staged_parent,
                staged_name,
                destination_parent,
                destination_name,
                RenameFlags::NOREPLACE,
            ),
            PublishScript::Interrupted => Err(rustix::io::Errno::INTR),
            PublishScript::Failed(error) => Err(error),
        }
    }

    fn after_publish(
        &mut self,
        outcome: Result<(), rustix::io::Errno>,
        _cancellation: &CancellationToken,
    ) {
        if outcome.is_err() {
            return;
        }
        match self.after_publish_mutation {
            AfterPublishMutation::None => {}
            AfterPublishMutation::Destination => {
                fs::write(self.workspace.join("destination"), b"tampered").unwrap();
            }
            AfterPublishMutation::Source => {
                fs::write(self.workspace.join("source"), b"mutated!").unwrap();
            }
        }
    }

    fn sync(
        &mut self,
        site: CopySyncSite,
        attempt: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        match site {
            CopySyncSite::Staged => {
                self.staged_sync_calls += 1;
                Self::scripted_sync(&self.staged_sync_script, attempt, descriptor)
            }
            CopySyncSite::DestinationParent => {
                self.parent_sync_calls += 1;
                Self::scripted_sync(&self.parent_sync_script, attempt, descriptor)
            }
        }
    }
}

fn fixture(label: &str) -> (TempDirectory, CopyFileTool) {
    let temporary = TempDirectory::new(label);
    fs::write(temporary.path().join("source"), b"original").unwrap();
    let tool = CopyFileTool::open(temporary.path()).unwrap();
    (temporary, tool)
}

fn execute_with(
    tool: &CopyFileTool,
    cancellation: &CancellationToken,
    evidence: &mut ScriptedEvidence,
) -> Result<ToolOutput, ToolError> {
    execute_paths_with(tool, "source", "destination", cancellation, evidence)
}

fn execute_paths_with(
    tool: &CopyFileTool,
    source: &str,
    destination: &str,
    cancellation: &CancellationToken,
    evidence: &mut ScriptedEvidence,
) -> Result<ToolOutput, ToolError> {
    tool.execute_supported_with_evidence(
        source,
        destination,
        cancellation,
        evidence,
        &NativeCopyFileCleanupEvidence,
    )
}

fn assert_error(error: ToolError, code: &str, retryable: bool) {
    assert_eq!(error.code, code);
    assert_eq!(error.retryable, retryable);
    drop(error);
}

#[test]
fn streaming_retries_partial_io_and_reuses_the_supplied_buffer() {
    let source = b"abcdefgh";
    let mut read_offset = 0_usize;
    let mut staged = Vec::new();
    let mut interrupted_read = false;
    let mut interrupted_write = false;
    let mut buffer = [0_u8; 3];
    let digest = stream_source_to_stage_with(
        source.len(),
        &CancellationToken::new(),
        &mut buffer,
        |chunk| {
            if !interrupted_read {
                interrupted_read = true;
                return Err(rustix::io::Errno::INTR);
            }
            if read_offset == source.len() {
                return Ok(0);
            }
            let progress = chunk.len().min(2).min(source.len() - read_offset);
            chunk[..progress].copy_from_slice(&source[read_offset..read_offset + progress]);
            read_offset += progress;
            Ok(progress)
        },
        |chunk| {
            if !interrupted_write {
                interrupted_write = true;
                return Err(rustix::io::Errno::INTR);
            }
            let progress = chunk.len().min(1);
            staged.extend_from_slice(&chunk[..progress]);
            Ok(progress)
        },
    )
    .unwrap();

    assert_eq!(staged, source);
    assert_eq!(digest, Sha256::digest(source).as_slice());
    assert!(interrupted_read);
    assert!(interrupted_write);
}

#[test]
fn streaming_fails_closed_on_zero_overreported_growth_and_call_caps() {
    let cancellation = CancellationToken::new();
    let mut buffer = [0_u8; 1];
    let error = stream_source_to_stage_with(1, &cancellation, &mut buffer, |_| Ok(0), |_| Ok(1))
        .unwrap_err();
    assert_error(error, "copy_file_target_changed", true);

    let error = stream_source_to_stage_with(
        1,
        &cancellation,
        &mut buffer,
        |chunk| Ok(chunk.len() + 1),
        |_| Ok(1),
    )
    .unwrap_err();
    assert_error(error, "copy_file_copy_failed", true);

    let reads = Cell::new(0_usize);
    let error = stream_source_to_stage_with(
        0,
        &cancellation,
        &mut buffer,
        |_| {
            reads.set(reads.get() + 1);
            Ok(1)
        },
        |_| Ok(1),
    )
    .unwrap_err();
    assert_error(error, "copy_file_target_changed", true);
    assert_eq!(reads.get(), 1);

    let reads = Cell::new(0_usize);
    let error = stream_source_to_stage_with(
        MAX_COPY_FILE_IO_CALLS,
        &cancellation,
        &mut buffer,
        |chunk| {
            reads.set(reads.get() + 1);
            chunk[0] = b'x';
            Ok(1)
        },
        |_| Ok(1),
    )
    .unwrap_err();
    assert_error(error, "copy_file_copy_failed", true);
    assert_eq!(reads.get(), MAX_COPY_FILE_IO_CALLS);
}

#[test]
fn streaming_and_hashing_bound_cumulative_interruptions() {
    let cancellation = CancellationToken::new();
    let mut buffer = [0_u8; 1];
    let reads = Cell::new(0_usize);
    let error = stream_source_to_stage_with(
        1,
        &cancellation,
        &mut buffer,
        |_| {
            reads.set(reads.get() + 1);
            Err(rustix::io::Errno::INTR)
        },
        |_| Ok(1),
    )
    .unwrap_err();
    assert_error(error, "copy_file_copy_failed", true);
    assert_eq!(reads.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);

    let reads = Cell::new(0_usize);
    let error = hash_file_with(1, None, &mut buffer, |_| {
        reads.set(reads.get() + 1);
        Err(rustix::io::Errno::INTR)
    })
    .unwrap_err();
    assert_error(error, "copy_file_copy_failed", true);
    assert_eq!(reads.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
}

#[test]
fn entropy_accepts_partial_progress_and_enforces_every_bound() {
    let cancellation = CancellationToken::new();
    let mut bytes = [0_u8; TEMP_RANDOM_BYTES];
    let calls = Cell::new(0_usize);
    fill_random_with(&mut bytes, &cancellation, |remaining| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            return Err(rustix::io::Errno::INTR);
        }
        remaining[0] = u8::try_from(calls.get()).unwrap();
        Ok(1)
    })
    .unwrap();
    assert_eq!(calls.get(), TEMP_RANDOM_BYTES + 1);
    assert!(bytes.iter().all(|byte| *byte != 0));

    let calls = Cell::new(0_usize);
    let error = fill_random_with(&mut bytes, &cancellation, |_| {
        calls.set(calls.get() + 1);
        Err(rustix::io::Errno::INTR)
    })
    .unwrap_err();
    assert_error(error, "copy_file_unavailable", true);
    assert_eq!(calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);

    assert_error(
        fill_random_with(&mut bytes, &cancellation, |_| Ok(0)).unwrap_err(),
        "copy_file_unavailable",
        true,
    );
    let error = fill_random_with(&mut bytes, &cancellation, |remaining| {
        Ok(remaining.len() + 1)
    })
    .unwrap_err();
    assert_error(error, "copy_file_unavailable", true);
}

#[test]
fn precommit_and_postcommit_sync_have_exact_sixteen_call_caps() {
    let cancellation = CancellationToken::new();
    let pre_calls = Cell::new(0_usize);
    let error = sync_precommit_with(&cancellation, |_| {
        pre_calls.set(pre_calls.get() + 1);
        Err(rustix::io::Errno::INTR)
    })
    .unwrap_err();
    assert_error(error, "copy_file_copy_failed", true);
    assert_eq!(pre_calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);

    let post_calls = Cell::new(0_usize);
    assert_eq!(
        sync_postcommit_with(|_| {
            post_calls.set(post_calls.get() + 1);
            Err(rustix::io::Errno::INTR)
        }),
        Err(rustix::io::Errno::INTR)
    );
    assert_eq!(post_calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
}

#[test]
fn full_execution_publishes_exactly_once_and_reuses_one_stage_name() {
    let (temporary, tool) = fixture("success");
    let mut evidence = ScriptedEvidence::new(temporary.path());
    let output = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap();

    assert_eq!(evidence.publish_calls, 1);
    assert_eq!(evidence.stage_open_calls, 1);
    assert_eq!(evidence.staged_sync_calls, 1);
    assert_eq!(evidence.parent_sync_calls, 1);
    assert_eq!(
        fs::read(temporary.path().join("source")).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read(temporary.path().join("destination")).unwrap(),
        b"original"
    );
    assert_eq!(
        output.content,
        json!({"source":"source","destination":"destination","bytes_copied":8})
    );
}

#[test]
fn publication_interruption_is_not_retried_and_cleans_the_stage() {
    let (temporary, tool) = fixture("publish-intr");
    let mut evidence = ScriptedEvidence::new(temporary.path());
    evidence.publish_script = PublishScript::Interrupted;
    let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();

    assert_error(error, "copy_file_commit_ambiguous", false);
    assert_eq!(evidence.publish_calls, 1);
    assert_eq!(evidence.parent_sync_calls, 1);
    assert!(!temporary.path().join("destination").exists());
    assert_eq!(
        fs::read_dir(temporary.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(TEMP_NAME_PREFIX))
            .count(),
        0
    );
}

#[test]
fn definitive_publication_errors_are_mapped_without_retry() {
    for (errno, code, retryable) in [
        (
            rustix::io::Errno::EXIST,
            "copy_file_destination_exists",
            false,
        ),
        (
            rustix::io::Errno::XDEV,
            "copy_file_unsupported_filesystem",
            false,
        ),
        (rustix::io::Errno::IO, "copy_file_copy_failed", true),
    ] {
        let (temporary, tool) = fixture("publish-errors");
        let mut evidence = ScriptedEvidence::new(temporary.path());
        evidence.publish_script = PublishScript::Failed(errno);
        let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
        assert_error(error, code, retryable);
        assert_eq!(evidence.publish_calls, 1);
        assert!(!temporary.path().join("destination").exists());
    }
}

#[test]
fn postcommit_corruption_and_source_change_are_ambiguous_and_still_sync() {
    for mutation in [
        AfterPublishMutation::Destination,
        AfterPublishMutation::Source,
    ] {
        let (temporary, tool) = fixture("postcommit-change");
        let mut evidence = ScriptedEvidence::new(temporary.path());
        evidence.after_publish_mutation = mutation;
        let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
        assert_error(error, "copy_file_commit_ambiguous", false);
        assert_eq!(evidence.publish_calls, 1);
        assert_eq!(evidence.parent_sync_calls, 1);
        assert!(temporary.path().join("destination").exists());
    }
}

#[test]
fn source_and_stage_content_changes_fail_before_publication() {
    for (mutation, code) in [
        (
            CheckpointMutation::SourceContent,
            "copy_file_target_changed",
        ),
        (CheckpointMutation::StageContent, "copy_file_copy_failed"),
    ] {
        let (temporary, tool) = fixture("precommit-content-change");
        let mut evidence = ScriptedEvidence::new(temporary.path());
        evidence.mutate_at = Some(CopyCheckpoint::AfterCopy);
        evidence.checkpoint_mutation = mutation;
        let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
        assert_error(error, code, true);
        assert_eq!(evidence.publish_calls, 0);
        assert!(!temporary.path().join("destination").exists());
    }
}

#[test]
fn staged_name_replacement_is_rejected_without_deleting_the_intruder() {
    let (temporary, tool) = fixture("stage-replacement");
    let mut evidence = ScriptedEvidence::new(temporary.path());
    evidence.mutate_at = Some(CopyCheckpoint::AfterFinalDestinationValidation);
    evidence.checkpoint_mutation = CheckpointMutation::StageNameReplacement;
    let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();

    assert_error(error, "copy_file_target_changed", true);
    assert_eq!(evidence.publish_calls, 0);
    assert_eq!(
        fs::read(
            temporary
                .path()
                .join(format!("{TEMP_NAME_PREFIX}{:032x}", 0))
        )
        .unwrap(),
        b"intruder"
    );
    assert!(temporary.path().join("moved-stage").exists());
}

#[test]
fn destination_appearance_in_the_final_window_is_preserved() {
    let (temporary, tool) = fixture("destination-race");
    let mut evidence = ScriptedEvidence::new(temporary.path());
    evidence.mutate_at = Some(CopyCheckpoint::AfterFinalStageValidation);
    evidence.checkpoint_mutation = CheckpointMutation::DestinationAppearance;
    let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();

    assert_error(error, "copy_file_destination_exists", false);
    assert_eq!(evidence.publish_calls, 1);
    assert_eq!(
        fs::read(temporary.path().join("destination")).unwrap(),
        b"raced-in"
    );
}

#[test]
fn destination_parent_moved_after_final_rewalk_receives_the_copy() {
    let temporary = TempDirectory::new("moved-destination-parent");
    fs::write(temporary.path().join("source"), b"original").unwrap();
    fs::create_dir(temporary.path().join("destination-parent")).unwrap();
    let tool = CopyFileTool::open(temporary.path()).unwrap();
    let mut evidence = ScriptedEvidence::new(temporary.path());
    evidence.mutate_at = Some(CopyCheckpoint::AfterFinalStageValidation);
    evidence.checkpoint_mutation = CheckpointMutation::MoveDestinationParent;

    let output = execute_paths_with(
        &tool,
        "source",
        "destination-parent/destination",
        &CancellationToken::new(),
        &mut evidence,
    )
    .unwrap();

    assert_eq!(evidence.publish_calls, 1);
    assert_eq!(evidence.parent_sync_calls, 1);
    assert!(
        !temporary
            .path()
            .join("destination-parent/destination")
            .exists()
    );
    assert_eq!(
        fs::read(temporary.path().join("moved-parent/destination")).unwrap(),
        b"original"
    );
    assert_eq!(
        output.content,
        json!({
            "source":"source",
            "destination":"destination-parent/destination",
            "bytes_copied":8
        })
    );
}

#[test]
fn staged_and_parent_sync_exhaustion_stay_on_the_correct_commit_side() {
    let interrupted = vec![Err(rustix::io::Errno::INTR); MAX_INTERRUPTED_SYSCALL_ATTEMPTS];

    let (temporary, tool) = fixture("staged-sync-exhaustion");
    let mut evidence = ScriptedEvidence::new(temporary.path());
    evidence.staged_sync_script = interrupted.clone();
    let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
    assert_error(error, "copy_file_copy_failed", true);
    assert_eq!(evidence.staged_sync_calls, MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
    assert_eq!(evidence.publish_calls, 0);
    assert!(!temporary.path().join("destination").exists());

    let (temporary, tool) = fixture("parent-sync-exhaustion");
    let mut evidence = ScriptedEvidence::new(temporary.path());
    evidence.parent_sync_script = interrupted;
    let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
    assert_error(error, "copy_file_commit_ambiguous", false);
    assert_eq!(evidence.publish_calls, 1);
    assert_eq!(evidence.parent_sync_calls, MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
    assert!(temporary.path().join("destination").exists());
}

#[test]
fn same_call_cancellation_wins_definitive_publish_failure_but_not_interruption() {
    for (script, code) in [
        (
            PublishScript::Failed(rustix::io::Errno::IO),
            "copy_file_cancelled",
        ),
        (PublishScript::Interrupted, "copy_file_commit_ambiguous"),
    ] {
        let (temporary, tool) = fixture("publish-cancellation");
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(temporary.path());
        evidence.publish_script = script;
        evidence.cancel_at = Some(CopyCheckpoint::AfterPublish);
        evidence.cancellation = Some(cancellation.clone());
        let error = execute_with(&tool, &cancellation, &mut evidence).unwrap_err();
        assert_error(error, code, false);
        assert_eq!(evidence.publish_calls, 1);
    }
}

#[test]
fn every_precommit_checkpoint_honors_cancellation_without_publication() {
    const PRECOMMIT: [CopyCheckpoint; 14] = [
        CopyCheckpoint::AfterInitialSourceParent,
        CopyCheckpoint::AfterSourceRetained,
        CopyCheckpoint::AfterInitialDestinationParent,
        CopyCheckpoint::AfterDestinationAbsent,
        CopyCheckpoint::AfterStageCreated,
        CopyCheckpoint::AfterCopy,
        CopyCheckpoint::AfterInitialStageVerification,
        CopyCheckpoint::AfterFinalStageVerification,
        CopyCheckpoint::AfterFinalSourceParent,
        CopyCheckpoint::AfterFinalSourceValidation,
        CopyCheckpoint::AfterFinalDestinationParent,
        CopyCheckpoint::AfterFinalDestinationValidation,
        CopyCheckpoint::AfterFinalStageValidation,
        CopyCheckpoint::FinalPrePublish,
    ];
    for checkpoint in PRECOMMIT {
        let (temporary, tool) = fixture("checkpoint-cancel");
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(temporary.path());
        evidence.cancel_at = Some(checkpoint);
        evidence.cancellation = Some(cancellation.clone());
        let error = execute_with(&tool, &cancellation, &mut evidence).unwrap_err();
        assert_error(error, "copy_file_cancelled", false);
        assert_eq!(evidence.publish_calls, 0);
        assert!(!temporary.path().join("destination").exists());
    }
}

#[test]
fn cancellation_after_publication_is_ignored_through_verification_and_sync() {
    let (temporary, tool) = fixture("postcommit-cancel");
    let cancellation = CancellationToken::new();
    let mut evidence = ScriptedEvidence::new(temporary.path());
    evidence.cancel_at = Some(CopyCheckpoint::AfterPublish);
    evidence.cancellation = Some(cancellation.clone());

    let output = execute_with(&tool, &cancellation, &mut evidence).unwrap();
    assert!(cancellation.is_cancelled());
    assert_eq!(evidence.publish_calls, 1);
    assert_eq!(evidence.parent_sync_calls, 1);
    assert_eq!(output.content["bytes_copied"], 8);
}

#[test]
fn stage_name_collisions_are_bounded_and_never_removed() {
    let (temporary, tool) = fixture("stage-collisions");
    let mut evidence = ScriptedEvidence::new(temporary.path());
    evidence.collide_all_stages = true;
    let error = execute_with(&tool, &CancellationToken::new(), &mut evidence).unwrap_err();
    assert_error(error, "copy_file_unavailable", true);
    assert_eq!(evidence.stage_open_calls, MAX_COPY_FILE_TEMP_ATTEMPTS);
    assert_eq!(evidence.publish_calls, 0);
}

struct FailingCleanup {
    calls: RefCell<Vec<&'static str>>,
}

impl CopyFileCleanupEvidence for FailingCleanup {
    fn set_mode(&self, _file: BorrowedFd<'_>, _mode: Mode) -> Result<(), rustix::io::Errno> {
        self.calls.borrow_mut().push("set_mode");
        Err(rustix::io::Errno::IO)
    }

    fn fstat(&self, _file: BorrowedFd<'_>) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        self.calls.borrow_mut().push("fstat");
        Err(rustix::io::Errno::IO)
    }
}

#[test]
fn cleanup_dual_failure_is_bounded_and_leaves_unowned_state_untouched() {
    let temporary = TempDirectory::new("cleanup-failure");
    let parent = rustix::fs::open(temporary.path(), directory_open_flags(), Mode::empty()).unwrap();
    let file = rustix::fs::openat(
        parent.as_fd(),
        "stage",
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .unwrap();
    let metadata = rustix::fs::fstat(&file).unwrap();
    let evidence = FailingCleanup {
        calls: RefCell::new(Vec::new()),
    };

    cleanup_unpublished_stage(
        parent.as_fd(),
        file.as_fd(),
        "stage",
        FileIdentity::from_stat(&metadata),
        &evidence,
    );

    assert_eq!(&*evidence.calls.borrow(), &["set_mode", "fstat"]);
    assert!(temporary.path().join("stage").exists());
}

#[test]
fn cleanup_identity_check_does_not_delete_a_replacement() {
    let temporary = TempDirectory::new("cleanup-identity");
    let parent = rustix::fs::open(temporary.path(), directory_open_flags(), Mode::empty()).unwrap();
    let held = rustix::fs::openat(
        parent.as_fd(),
        "stage",
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o700),
    )
    .unwrap();
    let identity = FileIdentity::from_stat(&rustix::fs::fstat(&held).unwrap());
    fs::rename(
        temporary.path().join("stage"),
        temporary.path().join("moved-stage"),
    )
    .unwrap();
    fs::write(temporary.path().join("stage"), b"replacement").unwrap();

    cleanup_unpublished_stage(
        parent.as_fd(),
        held.as_fd(),
        "stage",
        identity,
        &NativeCopyFileCleanupEvidence,
    );

    assert_eq!(
        fs::read(temporary.path().join("stage")).unwrap(),
        b"replacement"
    );
    assert!(temporary.path().join("moved-stage").exists());
}
