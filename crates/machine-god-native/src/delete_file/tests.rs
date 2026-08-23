#![cfg(any(target_os = "linux", target_os = "macos"))]

use super::*;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rustix::fd::{BorrowedFd, OwnedFd};
use rustix::fs::{AtFlags, Mode};
use serde_json::json;

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mg-delete-private-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("private delete fixture root can be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove private delete fixture: {error}"),
        }
    }
}

struct ModeRestoreGuard {
    path: PathBuf,
    mode: u32,
}

impl ModeRestoreGuard {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            mode: fs::metadata(path).unwrap().permissions().mode(),
        }
    }
}

impl Drop for ModeRestoreGuard {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
    }
}

fn create_nested_target(root: &Path, kind: TargetKind) -> PathBuf {
    let parent = root.join("a/b");
    fs::create_dir_all(&parent).unwrap();
    let target = parent.join("target");
    match kind {
        TargetKind::RegularFile => fs::write(&target, b"original bytes").unwrap(),
        TargetKind::Directory => fs::create_dir(&target).unwrap(),
    }
    target
}

fn execute_with<Evidence: DeleteFileEvidence>(
    tool: &DeleteFileTool,
    path: &str,
    evidence: &mut Evidence,
) -> Result<ToolOutput, ToolError> {
    tool.execute_supported_with_evidence(path, &CancellationToken::new(), evidence)
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
        "delete_file_cancelled",
        "delete_file execution was cancelled",
        false,
    );
}

fn assert_unavailable(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::Unavailable,
        "delete_file_unavailable",
        "requested path is unavailable",
        true,
    );
}

fn assert_invalid_path(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::InvalidInput,
        "delete_file_invalid_path",
        "delete_file path is invalid",
        false,
    );
}

fn assert_not_found(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::Unavailable,
        "delete_file_not_found",
        "requested path is unavailable",
        false,
    );
}

fn assert_permission_denied(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::PermissionDenied,
        "delete_file_permission_denied",
        "requested path cannot be deleted",
        false,
    );
}

fn assert_path_rejected(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::PermissionDenied,
        "delete_file_path_rejected",
        "requested path is not a confined regular file or empty directory",
        false,
    );
}

fn assert_target_changed(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::Execution,
        "delete_file_target_changed",
        "requested path changed before deletion",
        true,
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObservedUnlinkOutcome {
    Success,
    Error(rustix::io::Errno),
}

impl From<Result<(), rustix::io::Errno>> for ObservedUnlinkOutcome {
    fn from(outcome: Result<(), rustix::io::Errno>) -> Self {
        match outcome {
            Ok(()) => Self::Success,
            Err(error) => Self::Error(error),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Operation {
    Open(DeletePhase, OpenSite, usize, String),
    Fstat(DeletePhase, FstatSite, usize),
    Statat(DeletePhase, StatatSite, usize, String),
    AfterValidation(DeletePhase, String),
    FinalPreUnlink(String, TargetKind),
    Unlink(String, AtFlags),
    AfterUnlink(String, TargetKind, AtFlags, ObservedUnlinkOutcome),
    Sync(usize),
}

#[derive(Default)]
struct TraceEvidence {
    operations: Vec<Operation>,
    checkpoints: Vec<DeleteCheckpoint>,
}

impl DeleteFileEvidence for TraceEvidence {
    fn checkpoint(&mut self, checkpoint: DeleteCheckpoint, _cancellation: &CancellationToken) {
        self.checkpoints.push(checkpoint);
    }

    fn open_walk(
        &mut self,
        phase: DeletePhase,
        site: OpenSite,
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
        rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
    }

    fn fstat(
        &mut self,
        phase: DeletePhase,
        site: FstatSite,
        ordinal: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        self.operations.push(Operation::Fstat(phase, site, ordinal));
        rustix::fs::fstat(descriptor)
    }

    fn statat(
        &mut self,
        phase: DeletePhase,
        site: StatatSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        let observed_name = match site {
            #[cfg(target_os = "macos")]
            StatatSite::LinkedRoot => String::new(),
            StatatSite::Target => name.to_string_lossy().into_owned(),
        };
        self.operations
            .push(Operation::Statat(phase, site, ordinal, observed_name));
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn after_validation(
        &mut self,
        phase: DeletePhase,
        _parent: BorrowedFd<'_>,
        basename: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        self.operations
            .push(Operation::AfterValidation(phase, basename.to_owned()));
        Ok(())
    }

    fn final_pre_unlink(
        &mut self,
        _parent: BorrowedFd<'_>,
        basename: &str,
        kind: TargetKind,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        self.operations
            .push(Operation::FinalPreUnlink(basename.to_owned(), kind));
        Ok(())
    }

    fn unlink(
        &mut self,
        parent: BorrowedFd<'_>,
        basename: &str,
        flags: AtFlags,
    ) -> Result<(), rustix::io::Errno> {
        self.operations
            .push(Operation::Unlink(basename.to_owned(), flags));
        rustix::fs::unlinkat(parent, basename, flags)
    }

    fn after_unlink(
        &mut self,
        _parent: BorrowedFd<'_>,
        basename: &str,
        kind: TargetKind,
        flags: AtFlags,
        outcome: Result<(), rustix::io::Errno>,
        _cancellation: &CancellationToken,
    ) -> Result<(), rustix::io::Errno> {
        self.operations.push(Operation::AfterUnlink(
            basename.to_owned(),
            kind,
            flags,
            outcome.into(),
        ));
        Ok(())
    }

    fn sync_parent(
        &mut self,
        attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.operations.push(Operation::Sync(attempt));
        rustix::fs::fsync(parent)
    }
}

fn expected_phase_operations(
    phase: DeletePhase,
    open_base: usize,
    fstat_base: usize,
    statat_base: usize,
) -> Vec<Operation> {
    let mut operations = vec![
        Operation::Open(phase, OpenSite::Root, open_base, ".".to_owned()),
        Operation::Fstat(phase, FstatSite::Root, fstat_base),
    ];
    #[cfg(target_os = "macos")]
    operations.extend([
        Operation::Open(phase, OpenSite::RootParent, open_base + 1, "..".to_owned()),
        Operation::Statat(phase, StatatSite::LinkedRoot, statat_base, String::new()),
    ]);
    let intermediate_open_base = if cfg!(target_os = "macos") {
        open_base + 2
    } else {
        open_base + 1
    };
    operations.extend([
        Operation::Open(
            phase,
            OpenSite::Intermediate(0),
            intermediate_open_base,
            "a".to_owned(),
        ),
        Operation::Fstat(phase, FstatSite::Intermediate(0), fstat_base + 1),
        Operation::Open(
            phase,
            OpenSite::Intermediate(1),
            intermediate_open_base + 1,
            "b".to_owned(),
        ),
        Operation::Fstat(phase, FstatSite::Intermediate(1), fstat_base + 2),
        Operation::Fstat(phase, FstatSite::FinalParent, fstat_base + 3),
        Operation::Statat(
            phase,
            StatatSite::Target,
            if cfg!(target_os = "macos") {
                statat_base + 1
            } else {
                statat_base
            },
            "target".to_owned(),
        ),
        Operation::AfterValidation(phase, "target".to_owned()),
    ]);
    operations
}

fn expected_operations(kind: TargetKind) -> Vec<Operation> {
    let mut operations = expected_phase_operations(DeletePhase::Initial, 0, 0, 0);
    #[cfg(target_os = "linux")]
    operations.extend(expected_phase_operations(DeletePhase::Revalidate, 3, 4, 1));
    #[cfg(target_os = "macos")]
    operations.extend(expected_phase_operations(DeletePhase::Revalidate, 4, 4, 2));
    let flags = match kind {
        TargetKind::RegularFile => AtFlags::empty(),
        TargetKind::Directory => AtFlags::REMOVEDIR,
    };
    operations.extend([
        Operation::FinalPreUnlink("target".to_owned(), kind),
        Operation::Unlink("target".to_owned(), flags),
        Operation::AfterUnlink(
            "target".to_owned(),
            kind,
            flags,
            ObservedUnlinkOutcome::Success,
        ),
        Operation::Sync(0),
    ]);
    operations
}

fn push_operation_checkpoints(
    checkpoints: &mut Vec<DeleteCheckpoint>,
    phase: DeletePhase,
    open_base: usize,
    fstat_base: usize,
    statat_base: usize,
) {
    checkpoints.extend([
        DeleteCheckpoint::BeforeOpen(phase, OpenSite::Root, open_base),
        DeleteCheckpoint::AfterOpen(phase, OpenSite::Root, open_base),
        DeleteCheckpoint::BeforeFstat(phase, FstatSite::Root, fstat_base),
        DeleteCheckpoint::AfterFstat(phase, FstatSite::Root, fstat_base),
    ]);
    #[cfg(target_os = "macos")]
    checkpoints.extend([
        DeleteCheckpoint::BeforeOpen(phase, OpenSite::RootParent, open_base + 1),
        DeleteCheckpoint::AfterOpen(phase, OpenSite::RootParent, open_base + 1),
        DeleteCheckpoint::BeforeStatat(phase, StatatSite::LinkedRoot, statat_base),
        DeleteCheckpoint::AfterStatat(phase, StatatSite::LinkedRoot, statat_base),
    ]);
    checkpoints.push(DeleteCheckpoint::AfterRootValidation(phase));
    let intermediate_open_base = if cfg!(target_os = "macos") {
        open_base + 2
    } else {
        open_base + 1
    };
    for depth in 0..2 {
        checkpoints.extend([
            DeleteCheckpoint::BeforeOpen(
                phase,
                OpenSite::Intermediate(depth),
                intermediate_open_base + depth,
            ),
            DeleteCheckpoint::AfterOpen(
                phase,
                OpenSite::Intermediate(depth),
                intermediate_open_base + depth,
            ),
            DeleteCheckpoint::BeforeFstat(
                phase,
                FstatSite::Intermediate(depth),
                fstat_base + depth + 1,
            ),
            DeleteCheckpoint::AfterFstat(
                phase,
                FstatSite::Intermediate(depth),
                fstat_base + depth + 1,
            ),
        ]);
    }
    checkpoints.extend([
        DeleteCheckpoint::BeforeFstat(phase, FstatSite::FinalParent, fstat_base + 3),
        DeleteCheckpoint::AfterFstat(phase, FstatSite::FinalParent, fstat_base + 3),
        DeleteCheckpoint::BeforeStatat(
            phase,
            StatatSite::Target,
            if cfg!(target_os = "macos") {
                statat_base + 1
            } else {
                statat_base
            },
        ),
        DeleteCheckpoint::AfterStatat(
            phase,
            StatatSite::Target,
            if cfg!(target_os = "macos") {
                statat_base + 1
            } else {
                statat_base
            },
        ),
        DeleteCheckpoint::AfterValidation(phase),
    ]);
}

fn expected_checkpoints() -> Vec<DeleteCheckpoint> {
    let mut checkpoints = Vec::new();
    push_operation_checkpoints(&mut checkpoints, DeletePhase::Initial, 0, 0, 0);
    #[cfg(target_os = "linux")]
    push_operation_checkpoints(&mut checkpoints, DeletePhase::Revalidate, 3, 4, 1);
    #[cfg(target_os = "macos")]
    push_operation_checkpoints(&mut checkpoints, DeletePhase::Revalidate, 4, 4, 2);
    checkpoints.extend([
        DeleteCheckpoint::FinalPreUnlink,
        DeleteCheckpoint::AfterDelete,
    ]);
    checkpoints
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FaultPoint {
    Open(DeletePhase, OpenSite, usize),
    Fstat(DeletePhase, FstatSite, usize),
    Statat(DeletePhase, StatatSite, usize),
}

struct FaultEvidence {
    point: FaultPoint,
    error: rustix::io::Errno,
    hits: usize,
    unlink_calls: usize,
    sync_calls: usize,
}

struct CancelAndFaultEvidence {
    point: FaultPoint,
    cancellation: CancellationToken,
    checkpoints: Vec<DeleteCheckpoint>,
    hits: usize,
    unlink_calls: usize,
    sync_calls: usize,
}

impl CancelAndFaultEvidence {
    fn new(point: FaultPoint, cancellation: CancellationToken) -> Self {
        Self {
            point,
            cancellation,
            checkpoints: Vec::new(),
            hits: 0,
            unlink_calls: 0,
            sync_calls: 0,
        }
    }
}

impl DeleteFileEvidence for CancelAndFaultEvidence {
    fn checkpoint(&mut self, checkpoint: DeleteCheckpoint, _cancellation: &CancellationToken) {
        self.checkpoints.push(checkpoint);
    }

    fn open_walk(
        &mut self,
        phase: DeletePhase,
        site: OpenSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        if self.point == FaultPoint::Open(phase, site, ordinal) {
            self.hits += 1;
            let _ = self.cancellation.cancel();
            Err(rustix::io::Errno::IO)
        } else {
            rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
        }
    }

    fn fstat(
        &mut self,
        phase: DeletePhase,
        site: FstatSite,
        ordinal: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        if self.point == FaultPoint::Fstat(phase, site, ordinal) {
            self.hits += 1;
            let _ = self.cancellation.cancel();
            Err(rustix::io::Errno::IO)
        } else {
            rustix::fs::fstat(descriptor)
        }
    }

    fn statat(
        &mut self,
        phase: DeletePhase,
        site: StatatSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        if self.point == FaultPoint::Statat(phase, site, ordinal) {
            self.hits += 1;
            let _ = self.cancellation.cancel();
            Err(rustix::io::Errno::IO)
        } else {
            rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        }
    }

    fn unlink(
        &mut self,
        parent: BorrowedFd<'_>,
        basename: &str,
        flags: AtFlags,
    ) -> Result<(), rustix::io::Errno> {
        self.unlink_calls += 1;
        rustix::fs::unlinkat(parent, basename, flags)
    }

    fn sync_parent(
        &mut self,
        _attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.sync_calls += 1;
        rustix::fs::fsync(parent)
    }
}

impl FaultEvidence {
    const fn new(point: FaultPoint, error: rustix::io::Errno) -> Self {
        Self {
            point,
            error,
            hits: 0,
            unlink_calls: 0,
            sync_calls: 0,
        }
    }
}

impl DeleteFileEvidence for FaultEvidence {
    fn open_walk(
        &mut self,
        phase: DeletePhase,
        site: OpenSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        component: &OsStr,
    ) -> Result<OwnedFd, rustix::io::Errno> {
        if self.point == FaultPoint::Open(phase, site, ordinal) {
            self.hits += 1;
            Err(self.error)
        } else {
            rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
        }
    }

    fn fstat(
        &mut self,
        phase: DeletePhase,
        site: FstatSite,
        ordinal: usize,
        descriptor: BorrowedFd<'_>,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        if self.point == FaultPoint::Fstat(phase, site, ordinal) {
            self.hits += 1;
            Err(self.error)
        } else {
            rustix::fs::fstat(descriptor)
        }
    }

    fn statat(
        &mut self,
        phase: DeletePhase,
        site: StatatSite,
        ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        if self.point == FaultPoint::Statat(phase, site, ordinal) {
            self.hits += 1;
            Err(self.error)
        } else {
            rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        }
    }

    fn unlink(
        &mut self,
        parent: BorrowedFd<'_>,
        basename: &str,
        flags: AtFlags,
    ) -> Result<(), rustix::io::Errno> {
        self.unlink_calls += 1;
        rustix::fs::unlinkat(parent, basename, flags)
    }

    fn sync_parent(
        &mut self,
        _attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.sync_calls += 1;
        rustix::fs::fsync(parent)
    }
}

struct CancelAtEvidence {
    target: DeleteCheckpoint,
    seen: Vec<DeleteCheckpoint>,
    unlink_calls: usize,
    sync_calls: usize,
}

impl CancelAtEvidence {
    const fn new(target: DeleteCheckpoint) -> Self {
        Self {
            target,
            seen: Vec::new(),
            unlink_calls: 0,
            sync_calls: 0,
        }
    }
}

impl DeleteFileEvidence for CancelAtEvidence {
    fn checkpoint(&mut self, checkpoint: DeleteCheckpoint, cancellation: &CancellationToken) {
        self.seen.push(checkpoint);
        if checkpoint == self.target {
            let _ = cancellation.cancel();
        }
    }

    fn unlink(
        &mut self,
        parent: BorrowedFd<'_>,
        basename: &str,
        flags: AtFlags,
    ) -> Result<(), rustix::io::Errno> {
        self.unlink_calls += 1;
        rustix::fs::unlinkat(parent, basename, flags)
    }

    fn sync_parent(
        &mut self,
        _attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.sync_calls += 1;
        rustix::fs::fsync(parent)
    }
}

enum RevalidationAction {
    ReplaceTargetWithFile(PathBuf),
    RemoveTarget(PathBuf),
    ReplaceFileWithDirectory(PathBuf),
    ReplaceDirectoryWithFile(PathBuf),
    RemoveIntermediatePermissions(PathBuf),
    ReplaceParent {
        parent: PathBuf,
        moved: PathBuf,
        replacement_target: PathBuf,
    },
}

struct RevalidationEvidence {
    action: Option<RevalidationAction>,
    mode_restore: Option<ModeRestoreGuard>,
    mode_was_enforced: bool,
    unlink_calls: usize,
    sync_calls: usize,
}

impl RevalidationEvidence {
    const fn new(action: RevalidationAction) -> Self {
        Self {
            action: Some(action),
            mode_restore: None,
            mode_was_enforced: false,
            unlink_calls: 0,
            sync_calls: 0,
        }
    }
}

impl DeleteFileEvidence for RevalidationEvidence {
    fn after_validation(
        &mut self,
        phase: DeletePhase,
        _parent: BorrowedFd<'_>,
        _basename: &str,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        if phase != DeletePhase::Initial {
            return Ok(());
        }
        let Some(action) = self.action.take() else {
            return Ok(());
        };
        match action {
            RevalidationAction::ReplaceTargetWithFile(target) => {
                fs::remove_file(&target).unwrap();
                fs::write(target, b"replacement bytes").unwrap();
            }
            RevalidationAction::RemoveTarget(target) => {
                fs::remove_file(target).unwrap();
            }
            RevalidationAction::ReplaceFileWithDirectory(target) => {
                fs::remove_file(&target).unwrap();
                fs::create_dir(target).unwrap();
            }
            RevalidationAction::ReplaceDirectoryWithFile(target) => {
                fs::remove_dir(&target).unwrap();
                fs::write(target, b"replacement file").unwrap();
            }
            RevalidationAction::RemoveIntermediatePermissions(intermediate) => {
                self.mode_restore = Some(ModeRestoreGuard::new(&intermediate));
                fs::set_permissions(&intermediate, fs::Permissions::from_mode(0o000)).unwrap();
                self.mode_was_enforced =
                    rustix::fs::open(&intermediate, directory_open_flags(), Mode::empty()).is_err();
            }
            RevalidationAction::ReplaceParent {
                parent,
                moved,
                replacement_target,
            } => {
                fs::rename(&parent, &moved).unwrap();
                fs::create_dir(&parent).unwrap();
                fs::write(replacement_target, b"replacement parent sentinel").unwrap();
            }
        }
        Ok(())
    }

    fn unlink(
        &mut self,
        parent: BorrowedFd<'_>,
        basename: &str,
        flags: AtFlags,
    ) -> Result<(), rustix::io::Errno> {
        self.unlink_calls += 1;
        rustix::fs::unlinkat(parent, basename, flags)
    }

    fn sync_parent(
        &mut self,
        _attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.sync_calls += 1;
        rustix::fs::fsync(parent)
    }
}

fn fault_points_from_trace() -> Vec<FaultPoint> {
    expected_operations(TargetKind::RegularFile)
        .into_iter()
        .filter_map(|operation| match operation {
            Operation::Open(phase, site, ordinal, _) => {
                Some(FaultPoint::Open(phase, site, ordinal))
            }
            Operation::Fstat(phase, site, ordinal) => Some(FaultPoint::Fstat(phase, site, ordinal)),
            Operation::Statat(phase, site, ordinal, _) => {
                Some(FaultPoint::Statat(phase, site, ordinal))
            }
            Operation::AfterValidation(_, _)
            | Operation::FinalPreUnlink(_, _)
            | Operation::Unlink(_, _)
            | Operation::AfterUnlink(_, _, _, _)
            | Operation::Sync(_) => None,
        })
        .collect()
}

fn after_checkpoint_for_fault(point: FaultPoint) -> DeleteCheckpoint {
    match point {
        FaultPoint::Open(phase, site, ordinal) => DeleteCheckpoint::AfterOpen(phase, site, ordinal),
        FaultPoint::Fstat(phase, site, ordinal) => {
            DeleteCheckpoint::AfterFstat(phase, site, ordinal)
        }
        FaultPoint::Statat(phase, site, ordinal) => {
            DeleteCheckpoint::AfterStatat(phase, site, ordinal)
        }
    }
}

fn is_operational_fault_point(point: FaultPoint) -> bool {
    match point {
        FaultPoint::Open(_, OpenSite::Root, _) | FaultPoint::Fstat(_, FstatSite::Root, _) => true,
        #[cfg(target_os = "macos")]
        FaultPoint::Open(_, OpenSite::RootParent, _)
        | FaultPoint::Statat(_, StatatSite::LinkedRoot, _) => true,
        FaultPoint::Open(_, OpenSite::Intermediate(_), _)
        | FaultPoint::Fstat(_, FstatSite::Intermediate(_) | FstatSite::FinalParent, _)
        | FaultPoint::Statat(_, StatatSite::Target, _) => false,
    }
}

#[test]
fn serialized_argument_and_result_helpers_enforce_exact_one_under_and_one_over_bounds() {
    let empty_arguments = json!({"path": ""});
    let argument_overhead = serde_json::to_vec(&empty_arguments).unwrap().len();
    for (size, fits) in [
        (MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES - 1, true),
        (MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES, true),
        (MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES + 1, false),
    ] {
        let arguments = json!({"path": "x".repeat(size - argument_overhead)});
        assert_eq!(serde_json::to_vec(&arguments).unwrap().len(), size);
        assert_eq!(
            serialized_value_fits(&arguments, MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES),
            fits
        );
    }

    let empty_output = ToolOutput::success(json!({"path": ""}));
    let output_overhead = serde_json::to_vec(&empty_output).unwrap().len();
    let one_under_path = "x".repeat(MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES - 1 - output_overhead);
    let exact_path = format!("{one_under_path}x");
    let one_over_path = format!("{exact_path}x");
    assert_eq!(
        serde_json::to_vec(&ToolOutput::success(json!({"path": &one_under_path})))
            .unwrap()
            .len(),
        MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES - 1
    );
    assert_eq!(
        serde_json::to_vec(&ToolOutput::success(json!({"path": &exact_path})))
            .unwrap()
            .len(),
        MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES
    );
    assert_eq!(
        build_success_output_with_limit(&one_under_path, MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES)
            .unwrap(),
        ToolOutput::success(json!({"path": one_under_path}))
    );
    assert!(
        build_success_output_with_limit(&exact_path, MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES)
            .is_ok()
    );
    let one_over_error =
        build_success_output_with_limit(&one_over_path, MAX_DELETE_FILE_SERIALIZED_RESULT_BYTES)
            .unwrap_err();
    assert_error(
        &one_over_error,
        ToolErrorKind::Execution,
        "delete_file_delete_failed",
        "requested path could not be deleted",
        true,
    );
}

#[test]
fn path_bound_precedes_serialized_argument_bound_for_a_far_oversized_path() {
    let arguments = json!({
        "path": "x".repeat(MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES * 4)
    });
    assert!(
        serde_json::to_vec(&arguments).unwrap().len() > MAX_DELETE_FILE_SERIALIZED_ARGUMENT_BYTES
    );

    let Err(error) = validate_arguments(&arguments) else {
        panic!("far oversized path was unexpectedly accepted");
    };
    assert_invalid_path(&error);
}

#[test]
fn real_file_and_directory_pipelines_have_exact_bounded_operation_vocabulary_and_flags() {
    for (label, kind) in [
        ("trace-file", TargetKind::RegularFile),
        ("trace-directory", TargetKind::Directory),
    ] {
        let temporary = TemporaryDirectory::new(label);
        let target = create_nested_target(temporary.path(), kind);
        let tool = DeleteFileTool::open(temporary.path()).unwrap();
        let mut evidence = TraceEvidence::default();

        let output = execute_with(&tool, "a/b/target", &mut evidence).unwrap();

        assert_eq!(output, ToolOutput::success(json!({"path": "a/b/target"})));
        assert!(!target.exists());
        assert_eq!(evidence.operations, expected_operations(kind));
        assert_eq!(evidence.checkpoints, expected_checkpoints());
        assert!(evidence.operations.iter().all(|operation| !matches!(
            operation,
            Operation::Open(_, _, _, component) if component == "target"
        )));
        assert_eq!(
            evidence
                .operations
                .iter()
                .filter(|operation| matches!(operation, Operation::Unlink(_, _)))
                .count(),
            1
        );
        assert_eq!(
            evidence
                .operations
                .iter()
                .filter(|operation| matches!(operation, Operation::Sync(_)))
                .count(),
            1
        );
    }
}

#[test]
fn every_phase_site_and_ordinal_open_fstat_and_statat_fault_maps_precommit_exactly() {
    for (index, point) in fault_points_from_trace().into_iter().enumerate() {
        let temporary = TemporaryDirectory::new(&format!("fault-point-{index}"));
        let target = create_nested_target(temporary.path(), TargetKind::RegularFile);
        let tool = DeleteFileTool::open(temporary.path()).unwrap();
        let phase = match point {
            FaultPoint::Open(phase, _, _)
            | FaultPoint::Fstat(phase, _, _)
            | FaultPoint::Statat(phase, _, _) => phase,
        };
        let mut evidence = FaultEvidence::new(point, rustix::io::Errno::IO);

        let error = execute_with(&tool, "a/b/target", &mut evidence).unwrap_err();

        match phase {
            DeletePhase::Initial => assert_unavailable(&error),
            DeletePhase::Revalidate => assert_target_changed(&error),
        }
        assert_eq!(evidence.hits, 1, "fault point was not reached: {point:?}");
        assert_eq!(evidence.unlink_calls, 0, "fault reached unlink: {point:?}");
        assert_eq!(evidence.sync_calls, 0, "fault reached sync: {point:?}");
        assert_eq!(fs::read(&target).unwrap(), b"original bytes");
    }
}

#[test]
fn cancellation_wins_a_same_call_error_at_every_open_fstat_and_statat_site() {
    for (index, point) in fault_points_from_trace().into_iter().enumerate() {
        let temporary = TemporaryDirectory::new(&format!("cancel-error-{index}"));
        let target = create_nested_target(temporary.path(), TargetKind::RegularFile);
        let tool = DeleteFileTool::open(temporary.path()).unwrap();
        let cancellation = CancellationToken::new();
        let mut evidence = CancelAndFaultEvidence::new(point, cancellation.clone());

        let error = tool
            .execute_supported_with_evidence("a/b/target", &cancellation, &mut evidence)
            .unwrap_err();

        assert_cancelled(&error);
        assert!(cancellation.is_cancelled());
        assert_eq!(evidence.hits, 1, "fault point was not reached: {point:?}");
        assert!(
            evidence
                .checkpoints
                .contains(&after_checkpoint_for_fault(point)),
            "after checkpoint was not recorded: {point:?}"
        );
        assert_eq!(evidence.unlink_calls, 0, "fault reached unlink: {point:?}");
        assert_eq!(evidence.sync_calls, 0, "fault reached sync: {point:?}");
        assert_eq!(fs::read(&target).unwrap(), b"original bytes");
    }
}

#[test]
fn permission_errors_are_fixed_at_every_precommit_metadata_site() {
    for (point_index, point) in fault_points_from_trace().into_iter().enumerate() {
        for (errno_index, errno) in [
            rustix::io::Errno::ACCESS,
            rustix::io::Errno::PERM,
            rustix::io::Errno::ROFS,
        ]
        .into_iter()
        .enumerate()
        {
            let temporary =
                TemporaryDirectory::new(&format!("permission-matrix-{point_index}-{errno_index}"));
            let target = create_nested_target(temporary.path(), TargetKind::RegularFile);
            let tool = DeleteFileTool::open(temporary.path()).unwrap();
            let mut evidence = FaultEvidence::new(point, errno);

            let error = execute_with(&tool, "a/b/target", &mut evidence).unwrap_err();

            assert_permission_denied(&error);
            assert_eq!(evidence.hits, 1, "fault point was not reached: {point:?}");
            assert_eq!(evidence.unlink_calls, 0, "fault reached unlink: {point:?}");
            assert_eq!(evidence.sync_calls, 0, "fault reached sync: {point:?}");
            assert_eq!(fs::read(&target).unwrap(), b"original bytes");
        }
    }
}

#[test]
fn root_link_and_descriptor_faults_keep_phase_mapping_for_all_errno_classes() {
    let points = fault_points_from_trace()
        .into_iter()
        .filter(|point| is_operational_fault_point(*point))
        .collect::<Vec<_>>();
    assert!(!points.is_empty());
    for (point_index, point) in points.into_iter().enumerate() {
        for (errno_index, errno) in [
            rustix::io::Errno::NOENT,
            rustix::io::Errno::ACCESS,
            rustix::io::Errno::PERM,
            rustix::io::Errno::LOOP,
        ]
        .into_iter()
        .enumerate()
        {
            let temporary = TemporaryDirectory::new(&format!(
                "operational-taxonomy-{point_index}-{errno_index}"
            ));
            let target = create_nested_target(temporary.path(), TargetKind::RegularFile);
            let tool = DeleteFileTool::open(temporary.path()).unwrap();
            let phase = match point {
                FaultPoint::Open(phase, _, _)
                | FaultPoint::Fstat(phase, _, _)
                | FaultPoint::Statat(phase, _, _) => phase,
            };
            let mut evidence = FaultEvidence::new(point, errno);

            let error = execute_with(&tool, "a/b/target", &mut evidence).unwrap_err();

            match (phase, errno) {
                (_, rustix::io::Errno::ACCESS | rustix::io::Errno::PERM) => {
                    assert_permission_denied(&error);
                }
                (DeletePhase::Initial, _) => assert_unavailable(&error),
                (DeletePhase::Revalidate, _) => assert_target_changed(&error),
            }
            assert_eq!(evidence.hits, 1);
            assert_eq!(evidence.unlink_calls, 0);
            assert_eq!(evidence.sync_calls, 0);
            assert_eq!(fs::read(&target).unwrap(), b"original bytes");
        }
    }
}

#[derive(Clone, Copy)]
enum InitialFaultMapping {
    NotFound,
    PermissionDenied,
    PathRejected,
    Unavailable,
}

fn assert_initial_fault_mapping(error: &ToolError, expected: InitialFaultMapping) {
    match expected {
        InitialFaultMapping::NotFound => assert_not_found(error),
        InitialFaultMapping::PermissionDenied => assert_permission_denied(error),
        InitialFaultMapping::PathRejected => assert_path_rejected(error),
        InitialFaultMapping::Unavailable => assert_unavailable(error),
    }
}

#[test]
fn initial_parent_open_and_target_statat_errno_taxonomies_are_exact() {
    let intermediate_open_ordinal = if cfg!(target_os = "macos") { 2 } else { 1 };
    let target_statat_ordinal = usize::from(cfg!(target_os = "macos"));
    let points = [
        FaultPoint::Open(
            DeletePhase::Initial,
            OpenSite::Intermediate(0),
            intermediate_open_ordinal,
        ),
        FaultPoint::Statat(
            DeletePhase::Initial,
            StatatSite::Target,
            target_statat_ordinal,
        ),
    ];
    let mappings = [
        (rustix::io::Errno::NOENT, InitialFaultMapping::NotFound),
        (
            rustix::io::Errno::ACCESS,
            InitialFaultMapping::PermissionDenied,
        ),
        (rustix::io::Errno::LOOP, InitialFaultMapping::PathRejected),
        (rustix::io::Errno::NOTDIR, InitialFaultMapping::PathRejected),
        (rustix::io::Errno::IO, InitialFaultMapping::Unavailable),
    ];

    for (point_index, point) in points.into_iter().enumerate() {
        for (mapping_index, (errno, expected)) in mappings.iter().copied().enumerate() {
            let temporary =
                TemporaryDirectory::new(&format!("initial-taxonomy-{point_index}-{mapping_index}"));
            let target = create_nested_target(temporary.path(), TargetKind::RegularFile);
            let tool = DeleteFileTool::open(temporary.path()).unwrap();
            let mut evidence = FaultEvidence::new(point, errno);

            let error = execute_with(&tool, "a/b/target", &mut evidence).unwrap_err();

            assert_initial_fault_mapping(&error, expected);
            assert_eq!(evidence.hits, 1);
            assert_eq!(evidence.unlink_calls, 0);
            assert_eq!(evidence.sync_calls, 0);
            assert_eq!(fs::read(&target).unwrap(), b"original bytes");
        }
    }
}

#[test]
fn every_real_precommit_checkpoint_honors_cancellation_without_unlink_or_sync() {
    let mut checkpoints = expected_checkpoints();
    assert_eq!(checkpoints.pop(), Some(DeleteCheckpoint::AfterDelete));
    assert!(!checkpoints.is_empty());

    for (index, checkpoint) in checkpoints.into_iter().enumerate() {
        let temporary = TemporaryDirectory::new(&format!("cancel-{index}"));
        let target = create_nested_target(temporary.path(), TargetKind::RegularFile);
        let tool = DeleteFileTool::open(temporary.path()).unwrap();
        let mut evidence = CancelAtEvidence::new(checkpoint);

        let error = execute_with(&tool, "a/b/target", &mut evidence).unwrap_err();

        assert_cancelled(&error);
        assert!(evidence.seen.contains(&checkpoint));
        assert_eq!(
            evidence.unlink_calls, 0,
            "cancel reached unlink: {checkpoint:?}"
        );
        assert_eq!(
            evidence.sync_calls, 0,
            "cancel reached sync: {checkpoint:?}"
        );
        assert_eq!(fs::read(&target).unwrap(), b"original bytes");
    }
}

#[test]
fn real_intermediate_mode_loss_after_initial_validation_is_permission_denied() {
    let temporary = TemporaryDirectory::new("intermediate-mode-loss");
    let target = create_nested_target(temporary.path(), TargetKind::RegularFile);
    let intermediate = temporary.path().join("a");
    let original_mode = fs::metadata(&intermediate).unwrap().permissions().mode();
    let tool = DeleteFileTool::open(temporary.path()).unwrap();
    let mut evidence = RevalidationEvidence::new(
        RevalidationAction::RemoveIntermediatePermissions(intermediate.clone()),
    );

    let result = execute_with(&tool, "a/b/target", &mut evidence);
    let mode_was_enforced = evidence.mode_was_enforced;

    if mode_was_enforced {
        assert_permission_denied(&result.unwrap_err());
        assert_eq!(evidence.unlink_calls, 0);
        assert_eq!(evidence.sync_calls, 0);
    } else {
        assert_eq!(
            result.unwrap(),
            ToolOutput::success(json!({"path": "a/b/target"}))
        );
        assert_eq!(evidence.unlink_calls, 1);
        assert_eq!(evidence.sync_calls, 1);
    }

    drop(evidence);
    assert_eq!(
        fs::metadata(&intermediate).unwrap().permissions().mode() & 0o7777,
        original_mode & 0o7777
    );
    if mode_was_enforced {
        assert_eq!(fs::read(target).unwrap(), b"original bytes");
    } else {
        assert!(!target.exists());
    }
}

#[test]
fn target_inode_absence_and_both_type_changes_are_rejected_during_revalidation() {
    for (label, initial_kind, action_kind) in [
        ("replace-target", TargetKind::RegularFile, 0_u8),
        ("remove-target", TargetKind::RegularFile, 1),
        ("file-to-directory", TargetKind::RegularFile, 2),
        ("directory-to-file", TargetKind::Directory, 3),
    ] {
        let temporary = TemporaryDirectory::new(label);
        let target = create_nested_target(temporary.path(), initial_kind);
        let action = match action_kind {
            0 => RevalidationAction::ReplaceTargetWithFile(target.clone()),
            1 => RevalidationAction::RemoveTarget(target.clone()),
            2 => RevalidationAction::ReplaceFileWithDirectory(target.clone()),
            3 => RevalidationAction::ReplaceDirectoryWithFile(target.clone()),
            _ => unreachable!(),
        };
        let tool = DeleteFileTool::open(temporary.path()).unwrap();
        let mut evidence = RevalidationEvidence::new(action);

        let error = execute_with(&tool, "a/b/target", &mut evidence).unwrap_err();

        assert_target_changed(&error);
        assert_eq!(evidence.unlink_calls, 0);
        assert_eq!(evidence.sync_calls, 0);
        match action_kind {
            0 => assert_eq!(fs::read(&target).unwrap(), b"replacement bytes"),
            1 => assert!(!target.exists()),
            2 => assert!(target.is_dir()),
            3 => assert_eq!(fs::read(&target).unwrap(), b"replacement file"),
            _ => unreachable!(),
        }
    }
}

#[test]
fn final_parent_identity_replacement_is_rejected_before_target_inspection_or_unlink() {
    let temporary = TemporaryDirectory::new("replace-final-parent");
    let target = create_nested_target(temporary.path(), TargetKind::RegularFile);
    let parent = target.parent().unwrap().to_path_buf();
    let moved = temporary.path().join("a/moved-b");
    let replacement_target = parent.join("target");
    let tool = DeleteFileTool::open(temporary.path()).unwrap();
    let mut evidence = RevalidationEvidence::new(RevalidationAction::ReplaceParent {
        parent: parent.clone(),
        moved: moved.clone(),
        replacement_target: replacement_target.clone(),
    });

    let error = execute_with(&tool, "a/b/target", &mut evidence).unwrap_err();

    assert_target_changed(&error);
    assert_eq!(evidence.unlink_calls, 0);
    assert_eq!(evidence.sync_calls, 0);
    assert_eq!(fs::read(moved.join("target")).unwrap(), b"original bytes");
    assert_eq!(
        fs::read(replacement_target).unwrap(),
        b"replacement parent sentinel"
    );
}
