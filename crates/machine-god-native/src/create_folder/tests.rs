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
                "machine-god-create-folder-private-{label}-{}-{suffix}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create private create_folder directory: {error}"),
            }
        }
        panic!("allocate private create_folder directory")
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum Operation {
    Open(CreateFolderPhase, CreateFolderOpenSite, usize, String),
    Fstat(CreateFolderPhase, CreateFolderFstatSite, usize),
    #[cfg(target_os = "macos")]
    Statat(CreateFolderPhase, CreateFolderStatatSite, usize),
    Mkdir(usize, usize, String, u32),
    Sync(CreateFolderSyncSite, usize),
}

#[derive(Clone, Copy)]
enum MkdirAction {
    Native,
    Return(rustix::io::Errno),
    CreateThenReturn(rustix::io::Errno),
    FileThenExist,
}

struct TraceEvidence {
    operations: Vec<Operation>,
    checkpoints: Vec<CreateFolderCheckpoint>,
    cancel_at: Option<CreateFolderCheckpoint>,
    checkpoint_action: Option<(CreateFolderCheckpoint, Box<dyn FnMut()>)>,
    mkdir_actions: Vec<MkdirAction>,
    open_fault: Option<(CreateFolderPhase, CreateFolderOpenSite, rustix::io::Errno)>,
    fstat_fault: Option<(CreateFolderPhase, CreateFolderFstatSite, rustix::io::Errno)>,
    sync_errors: Vec<(CreateFolderSyncSite, usize, rustix::io::Errno)>,
}

impl TraceEvidence {
    fn new() -> Self {
        Self {
            operations: Vec::new(),
            checkpoints: Vec::new(),
            cancel_at: None,
            checkpoint_action: None,
            mkdir_actions: Vec::new(),
            open_fault: None,
            fstat_fault: None,
            sync_errors: Vec::new(),
        }
    }

    fn mkdir_operations(&self) -> Vec<&Operation> {
        self.operations
            .iter()
            .filter(|operation| matches!(operation, Operation::Mkdir(..)))
            .collect()
    }

    fn sync_operations(&self) -> Vec<(CreateFolderSyncSite, usize)> {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::Sync(site, attempt) => Some((*site, *attempt)),
                _ => None,
            })
            .collect()
    }
}

impl CreateFolderEvidence for TraceEvidence {
    fn checkpoint(&mut self, checkpoint: CreateFolderCheckpoint, cancellation: &CancellationToken) {
        self.checkpoints.push(checkpoint);
        if self.cancel_at == Some(checkpoint) {
            let _ = cancellation.cancel();
        }
        if let Some((target, action)) = self.checkpoint_action.as_mut()
            && *target == checkpoint
        {
            action();
        }
    }

    fn open_walk(
        &mut self,
        phase: CreateFolderPhase,
        site: CreateFolderOpenSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        self.operations.push(Operation::Open(
            phase,
            site,
            ordinal,
            component.to_string_lossy().into_owned(),
        ));
        if let Some((fault_phase, fault_site, error)) = self.open_fault
            && fault_phase == phase
            && fault_site == site
        {
            return Err(error);
        }
        rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
    }

    fn fstat(
        &mut self,
        phase: CreateFolderPhase,
        site: CreateFolderFstatSite,
        ordinal: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        self.operations.push(Operation::Fstat(phase, site, ordinal));
        if let Some((fault_phase, fault_site, error)) = self.fstat_fault
            && fault_phase == phase
            && fault_site == site
        {
            return Err(error);
        }
        rustix::fs::fstat(descriptor)
    }

    #[cfg(target_os = "macos")]
    fn statat(
        &mut self,
        phase: CreateFolderPhase,
        site: CreateFolderStatatSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        self.operations
            .push(Operation::Statat(phase, site, ordinal));
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn mkdir(
        &mut self,
        ordinal: usize,
        component_index: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
        mode: Mode,
    ) -> Result<(), rustix::io::Errno> {
        self.operations.push(Operation::Mkdir(
            ordinal,
            component_index,
            component.to_string_lossy().into_owned(),
            u32::from(mode.bits()),
        ));
        match self
            .mkdir_actions
            .get(ordinal)
            .copied()
            .unwrap_or(MkdirAction::Native)
        {
            MkdirAction::Native => rustix::fs::mkdirat(parent, component, mode),
            MkdirAction::Return(error) => Err(error),
            MkdirAction::CreateThenReturn(error) => {
                rustix::fs::mkdirat(parent, component, mode)?;
                Err(error)
            }
            MkdirAction::FileThenExist => {
                let file = rustix::fs::openat(
                    parent,
                    component,
                    OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW,
                    Mode::from_raw_mode(0o600),
                )?;
                drop(file);
                Err(rustix::io::Errno::EXIST)
            }
        }
    }

    fn sync_directory(
        &mut self,
        site: CreateFolderSyncSite,
        attempt: usize,
        directory: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.operations.push(Operation::Sync(site, attempt));
        if let Some((_, _, error)) = self
            .sync_errors
            .iter()
            .find(|(error_site, error_attempt, _)| *error_site == site && *error_attempt == attempt)
        {
            return Err(*error);
        }
        rustix::fs::fsync(directory)
    }
}

fn fixture(label: &str) -> (TempDirectory, CreateFolderTool) {
    let temporary = TempDirectory::new(label);
    let tool = CreateFolderTool::open(temporary.path()).unwrap();
    (temporary, tool)
}

fn execute_with<Evidence: CreateFolderEvidence>(
    tool: &CreateFolderTool,
    path: &str,
    cancellation: &CancellationToken,
    evidence: &mut Evidence,
) -> Result<ToolOutput, ToolError> {
    tool.execute_supported_with_evidence(path, cancellation, evidence)
}

fn assert_error(
    error: &ToolError,
    kind: ToolErrorKind,
    code: &str,
    message: &str,
    retryable: bool,
) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.code, code);
    assert_eq!(error.message, message);
    assert_eq!(error.retryable, retryable);
    assert_eq!(error.to_string(), format!("{code}: {message}"));
}

fn assert_cancelled(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::Cancelled,
        "create_folder_cancelled",
        "create_folder execution was cancelled",
        false,
    );
}

fn assert_ambiguous(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::Execution,
        "create_folder_commit_ambiguous",
        "requested folder creation status is uncertain",
        false,
    );
}

#[test]
fn serialized_argument_and_result_guards_accept_exact_and_reject_one_over() {
    let argument_overhead = serde_json::to_vec(&json!({"path": ""})).unwrap().len();
    let exact_arguments = json!({"path": "x".repeat(MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES - argument_overhead)});
    assert_eq!(
        serde_json::to_vec(&exact_arguments).unwrap().len(),
        MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES
    );
    assert!(serialized_value_fits(
        &exact_arguments,
        MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES
    ));
    let over_arguments = json!({"path": "x".repeat(MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES - argument_overhead + 1)});
    assert_eq!(
        serde_json::to_vec(&over_arguments).unwrap().len(),
        MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES + 1
    );
    assert!(!serialized_value_fits(
        &over_arguments,
        MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES
    ));

    let result_overhead = serde_json::to_vec(&ToolOutput::success(json!({"path": ""})))
        .unwrap()
        .len();
    let exact_result_path = "x".repeat(MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES - result_overhead);
    let exact_result = build_success_output(&exact_result_path).unwrap();
    assert_eq!(
        serde_json::to_vec(&exact_result).unwrap().len(),
        MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES
    );
    let over_result_path =
        "x".repeat(MAX_CREATE_FOLDER_SERIALIZED_RESULT_BYTES - result_overhead + 1);
    let error = build_success_output(&over_result_path).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Execution,
        "create_folder_create_failed",
        "requested folder could not be created",
        true,
    );

    assert_eq!(
        build_success_output("parent/folder").unwrap(),
        ToolOutput::success(json!({"path": "parent/folder"}))
    );
}

#[test]
fn real_pipeline_uses_one_mkdir_per_component_requested_mode_and_bottom_up_sync() {
    let (temporary, tool) = fixture("trace");
    fs::create_dir(temporary.path().join("existing")).unwrap();
    let mut evidence = TraceEvidence::new();

    assert_eq!(
        execute_with(
            &tool,
            "existing/parent/final",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap(),
        ToolOutput::success(json!({"path": "existing/parent/final"}))
    );
    assert_eq!(
        evidence.mkdir_operations(),
        vec![
            &Operation::Mkdir(0, 1, "parent".to_owned(), 0o755),
            &Operation::Mkdir(1, 2, "final".to_owned(), 0o755),
        ]
    );
    assert_eq!(
        evidence.sync_operations(),
        vec![
            (CreateFolderSyncSite::CreatedDirectory(2), 0),
            (CreateFolderSyncSite::CreatedDirectory(1), 0),
            (CreateFolderSyncSite::FirstCreatedParent(1), 0),
        ]
    );
    assert_eq!(
        evidence
            .checkpoints
            .iter()
            .filter(|checkpoint| **checkpoint == CreateFolderCheckpoint::AfterCommit)
            .count(),
        1
    );
    assert_eq!(
        evidence
            .operations
            .iter()
            .filter(|operation| {
                matches!(
                    operation,
                    Operation::Open(
                        CreateFolderPhase::Postcommit,
                        CreateFolderOpenSite::Root,
                        _,
                        _
                    )
                )
            })
            .count(),
        1
    );
    for (component_index, expected_opens) in [(0, 1), (1, 2), (2, 2)] {
        assert_eq!(
            evidence
                .operations
                .iter()
                .filter(|operation| {
                    matches!(
                        operation,
                        Operation::Open(
                            CreateFolderPhase::Postcommit,
                            CreateFolderOpenSite::Component(index),
                            _,
                            _
                        ) if *index == component_index
                    )
                })
                .count(),
            expected_opens
        );
    }
    assert!(temporary.path().join("existing/parent/final").is_dir());
}

#[test]
fn every_observed_precommit_checkpoint_cancels_without_a_mkdir_effect() {
    let (_discovery_root, discovery_tool) = fixture("cancel-discovery");
    let mut discovery = TraceEvidence::new();
    execute_with(
        &discovery_tool,
        "parent/final",
        &CancellationToken::new(),
        &mut discovery,
    )
    .unwrap();
    let final_precreate = discovery
        .checkpoints
        .iter()
        .position(|checkpoint| *checkpoint == CreateFolderCheckpoint::FinalPreCreate)
        .unwrap();
    let before_first_mkdir = discovery
        .checkpoints
        .iter()
        .position(|checkpoint| matches!(checkpoint, CreateFolderCheckpoint::BeforeMkdir(0, 0)))
        .unwrap();
    assert!(final_precreate < before_first_mkdir);
    let mut targets = discovery.checkpoints[..=before_first_mkdir].to_vec();
    targets.dedup();

    for (index, target) in targets.into_iter().enumerate() {
        let (temporary, tool) = fixture(&format!("cancel-{index}"));
        let cancellation = CancellationToken::new();
        let mut evidence = TraceEvidence::new();
        evidence.cancel_at = Some(target);

        assert_cancelled(
            &execute_with(&tool, "parent/final", &cancellation, &mut evidence).unwrap_err(),
        );
        assert!(evidence.mkdir_operations().is_empty());
        assert!(!temporary.path().join("parent").exists());
        assert!(evidence.sync_operations().is_empty());
    }
}

#[test]
fn idempotent_existing_directory_uses_final_cancellation_check_without_sync() {
    let (temporary, tool) = fixture("idempotent-private");
    fs::create_dir(temporary.path().join("existing")).unwrap();
    let mut evidence = TraceEvidence::new();
    assert_eq!(
        execute_with(&tool, "existing", &CancellationToken::new(), &mut evidence,).unwrap(),
        ToolOutput::success(json!({"path": "existing"}))
    );
    assert!(evidence.mkdir_operations().is_empty());
    assert!(evidence.sync_operations().is_empty());
    assert!(
        evidence
            .checkpoints
            .contains(&CreateFolderCheckpoint::AfterWalk(
                CreateFolderPhase::Revalidate
            ))
    );

    let cancellation = CancellationToken::new();
    let mut evidence = TraceEvidence::new();
    evidence.cancel_at = Some(CreateFolderCheckpoint::AfterWalk(
        CreateFolderPhase::Revalidate,
    ));
    assert_cancelled(&execute_with(&tool, "existing", &cancellation, &mut evidence).unwrap_err());
    assert!(evidence.mkdir_operations().is_empty());
    assert!(evidence.sync_operations().is_empty());
    assert!(temporary.path().join("existing").is_dir());
}

#[test]
fn definitive_mkdir_error_has_saved_error_cancellation_precedence_before_commit() {
    let (temporary, tool) = fixture("saved-mkdir-cancel");
    let cancellation = CancellationToken::new();
    let mut evidence = TraceEvidence::new();
    evidence.mkdir_actions = vec![MkdirAction::Return(rustix::io::Errno::IO)];
    evidence.cancel_at = Some(CreateFolderCheckpoint::AfterMkdir(0, 0));

    assert_cancelled(&execute_with(&tool, "folder", &cancellation, &mut evidence).unwrap_err());
    assert_eq!(evidence.mkdir_operations().len(), 1);
    assert!(!temporary.path().join("folder").exists());
    assert!(evidence.sync_operations().is_empty());
}

#[test]
fn interrupted_mkdir_is_never_retried_and_both_possible_effects_are_ambiguous() {
    for (label, action, exists, expected_sites) in [
        (
            "intr-no-effect",
            MkdirAction::Return(rustix::io::Errno::INTR),
            false,
            vec![(CreateFolderSyncSite::FirstCreatedParent(0), 0)],
        ),
        (
            "intr-effect",
            MkdirAction::CreateThenReturn(rustix::io::Errno::INTR),
            true,
            vec![
                (CreateFolderSyncSite::CreatedDirectory(0), 0),
                (CreateFolderSyncSite::FirstCreatedParent(0), 0),
            ],
        ),
    ] {
        let (temporary, tool) = fixture(label);
        let mut evidence = TraceEvidence::new();
        evidence.mkdir_actions = vec![action];

        assert_ambiguous(
            &execute_with(
                &tool,
                "folder/never-attempted",
                &CancellationToken::new(),
                &mut evidence,
            )
            .unwrap_err(),
        );
        assert_eq!(evidence.mkdir_operations().len(), 1);
        assert_eq!(temporary.path().join("folder").exists(), exists);
        assert!(!temporary.path().join("folder/never-attempted").exists());
        assert_eq!(evidence.sync_operations(), expected_sites);
    }
}

#[test]
fn raced_eexist_directory_is_validated_and_can_become_the_first_created_parent() {
    let (temporary, tool) = fixture("eexist-directory");
    let mut evidence = TraceEvidence::new();
    evidence.mkdir_actions = vec![
        MkdirAction::CreateThenReturn(rustix::io::Errno::EXIST),
        MkdirAction::Native,
    ];

    assert_eq!(
        execute_with(
            &tool,
            "raced/final",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap(),
        ToolOutput::success(json!({"path": "raced/final"}))
    );
    assert_eq!(evidence.mkdir_operations().len(), 2);
    assert_eq!(
        evidence.sync_operations(),
        vec![
            (CreateFolderSyncSite::CreatedDirectory(1), 0),
            (CreateFolderSyncSite::FirstCreatedParent(1), 0),
        ]
    );
    assert!(temporary.path().join("raced/final").is_dir());
}

#[test]
fn postcommit_failure_ignores_cancellation_keeps_prefix_and_syncs_every_retained_site() {
    let (temporary, tool) = fixture("late-cancel");
    let cancellation = CancellationToken::new();
    let mut evidence = TraceEvidence::new();
    evidence.mkdir_actions = vec![
        MkdirAction::Native,
        MkdirAction::Return(rustix::io::Errno::IO),
    ];
    evidence.cancel_at = Some(CreateFolderCheckpoint::AfterMkdir(1, 1));

    assert_ambiguous(
        &execute_with(&tool, "created/not-created", &cancellation, &mut evidence).unwrap_err(),
    );
    assert!(cancellation.is_cancelled());
    assert_eq!(evidence.mkdir_operations().len(), 2);
    assert!(temporary.path().join("created").is_dir());
    assert!(!temporary.path().join("created/not-created").exists());
    assert_eq!(
        evidence.sync_operations(),
        vec![
            (CreateFolderSyncSite::CreatedDirectory(0), 0),
            (CreateFolderSyncSite::FirstCreatedParent(0), 0),
        ]
    );
}

#[test]
fn cancellation_at_the_first_commit_transition_is_ignored_through_success() {
    let (temporary, tool) = fixture("late-cancel-success");
    let cancellation = CancellationToken::new();
    let mut evidence = TraceEvidence::new();
    evidence.cancel_at = Some(CreateFolderCheckpoint::AfterCommit);

    assert_eq!(
        execute_with(&tool, "folder", &cancellation, &mut evidence).unwrap(),
        ToolOutput::success(json!({"path": "folder"}))
    );
    assert!(cancellation.is_cancelled());
    assert!(temporary.path().join("folder").is_dir());
    assert_eq!(
        evidence.sync_operations(),
        vec![
            (CreateFolderSyncSite::CreatedDirectory(0), 0),
            (CreateFolderSyncSite::FirstCreatedParent(0), 0),
        ]
    );
}

#[test]
fn raced_non_directory_is_definitive_before_commit_and_ambiguous_after_commit() {
    let (temporary, tool) = fixture("precommit-raced-file");
    let mut evidence = TraceEvidence::new();
    evidence.mkdir_actions = vec![MkdirAction::FileThenExist];
    let error =
        execute_with(&tool, "intruder", &CancellationToken::new(), &mut evidence).unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Execution,
        "create_folder_target_exists",
        "requested folder path already exists as a non-directory",
        false,
    );
    assert!(temporary.path().join("intruder").is_file());
    assert!(evidence.sync_operations().is_empty());

    let (temporary, tool) = fixture("postcommit-raced-file");
    let mut evidence = TraceEvidence::new();
    evidence.mkdir_actions = vec![MkdirAction::Native, MkdirAction::FileThenExist];
    assert_ambiguous(
        &execute_with(
            &tool,
            "created/intruder",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err(),
    );
    assert!(temporary.path().join("created").is_dir());
    assert!(temporary.path().join("created/intruder").is_file());
    assert_eq!(
        evidence.sync_operations(),
        vec![
            (CreateFolderSyncSite::CreatedDirectory(0), 0),
            (CreateFolderSyncSite::FirstCreatedParent(0), 0),
        ]
    );
}

#[test]
fn existing_prefix_identity_replacement_during_revalidation_fails_before_mkdir() {
    let (temporary, tool) = fixture("prefix-replacement");
    fs::create_dir(temporary.path().join("existing")).unwrap();
    let root = temporary.path().to_owned();
    let mut evidence = TraceEvidence::new();
    evidence.checkpoint_action = Some((
        CreateFolderCheckpoint::AfterRootValidation(CreateFolderPhase::Revalidate),
        Box::new(move || {
            fs::rename(root.join("existing"), root.join("displaced")).unwrap();
            fs::create_dir(root.join("existing")).unwrap();
        }),
    ));

    let error = execute_with(
        &tool,
        "existing/final",
        &CancellationToken::new(),
        &mut evidence,
    )
    .unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Execution,
        "create_folder_target_changed",
        "requested folder path changed during creation",
        true,
    );
    assert!(evidence.mkdir_operations().is_empty());
    assert!(!temporary.path().join("existing/final").exists());
    assert!(!temporary.path().join("displaced/final").exists());
    assert!(evidence.sync_operations().is_empty());
}

#[test]
fn moved_retained_parent_with_public_replacement_is_ambiguous_and_not_rolled_back() {
    let (temporary, tool) = fixture("moved-parent");
    let root = temporary.path().to_owned();
    let mut evidence = TraceEvidence::new();
    evidence.checkpoint_action = Some((
        CreateFolderCheckpoint::BeforeMkdir(1, 1),
        Box::new(move || {
            fs::rename(root.join("moved"), root.join("displaced")).unwrap();
            fs::create_dir(root.join("moved")).unwrap();
        }),
    ));

    assert_ambiguous(
        &execute_with(
            &tool,
            "moved/final",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err(),
    );
    assert!(temporary.path().join("displaced/final").is_dir());
    assert!(temporary.path().join("moved").is_dir());
    assert!(!temporary.path().join("moved/final").exists());
    assert_eq!(
        evidence.sync_operations(),
        vec![
            (CreateFolderSyncSite::CreatedDirectory(1), 0),
            (CreateFolderSyncSite::CreatedDirectory(0), 0),
            (CreateFolderSyncSite::FirstCreatedParent(0), 0),
        ]
    );
}

#[test]
fn sync_retries_are_per_site_bounded_and_all_sites_are_attempted_after_failure() {
    let (_temporary, tool) = fixture("sync-caps");
    let mut evidence = TraceEvidence::new();
    for site in [
        CreateFolderSyncSite::CreatedDirectory(1),
        CreateFolderSyncSite::CreatedDirectory(0),
        CreateFolderSyncSite::FirstCreatedParent(0),
    ] {
        for attempt in 0..15 {
            evidence
                .sync_errors
                .push((site, attempt, rustix::io::Errno::INTR));
        }
    }
    assert!(
        execute_with(
            &tool,
            "parent/final",
            &CancellationToken::new(),
            &mut evidence,
        )
        .is_ok()
    );
    assert_eq!(evidence.sync_operations().len(), 3 * 16);

    let (_temporary, tool) = fixture("sync-attempt-all");
    let mut evidence = TraceEvidence::new();
    evidence.sync_errors.push((
        CreateFolderSyncSite::CreatedDirectory(1),
        0,
        rustix::io::Errno::IO,
    ));
    assert_ambiguous(
        &execute_with(
            &tool,
            "parent/final",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err(),
    );
    assert_eq!(
        evidence.sync_operations(),
        vec![
            (CreateFolderSyncSite::CreatedDirectory(1), 0),
            (CreateFolderSyncSite::CreatedDirectory(0), 0),
            (CreateFolderSyncSite::FirstCreatedParent(0), 0),
        ]
    );
}

#[test]
fn exact_maximum_sync_bound_is_four_thousand_one_hundred_twelve_calls() {
    let (_temporary, tool) = fixture("sync-total-bound");
    let path = vec!["c"; MAX_CREATE_FOLDER_PATH_COMPONENTS].join("/");
    let mut evidence = TraceEvidence::new();
    for component_index in 0..MAX_CREATE_FOLDER_PATH_COMPONENTS {
        for attempt in 0..16 {
            evidence.sync_errors.push((
                CreateFolderSyncSite::CreatedDirectory(component_index),
                attempt,
                rustix::io::Errno::INTR,
            ));
        }
    }
    for attempt in 0..16 {
        evidence.sync_errors.push((
            CreateFolderSyncSite::FirstCreatedParent(0),
            attempt,
            rustix::io::Errno::INTR,
        ));
    }

    assert_ambiguous(
        &execute_with(&tool, &path, &CancellationToken::new(), &mut evidence).unwrap_err(),
    );
    assert_eq!(
        evidence.mkdir_operations().len(),
        MAX_CREATE_FOLDER_MKDIR_CALLS
    );
    assert_eq!(
        evidence.sync_operations().len(),
        MAX_CREATE_FOLDER_SYNC_CALLS
    );
    assert!(
        evidence
            .sync_operations()
            .iter()
            .all(|(_, attempt)| *attempt < 16)
    );
}

#[test]
fn deterministic_unopenable_created_component_is_ambiguous_and_still_syncs_parent() {
    let (temporary, tool) = fixture("unopenable-created");
    let mut evidence = TraceEvidence::new();
    evidence.open_fault = Some((
        CreateFolderPhase::Postcommit,
        CreateFolderOpenSite::Component(0),
        rustix::io::Errno::ACCESS,
    ));

    assert_ambiguous(
        &execute_with(
            &tool,
            "partial/final",
            &CancellationToken::new(),
            &mut evidence,
        )
        .unwrap_err(),
    );
    assert!(temporary.path().join("partial").is_dir());
    assert!(!temporary.path().join("partial/final").exists());
    assert_eq!(
        evidence
            .operations
            .iter()
            .filter(|operation| {
                matches!(
                    operation,
                    Operation::Open(
                        CreateFolderPhase::Postcommit,
                        CreateFolderOpenSite::Root,
                        _,
                        _
                    )
                )
            })
            .count(),
        1
    );
    assert_eq!(
        evidence.sync_operations(),
        vec![(CreateFolderSyncSite::FirstCreatedParent(0), 0)]
    );
}

#[test]
fn exact_fixed_permission_changed_and_create_failed_errors_are_reachable_precommit() {
    for (label, error, kind, code, message, retryable) in [
        (
            "permission",
            rustix::io::Errno::ACCESS,
            ToolErrorKind::PermissionDenied,
            "create_folder_permission_denied",
            "requested folder cannot be created",
            false,
        ),
        (
            "changed",
            rustix::io::Errno::NOTDIR,
            ToolErrorKind::Execution,
            "create_folder_target_changed",
            "requested folder path changed during creation",
            true,
        ),
        (
            "failed",
            rustix::io::Errno::IO,
            ToolErrorKind::Execution,
            "create_folder_create_failed",
            "requested folder could not be created",
            true,
        ),
    ] {
        let (temporary, tool) = fixture(label);
        let mut evidence = TraceEvidence::new();
        evidence.mkdir_actions = vec![MkdirAction::Return(error)];
        let actual =
            execute_with(&tool, "folder", &CancellationToken::new(), &mut evidence).unwrap_err();
        assert_error(&actual, kind, code, message, retryable);
        assert!(!temporary.path().join("folder").exists());
        assert!(evidence.sync_operations().is_empty());
    }
}
