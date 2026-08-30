#![cfg(any(target_os = "linux", target_os = "macos"))]

#[cfg(target_os = "macos")]
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use super::*;

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(label: &str) -> Self {
        for suffix in 0..1_000_u64 {
            let path = std::env::temp_dir().join(format!(
                "machine-god-delete-race-{label}-{}-{suffix}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create private delete test directory: {error}"),
            }
        }
        panic!("allocate private delete test directory")
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

fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("invoke the POSIX mkfifo utility");
    assert!(status.success(), "mkfifo failed with {status}");
}

enum FinalAction {
    ReplaceFile {
        target: PathBuf,
        displaced: PathBuf,
        replacement: Vec<u8>,
    },
    ReplaceDirectory {
        target: PathBuf,
        displaced: PathBuf,
    },
    ReplaceWithSymlink {
        target: PathBuf,
        displaced: PathBuf,
        referent: PathBuf,
    },
    ReplaceWithFifo {
        target: PathBuf,
        displaced: PathBuf,
    },
    ReplaceWithSocket {
        target: PathBuf,
        displaced: PathBuf,
    },
    ReplaceFileWithDirectory {
        target: PathBuf,
        displaced: PathBuf,
    },
    ReplaceDirectoryWithFile {
        target: PathBuf,
        displaced: PathBuf,
        replacement: Vec<u8>,
    },
    MoveParent {
        original_parent: PathBuf,
        moved_parent: PathBuf,
        replacement: Vec<u8>,
    },
}

impl FinalAction {
    fn run(self, basename: &str) {
        match self {
            Self::ReplaceFile {
                target,
                displaced,
                replacement,
            }
            | Self::ReplaceDirectoryWithFile {
                target,
                displaced,
                replacement,
            } => {
                fs::rename(target, &displaced).unwrap();
                fs::write(displaced.parent().unwrap().join(basename), replacement).unwrap();
            }
            Self::ReplaceDirectory { target, displaced }
            | Self::ReplaceFileWithDirectory { target, displaced } => {
                fs::rename(target, &displaced).unwrap();
                fs::create_dir(displaced.parent().unwrap().join(basename)).unwrap();
            }
            Self::ReplaceWithSymlink {
                target,
                displaced,
                referent,
            } => {
                fs::rename(target, &displaced).unwrap();
                symlink(referent, displaced.parent().unwrap().join(basename)).unwrap();
            }
            Self::ReplaceWithFifo { target, displaced } => {
                fs::rename(target, &displaced).unwrap();
                create_fifo(&displaced.parent().unwrap().join(basename));
            }
            Self::ReplaceWithSocket { target, displaced } => {
                fs::rename(target, &displaced).unwrap();
                let replacement = displaced.parent().unwrap().join(basename);
                let listener = UnixListener::bind(&replacement).unwrap();
                drop(listener);
            }
            Self::MoveParent {
                original_parent,
                moved_parent,
                replacement,
            } => {
                fs::rename(&original_parent, &moved_parent).unwrap();
                fs::create_dir(&original_parent).unwrap();
                fs::write(original_parent.join(basename), replacement).unwrap();
            }
        }
    }
}

#[cfg(target_os = "macos")]
enum MacosAfterUnlinkAction {
    RemoveDirectory(PathBuf),
    ReplaceDirectoryWithSymlink { target: PathBuf, referent: PathBuf },
    ReplaceDirectoryWithFifo(PathBuf),
    ReplaceDirectoryWithSocket(PathBuf),
    ReplaceDirectoryWithFile(PathBuf),
}

#[cfg(target_os = "macos")]
impl MacosAfterUnlinkAction {
    fn run(self) {
        match self {
            Self::RemoveDirectory(target) => fs::remove_dir(target).unwrap(),
            Self::ReplaceDirectoryWithSymlink { target, referent } => {
                fs::remove_dir(&target).unwrap();
                symlink(referent, target).unwrap();
            }
            Self::ReplaceDirectoryWithFifo(target) => {
                fs::remove_dir(&target).unwrap();
                create_fifo(&target);
            }
            Self::ReplaceDirectoryWithSocket(target) => {
                fs::remove_dir(&target).unwrap();
                let listener = UnixListener::bind(target).unwrap();
                drop(listener);
            }
            Self::ReplaceDirectoryWithFile(target) => {
                fs::remove_dir(&target).unwrap();
                fs::write(target, b"different regular file").unwrap();
            }
        }
    }
}

#[derive(Clone, Copy)]
enum UnlinkScript {
    Native,
    Error(rustix::io::Errno),
    NativeThenError(rustix::io::Errno),
}

#[derive(Clone, Copy)]
enum SyncScript {
    Native,
    Error(rustix::io::Errno),
    InterruptionsThenNative(usize),
    AlwaysInterrupted,
}

struct ScriptedEvidence {
    final_action: Option<FinalAction>,
    #[cfg(target_os = "macos")]
    after_unlink_action: Option<MacosAfterUnlinkAction>,
    unlink_script: UnlinkScript,
    sync_script: SyncScript,
    recreate_after_unlink: Option<(PathBuf, Vec<u8>)>,
    cancel_after_unlink: bool,
    final_pre_unlink_calls: usize,
    unlink_calls: usize,
    unlink_flags: Vec<AtFlags>,
    sync_attempts: Vec<usize>,
}

impl ScriptedEvidence {
    fn new(unlink_script: UnlinkScript, sync_script: SyncScript) -> Self {
        Self {
            final_action: None,
            #[cfg(target_os = "macos")]
            after_unlink_action: None,
            unlink_script,
            sync_script,
            recreate_after_unlink: None,
            cancel_after_unlink: false,
            final_pre_unlink_calls: 0,
            unlink_calls: 0,
            unlink_flags: Vec::new(),
            sync_attempts: Vec::new(),
        }
    }

    fn with_final_action(mut self, action: FinalAction) -> Self {
        self.final_action = Some(action);
        self
    }

    #[cfg(target_os = "macos")]
    fn with_after_unlink_action(mut self, action: MacosAfterUnlinkAction) -> Self {
        self.after_unlink_action = Some(action);
        self
    }
}

impl DeleteFileEvidence for ScriptedEvidence {
    fn final_pre_unlink(
        &mut self,
        _parent: BorrowedFd<'_>,
        basename: &str,
        _kind: TargetKind,
        _cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        self.final_pre_unlink_calls += 1;
        if let Some(action) = self.final_action.take() {
            action.run(basename);
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
        self.unlink_flags.push(flags);
        match self.unlink_script {
            UnlinkScript::Native => rustix::fs::unlinkat(parent, basename, flags),
            UnlinkScript::Error(error) => Err(error),
            UnlinkScript::NativeThenError(error) => {
                rustix::fs::unlinkat(parent, basename, flags)?;
                Err(error)
            }
        }
    }

    fn after_unlink(
        &mut self,
        _parent: BorrowedFd<'_>,
        _basename: &str,
        _kind: TargetKind,
        _flags: AtFlags,
        _outcome: Result<(), rustix::io::Errno>,
        cancellation: &CancellationToken,
    ) -> Result<(), rustix::io::Errno> {
        #[cfg(target_os = "macos")]
        if let Some(action) = self.after_unlink_action.take() {
            action.run();
        }
        if let Some((path, bytes)) = self.recreate_after_unlink.take() {
            fs::write(path, bytes).map_err(|_| rustix::io::Errno::IO)?;
        }
        if self.cancel_after_unlink {
            cancellation.cancel();
        }
        Ok(())
    }

    fn sync_parent(
        &mut self,
        attempt: usize,
        parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        assert_eq!(attempt, self.sync_attempts.len());
        self.sync_attempts.push(attempt);
        match self.sync_script {
            SyncScript::Error(error) => Err(error),
            SyncScript::InterruptionsThenNative(interruptions) if attempt < interruptions => {
                Err(rustix::io::Errno::INTR)
            }
            SyncScript::Native | SyncScript::InterruptionsThenNative(_) => {
                rustix::fs::fsync(parent)
            }
            SyncScript::AlwaysInterrupted => Err(rustix::io::Errno::INTR),
        }
    }
}

#[cfg(target_os = "macos")]
struct CancelMacosDiagnosticEvidence {
    cancellation: CancellationToken,
    revalidation_target_before_calls: usize,
    revalidation_target_after_calls: usize,
    revalidation_target_statat_calls: usize,
    unlink_calls: usize,
    sync_calls: usize,
}

#[cfg(target_os = "macos")]
impl CancelMacosDiagnosticEvidence {
    const fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            revalidation_target_before_calls: 0,
            revalidation_target_after_calls: 0,
            revalidation_target_statat_calls: 0,
            unlink_calls: 0,
            sync_calls: 0,
        }
    }
}

#[cfg(target_os = "macos")]
impl DeleteFileEvidence for CancelMacosDiagnosticEvidence {
    fn checkpoint(&mut self, checkpoint: DeleteCheckpoint, _cancellation: &CancellationToken) {
        if matches!(
            checkpoint,
            DeleteCheckpoint::BeforeStatat(DeletePhase::Revalidate, StatatSite::Target, _)
        ) {
            self.revalidation_target_before_calls += 1;
        }
        if matches!(
            checkpoint,
            DeleteCheckpoint::AfterStatat(DeletePhase::Revalidate, StatatSite::Target, _)
        ) {
            self.revalidation_target_after_calls += 1;
        }
    }

    fn statat(
        &mut self,
        phase: DeletePhase,
        site: StatatSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        if phase == DeletePhase::Revalidate && site == StatatSite::Target {
            self.revalidation_target_statat_calls += 1;
            if self.revalidation_target_statat_calls == 2 {
                let _ = self.cancellation.cancel();
                return Err(rustix::io::Errno::IO);
            }
        }
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn unlink(
        &mut self,
        _parent: BorrowedFd<'_>,
        _basename: &str,
        flags: AtFlags,
    ) -> Result<(), rustix::io::Errno> {
        self.unlink_calls += 1;
        assert_eq!(flags, AtFlags::empty());
        Err(rustix::io::Errno::PERM)
    }

    fn sync_parent(
        &mut self,
        _attempt: usize,
        _parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.sync_calls += 1;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
struct MacosDiagnosticErrorEvidence {
    error: rustix::io::Errno,
    revalidation_target_statat_calls: usize,
    unlink_calls: usize,
    sync_calls: usize,
}

#[cfg(target_os = "macos")]
impl MacosDiagnosticErrorEvidence {
    const fn new(error: rustix::io::Errno) -> Self {
        Self {
            error,
            revalidation_target_statat_calls: 0,
            unlink_calls: 0,
            sync_calls: 0,
        }
    }
}

#[cfg(target_os = "macos")]
impl DeleteFileEvidence for MacosDiagnosticErrorEvidence {
    fn statat(
        &mut self,
        phase: DeletePhase,
        site: StatatSite,
        _ordinal: usize,
        parent: BorrowedFd<'_>,
        name: &OsStr,
    ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
        if phase == DeletePhase::Revalidate && site == StatatSite::Target {
            self.revalidation_target_statat_calls += 1;
            if self.revalidation_target_statat_calls == 2 {
                return Err(self.error);
            }
        }
        rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn unlink(
        &mut self,
        _parent: BorrowedFd<'_>,
        _basename: &str,
        flags: AtFlags,
    ) -> Result<(), rustix::io::Errno> {
        self.unlink_calls += 1;
        assert_eq!(flags, AtFlags::empty());
        Err(rustix::io::Errno::PERM)
    }

    fn sync_parent(
        &mut self,
        _attempt: usize,
        _parent: BorrowedFd<'_>,
    ) -> Result<(), rustix::io::Errno> {
        self.sync_calls += 1;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ExpectedUnlinkError {
    PermissionDenied,
    TargetChanged,
    DeleteFailed,
    DirectoryNotEmpty,
}

impl ExpectedUnlinkError {
    const fn details(self) -> (ToolErrorKind, &'static str, &'static str, bool) {
        match self {
            Self::PermissionDenied => (
                ToolErrorKind::PermissionDenied,
                "delete_file_permission_denied",
                "requested path cannot be deleted",
                false,
            ),
            Self::TargetChanged => (
                ToolErrorKind::Execution,
                "delete_file_target_changed",
                "requested path changed before deletion",
                true,
            ),
            Self::DeleteFailed => (
                ToolErrorKind::Execution,
                "delete_file_delete_failed",
                "requested path could not be deleted",
                true,
            ),
            Self::DirectoryNotEmpty => (
                ToolErrorKind::Execution,
                "delete_file_directory_not_empty",
                "requested directory is not empty",
                false,
            ),
        }
    }
}

struct UnlinkErrorCase {
    label: &'static str,
    errno: rustix::io::Errno,
    directory: bool,
    expected: ExpectedUnlinkError,
}

fn assert_success(output: &ToolOutput, path: &str) {
    assert!(!output.is_error);
    assert_eq!(output.content, json!({ "path": path }));
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
}

fn assert_ambiguous(error: &ToolError) {
    assert_error(
        error,
        ToolErrorKind::Execution,
        "delete_file_commit_ambiguous",
        "requested path deletion status is uncertain",
        false,
    );
}

#[test]
fn final_window_same_type_replacement_is_the_entry_deleted_for_files_and_directories() {
    let temporary = TempDirectory::new("same-type");
    let file_target = temporary.path().join("file");
    let displaced_file = temporary.path().join("file-original");
    fs::write(&file_target, b"original file").unwrap();
    let mut file_evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native)
        .with_final_action(FinalAction::ReplaceFile {
            target: file_target.clone(),
            displaced: displaced_file.clone(),
            replacement: b"replacement file".to_vec(),
        });
    let output = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("file", &CancellationToken::new(), &mut file_evidence)
        .unwrap();
    assert_success(&output, "file");
    assert!(!file_target.exists());
    assert_eq!(fs::read(displaced_file).unwrap(), b"original file");
    assert_eq!(file_evidence.final_pre_unlink_calls, 1);
    assert_eq!(file_evidence.unlink_calls, 1);
    assert_eq!(file_evidence.unlink_flags, [AtFlags::empty()]);

    let directory_target = temporary.path().join("directory");
    let displaced_directory = temporary.path().join("directory-original");
    fs::create_dir(&directory_target).unwrap();
    let mut directory_evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native)
        .with_final_action(FinalAction::ReplaceDirectory {
            target: directory_target.clone(),
            displaced: displaced_directory.clone(),
        });
    let output = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence(
            "directory",
            &CancellationToken::new(),
            &mut directory_evidence,
        )
        .unwrap();
    assert_success(&output, "directory");
    assert!(!directory_target.exists());
    assert!(displaced_directory.is_dir());
    assert_eq!(directory_evidence.final_pre_unlink_calls, 1);
    assert_eq!(directory_evidence.unlink_calls, 1);
    assert_eq!(directory_evidence.unlink_flags, [AtFlags::REMOVEDIR]);
}

#[test]
fn final_window_file_to_symlink_removes_only_the_link_and_preserves_its_referent() {
    let temporary = TempDirectory::new("symlink-replacement");
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let target = workspace.join("target");
    let displaced = workspace.join("target-original");
    let sentinel = temporary.path().join("external-sentinel");
    fs::write(&target, b"original").unwrap();
    fs::write(&sentinel, b"external sentinel").unwrap();
    let mut evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native)
        .with_final_action(FinalAction::ReplaceWithSymlink {
            target: target.clone(),
            displaced: displaced.clone(),
            referent: sentinel.clone(),
        });

    let output = DeleteFileTool::open(&workspace)
        .unwrap()
        .execute_supported_with_evidence("target", &CancellationToken::new(), &mut evidence)
        .unwrap();

    assert_success(&output, "target");
    assert!(fs::symlink_metadata(&target).is_err());
    assert_eq!(fs::read(displaced).unwrap(), b"original");
    assert_eq!(fs::read(sentinel).unwrap(), b"external sentinel");
    assert_eq!(evidence.unlink_calls, 1);
    assert_eq!(evidence.unlink_flags, [AtFlags::empty()]);
}

#[test]
fn final_window_file_to_fifo_and_socket_removes_only_the_replacement_entries() {
    let temporary = TempDirectory::new("sp");
    let sibling_sentinel = temporary.path().join("sentinel");
    fs::write(&sibling_sentinel, b"sibling sentinel").unwrap();

    for (name, action) in [
        {
            let target = temporary.path().join("f");
            let displaced = temporary.path().join("fo");
            fs::write(&target, b"fifo original").unwrap();
            ("f", FinalAction::ReplaceWithFifo { target, displaced })
        },
        {
            let target = temporary.path().join("s");
            let displaced = temporary.path().join("so");
            fs::write(&target, b"socket original").unwrap();
            ("s", FinalAction::ReplaceWithSocket { target, displaced })
        },
    ] {
        let displaced = match &action {
            FinalAction::ReplaceWithFifo { displaced, .. }
            | FinalAction::ReplaceWithSocket { displaced, .. } => displaced.clone(),
            _ => unreachable!(),
        };
        let expected_original = if name == "f" {
            b"fifo original".as_slice()
        } else {
            b"socket original".as_slice()
        };
        let mut evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native)
            .with_final_action(action);

        let output = DeleteFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence(name, &CancellationToken::new(), &mut evidence)
            .unwrap();

        assert_success(&output, name);
        assert!(fs::symlink_metadata(temporary.path().join(name)).is_err());
        assert_eq!(fs::read(displaced).unwrap(), expected_original);
        assert_eq!(fs::read(&sibling_sentinel).unwrap(), b"sibling sentinel");
        assert_eq!(evidence.final_pre_unlink_calls, 1);
        assert_eq!(evidence.unlink_calls, 1);
        assert_eq!(evidence.unlink_flags, [AtFlags::empty()]);
        assert_eq!(evidence.sync_attempts, [0]);
    }
}

#[test]
fn final_window_type_changes_report_target_changed_and_preserve_replacements() {
    let temporary = TempDirectory::new("type-change");
    let file_target = temporary.path().join("file");
    let displaced_file = temporary.path().join("file-original");
    fs::write(&file_target, b"original file").unwrap();
    let mut file_evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native)
        .with_final_action(FinalAction::ReplaceFileWithDirectory {
            target: file_target.clone(),
            displaced: displaced_file.clone(),
        });
    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("file", &CancellationToken::new(), &mut file_evidence)
        .unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Execution,
        "delete_file_target_changed",
        "requested path changed before deletion",
        true,
    );
    assert!(file_target.is_dir());
    assert_eq!(fs::read(displaced_file).unwrap(), b"original file");
    assert_eq!(file_evidence.unlink_calls, 1);
    assert!(file_evidence.sync_attempts.is_empty());

    let directory_target = temporary.path().join("directory");
    let displaced_directory = temporary.path().join("directory-original");
    fs::create_dir(&directory_target).unwrap();
    let mut directory_evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native)
        .with_final_action(FinalAction::ReplaceDirectoryWithFile {
            target: directory_target.clone(),
            displaced: displaced_directory.clone(),
            replacement: b"replacement file".to_vec(),
        });
    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence(
            "directory",
            &CancellationToken::new(),
            &mut directory_evidence,
        )
        .unwrap_err();
    assert_error(
        &error,
        ToolErrorKind::Execution,
        "delete_file_target_changed",
        "requested path changed before deletion",
        true,
    );
    assert_eq!(fs::read(&directory_target).unwrap(), b"replacement file");
    assert!(displaced_directory.is_dir());
    assert_eq!(directory_evidence.unlink_calls, 1);
    assert!(directory_evidence.sync_attempts.is_empty());
}

#[test]
fn retained_parent_moved_outside_public_path_receives_only_the_intended_delete() {
    let temporary = TempDirectory::new("moved-parent");
    let workspace = temporary.path().join("workspace");
    let original_parent = workspace.join("nested");
    let moved_parent = temporary.path().join("moved-outside");
    fs::create_dir_all(&original_parent).unwrap();
    fs::write(original_parent.join("target"), b"retained target").unwrap();
    fs::write(original_parent.join("sentinel"), b"retained sentinel").unwrap();
    let mut evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native)
        .with_final_action(FinalAction::MoveParent {
            original_parent: original_parent.clone(),
            moved_parent: moved_parent.clone(),
            replacement: b"public replacement".to_vec(),
        });

    let output = DeleteFileTool::open(&workspace)
        .unwrap()
        .execute_supported_with_evidence("nested/target", &CancellationToken::new(), &mut evidence)
        .unwrap();

    assert_success(&output, "nested/target");
    assert!(!moved_parent.join("target").exists());
    assert_eq!(
        fs::read(moved_parent.join("sentinel")).unwrap(),
        b"retained sentinel"
    );
    assert_eq!(
        fs::read(original_parent.join("target")).unwrap(),
        b"public replacement"
    );
    assert_eq!(evidence.final_pre_unlink_calls, 1);
    assert_eq!(evidence.unlink_calls, 1);
    assert_eq!(evidence.sync_attempts, [0]);
}

#[test]
fn unlink_receives_exact_file_and_directory_flags() {
    let temporary = TempDirectory::new("exact-flags");
    for (name, directory, expected_flags) in [
        ("file", false, AtFlags::empty()),
        ("directory", true, AtFlags::REMOVEDIR),
    ] {
        let target = temporary.path().join(name);
        if directory {
            fs::create_dir(&target).unwrap();
        } else {
            fs::write(&target, b"contents").unwrap();
        }
        let mut evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native);
        let output = DeleteFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence(name, &CancellationToken::new(), &mut evidence)
            .unwrap();
        assert_success(&output, name);
        assert_eq!(evidence.unlink_calls, 1);
        assert_eq!(evidence.unlink_flags, [expected_flags]);
        assert_eq!(evidence.sync_attempts, [0]);
    }
}

#[test]
fn definitive_unlink_errors_have_exact_mapping_one_call_and_no_sync() {
    let cases = [
        UnlinkErrorCase {
            label: "eacces",
            errno: rustix::io::Errno::ACCESS,
            directory: false,
            expected: ExpectedUnlinkError::PermissionDenied,
        },
        UnlinkErrorCase {
            label: "eperm",
            errno: rustix::io::Errno::PERM,
            directory: false,
            expected: ExpectedUnlinkError::PermissionDenied,
        },
        UnlinkErrorCase {
            label: "erofs",
            errno: rustix::io::Errno::ROFS,
            directory: false,
            expected: ExpectedUnlinkError::PermissionDenied,
        },
        UnlinkErrorCase {
            label: "enoent",
            errno: rustix::io::Errno::NOENT,
            directory: false,
            expected: ExpectedUnlinkError::TargetChanged,
        },
        UnlinkErrorCase {
            label: "enotdir",
            errno: rustix::io::Errno::NOTDIR,
            directory: false,
            expected: ExpectedUnlinkError::TargetChanged,
        },
        UnlinkErrorCase {
            label: "eisdir",
            errno: rustix::io::Errno::ISDIR,
            directory: false,
            expected: ExpectedUnlinkError::TargetChanged,
        },
        UnlinkErrorCase {
            label: "eloop",
            errno: rustix::io::Errno::LOOP,
            directory: false,
            expected: ExpectedUnlinkError::TargetChanged,
        },
        UnlinkErrorCase {
            label: "eio",
            errno: rustix::io::Errno::IO,
            directory: false,
            expected: ExpectedUnlinkError::DeleteFailed,
        },
        UnlinkErrorCase {
            label: "enotempty",
            errno: rustix::io::Errno::NOTEMPTY,
            directory: true,
            expected: ExpectedUnlinkError::DirectoryNotEmpty,
        },
        UnlinkErrorCase {
            label: "eexist",
            errno: rustix::io::Errno::EXIST,
            directory: true,
            expected: ExpectedUnlinkError::DirectoryNotEmpty,
        },
    ];

    for case in cases {
        let temporary = TempDirectory::new(case.label);
        let target = temporary.path().join("target");
        if case.directory {
            fs::create_dir(&target).unwrap();
        } else {
            fs::write(&target, b"contents").unwrap();
        }
        let mut evidence =
            ScriptedEvidence::new(UnlinkScript::Error(case.errno), SyncScript::Native);
        let error = DeleteFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence("target", &CancellationToken::new(), &mut evidence)
            .unwrap_err();
        let (kind, code, message, retryable) = case.expected.details();
        assert_error(&error, kind, code, message, retryable);
        assert_eq!(evidence.unlink_calls, 1);
        assert_eq!(
            evidence.unlink_flags,
            [if case.directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            }],
        );
        assert!(evidence.sync_attempts.is_empty());
        assert!(target.exists());
    }
}

#[test]
fn post_unlink_cancellation_wins_every_definitive_error_without_syncing() {
    let cases = [
        ("cancel-eio", rustix::io::Errno::IO, false),
        ("cancel-eacces", rustix::io::Errno::ACCESS, false),
        ("cancel-eperm", rustix::io::Errno::PERM, false),
        ("cancel-erofs", rustix::io::Errno::ROFS, false),
        ("cancel-enoent", rustix::io::Errno::NOENT, false),
        ("cancel-enotdir", rustix::io::Errno::NOTDIR, false),
        ("cancel-eisdir", rustix::io::Errno::ISDIR, false),
        ("cancel-eloop", rustix::io::Errno::LOOP, false),
        ("cancel-enotempty", rustix::io::Errno::NOTEMPTY, true),
        ("cancel-eexist", rustix::io::Errno::EXIST, true),
    ];

    for (label, errno, directory) in cases {
        let temporary = TempDirectory::new(label);
        let target = temporary.path().join("target");
        if directory {
            fs::create_dir(&target).unwrap();
            fs::write(target.join("sentinel"), b"retained directory contents").unwrap();
        } else {
            fs::write(&target, b"retained file contents").unwrap();
        }
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(UnlinkScript::Error(errno), SyncScript::Native);
        evidence.cancel_after_unlink = true;

        let error = DeleteFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence("target", &cancellation, &mut evidence)
            .unwrap_err();

        assert_error(
            &error,
            ToolErrorKind::Cancelled,
            "delete_file_cancelled",
            "delete_file execution was cancelled",
            false,
        );
        assert!(cancellation.is_cancelled());
        assert_eq!(evidence.unlink_calls, 1);
        assert_eq!(
            evidence.unlink_flags,
            [if directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            }],
        );
        assert!(evidence.sync_attempts.is_empty());
        if directory {
            assert!(target.is_dir());
            assert_eq!(
                fs::read(target.join("sentinel")).unwrap(),
                b"retained directory contents"
            );
        } else {
            assert_eq!(fs::read(target).unwrap(), b"retained file contents");
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_eperm_diagnostic_rejects_every_post_unlink_identity_change() {
    for replacement_kind in 0_u8..5 {
        let temporary = TempDirectory::new(&format!("mi{replacement_kind}"));
        let target = temporary.path().join("t");
        let displaced = temporary.path().join("o");
        let referent = temporary.path().join("r");
        let unrelated = temporary.path().join("u");
        fs::write(&target, b"validated original").unwrap();
        fs::write(&referent, b"referent sentinel").unwrap();
        fs::write(&unrelated, b"unrelated sentinel").unwrap();

        let after_unlink_action = match replacement_kind {
            0 => MacosAfterUnlinkAction::RemoveDirectory(target.clone()),
            1 => MacosAfterUnlinkAction::ReplaceDirectoryWithSymlink {
                target: target.clone(),
                referent: referent.clone(),
            },
            2 => MacosAfterUnlinkAction::ReplaceDirectoryWithFifo(target.clone()),
            3 => MacosAfterUnlinkAction::ReplaceDirectoryWithSocket(target.clone()),
            4 => MacosAfterUnlinkAction::ReplaceDirectoryWithFile(target.clone()),
            _ => unreachable!(),
        };
        let mut evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native)
            .with_final_action(FinalAction::ReplaceFileWithDirectory {
                target: target.clone(),
                displaced: displaced.clone(),
            })
            .with_after_unlink_action(after_unlink_action);

        let error = DeleteFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence("t", &CancellationToken::new(), &mut evidence)
            .unwrap_err();

        assert_error(
            &error,
            ToolErrorKind::Execution,
            "delete_file_target_changed",
            "requested path changed before deletion",
            true,
        );
        assert_eq!(evidence.unlink_calls, 1);
        assert_eq!(evidence.unlink_flags, [AtFlags::empty()]);
        assert!(evidence.sync_attempts.is_empty());
        assert_eq!(fs::read(&displaced).unwrap(), b"validated original");
        assert_eq!(fs::read(&referent).unwrap(), b"referent sentinel");
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated sentinel");
        match replacement_kind {
            0 => assert!(fs::symlink_metadata(&target).is_err()),
            1 => assert!(
                fs::symlink_metadata(&target)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            ),
            2 => assert!(fs::symlink_metadata(&target).unwrap().file_type().is_fifo()),
            3 => assert!(
                fs::symlink_metadata(&target)
                    .unwrap()
                    .file_type()
                    .is_socket()
            ),
            4 => assert_eq!(fs::read(&target).unwrap(), b"different regular file"),
            _ => unreachable!(),
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_eperm_diagnostic_errno_precedence_is_exact() {
    for (index, (diagnostic_error, expected)) in [
        (rustix::io::Errno::NOENT, ExpectedUnlinkError::TargetChanged),
        (
            rustix::io::Errno::NOTDIR,
            ExpectedUnlinkError::TargetChanged,
        ),
        (rustix::io::Errno::ISDIR, ExpectedUnlinkError::TargetChanged),
        (rustix::io::Errno::LOOP, ExpectedUnlinkError::TargetChanged),
        (
            rustix::io::Errno::ACCESS,
            ExpectedUnlinkError::PermissionDenied,
        ),
        (
            rustix::io::Errno::PERM,
            ExpectedUnlinkError::PermissionDenied,
        ),
        (rustix::io::Errno::IO, ExpectedUnlinkError::PermissionDenied),
    ]
    .into_iter()
    .enumerate()
    {
        let temporary = TempDirectory::new(&format!("me{index}"));
        let target = temporary.path().join("t");
        fs::write(&target, b"unchanged regular file").unwrap();
        let mut evidence = MacosDiagnosticErrorEvidence::new(diagnostic_error);

        let error = DeleteFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence("t", &CancellationToken::new(), &mut evidence)
            .unwrap_err();

        let (kind, code, message, retryable) = expected.details();
        assert_error(&error, kind, code, message, retryable);
        assert_eq!(evidence.revalidation_target_statat_calls, 2);
        assert_eq!(evidence.unlink_calls, 1);
        assert_eq!(evidence.sync_calls, 0);
        assert_eq!(fs::read(&target).unwrap(), b"unchanged regular file");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_eperm_diagnostic_preserves_permission_for_an_unchanged_regular_file() {
    let temporary = TempDirectory::new("unchanged");
    let target = temporary.path().join("t");
    fs::write(&target, b"unchanged regular file").unwrap();
    let mut evidence = ScriptedEvidence::new(
        UnlinkScript::Error(rustix::io::Errno::PERM),
        SyncScript::Native,
    );

    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("t", &CancellationToken::new(), &mut evidence)
        .unwrap_err();

    let (kind, code, message, retryable) = ExpectedUnlinkError::PermissionDenied.details();
    assert_error(&error, kind, code, message, retryable);
    assert_eq!(evidence.unlink_calls, 1);
    assert_eq!(evidence.unlink_flags, [AtFlags::empty()]);
    assert!(evidence.sync_attempts.is_empty());
    assert_eq!(fs::read(&target).unwrap(), b"unchanged regular file");
}

#[cfg(target_os = "macos")]
#[test]
fn macos_file_eperm_diagnostic_error_honors_cancellation_from_a_token_clone() {
    let temporary = TempDirectory::new("macos-eperm-diagnostic-cancel");
    let target = temporary.path().join("target");
    fs::write(&target, b"original bytes").unwrap();
    let cancellation = CancellationToken::new();
    let mut evidence = CancelMacosDiagnosticEvidence::new(cancellation.clone());

    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &cancellation, &mut evidence)
        .unwrap_err();

    assert_error(
        &error,
        ToolErrorKind::Cancelled,
        "delete_file_cancelled",
        "delete_file execution was cancelled",
        false,
    );
    assert!(cancellation.is_cancelled());
    assert_eq!(evidence.revalidation_target_before_calls, 2);
    assert_eq!(evidence.revalidation_target_after_calls, 2);
    assert_eq!(evidence.revalidation_target_statat_calls, 2);
    assert_eq!(evidence.unlink_calls, 1);
    assert_eq!(evidence.sync_calls, 0);
    assert_eq!(fs::read(target).unwrap(), b"original bytes");
}

#[test]
fn interrupted_unlink_is_never_retried_and_syncs_for_both_possible_commit_states() {
    let temporary = TempDirectory::new("eintr-no-delete");
    let target = temporary.path().join("target");
    fs::write(&target, b"contents").unwrap();
    let mut no_delete = ScriptedEvidence::new(
        UnlinkScript::Error(rustix::io::Errno::INTR),
        SyncScript::Native,
    );
    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &CancellationToken::new(), &mut no_delete)
        .unwrap_err();
    assert_ambiguous(&error);
    assert_eq!(no_delete.unlink_calls, 1);
    assert_eq!(no_delete.sync_attempts, [0]);
    assert_eq!(fs::read(&target).unwrap(), b"contents");

    let temporary = TempDirectory::new("eintr-after-delete");
    let target = temporary.path().join("target");
    fs::write(&target, b"contents").unwrap();
    let mut after_delete = ScriptedEvidence::new(
        UnlinkScript::NativeThenError(rustix::io::Errno::INTR),
        SyncScript::Native,
    );
    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &CancellationToken::new(), &mut after_delete)
        .unwrap_err();
    assert_ambiguous(&error);
    assert_eq!(after_delete.unlink_calls, 1);
    assert_eq!(after_delete.sync_attempts, [0]);
    assert!(!target.exists());
}

#[test]
fn successful_unlink_followed_by_sync_failure_is_absent_and_ambiguous() {
    let temporary = TempDirectory::new("sync-failure");
    let target = temporary.path().join("target");
    fs::write(&target, b"contents").unwrap();
    let mut evidence = ScriptedEvidence::new(
        UnlinkScript::Native,
        SyncScript::Error(rustix::io::Errno::IO),
    );

    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &CancellationToken::new(), &mut evidence)
        .unwrap_err();

    assert_ambiguous(&error);
    assert!(!target.exists());
    assert_eq!(evidence.unlink_calls, 1);
    assert_eq!(evidence.sync_attempts, [0]);
}

#[test]
fn parent_sync_allows_fifteen_interruptions_and_bounds_the_sixteenth() {
    let temporary = TempDirectory::new("sync-fifteen");
    let target = temporary.path().join("target");
    fs::write(&target, b"contents").unwrap();
    let mut succeeds = ScriptedEvidence::new(
        UnlinkScript::Native,
        SyncScript::InterruptionsThenNative(15),
    );
    let output = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &CancellationToken::new(), &mut succeeds)
        .unwrap();
    assert_success(&output, "target");
    assert_eq!(succeeds.unlink_calls, 1);
    assert_eq!(succeeds.sync_attempts, (0..16).collect::<Vec<_>>());
    assert!(!target.exists());

    let temporary = TempDirectory::new("sync-sixteen");
    let target = temporary.path().join("target");
    fs::write(&target, b"contents").unwrap();
    let mut exhausted = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::AlwaysInterrupted);
    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &CancellationToken::new(), &mut exhausted)
        .unwrap_err();
    assert_ambiguous(&error);
    assert_eq!(exhausted.unlink_calls, 1);
    assert_eq!(exhausted.sync_attempts, (0..16).collect::<Vec<_>>());
    assert!(!target.exists());

    let temporary = TempDirectory::new("ambiguous-unlink-precedence");
    let target = temporary.path().join("target");
    fs::write(&target, b"contents").unwrap();
    let mut ambiguous = ScriptedEvidence::new(
        UnlinkScript::NativeThenError(rustix::io::Errno::INTR),
        SyncScript::AlwaysInterrupted,
    );
    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &CancellationToken::new(), &mut ambiguous)
        .unwrap_err();
    assert_ambiguous(&error);
    assert_eq!(ambiguous.unlink_calls, 1);
    assert_eq!(ambiguous.sync_attempts, (0..16).collect::<Vec<_>>());
    assert!(!target.exists());
}

#[test]
fn post_unlink_cancellation_is_ignored_for_success_and_interruption_ambiguity() {
    let temporary = TempDirectory::new("post-unlink-success-cancel");
    let target = temporary.path().join("target");
    fs::write(&target, b"contents").unwrap();
    let cancellation = CancellationToken::new();
    let mut success = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native);
    success.cancel_after_unlink = true;
    let output = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &cancellation, &mut success)
        .unwrap();
    assert_success(&output, "target");
    assert!(cancellation.is_cancelled());
    assert_eq!(success.unlink_calls, 1);
    assert_eq!(success.sync_attempts, [0]);
    assert!(!target.exists());

    let temporary = TempDirectory::new("post-unlink-eintr-cancel");
    let target = temporary.path().join("target");
    fs::write(&target, b"contents").unwrap();
    let cancellation = CancellationToken::new();
    let mut interrupted = ScriptedEvidence::new(
        UnlinkScript::Error(rustix::io::Errno::INTR),
        SyncScript::Native,
    );
    interrupted.cancel_after_unlink = true;
    let error = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &cancellation, &mut interrupted)
        .unwrap_err();
    assert_ambiguous(&error);
    assert!(cancellation.is_cancelled());
    assert_eq!(interrupted.unlink_calls, 1);
    assert_eq!(interrupted.sync_attempts, [0]);
    assert_eq!(fs::read(target).unwrap(), b"contents");
}

#[test]
fn immediate_recreation_can_leave_the_path_present_after_reported_success() {
    let temporary = TempDirectory::new("immediate-recreate");
    let target = temporary.path().join("target");
    fs::write(&target, b"original").unwrap();
    let mut evidence = ScriptedEvidence::new(UnlinkScript::Native, SyncScript::Native);
    evidence.recreate_after_unlink = Some((target.clone(), b"recreated".to_vec()));

    let output = DeleteFileTool::open(temporary.path())
        .unwrap()
        .execute_supported_with_evidence("target", &CancellationToken::new(), &mut evidence)
        .unwrap();

    assert_success(&output, "target");
    assert_eq!(fs::read(target).unwrap(), b"recreated");
    assert_eq!(evidence.unlink_calls, 1);
    assert_eq!(evidence.sync_attempts, [0]);
}

#[cfg(feature = "ai-gateway-http")]
#[test]
fn workspace_composition_uses_one_original_descriptor_and_sixteen_identity_clones() {
    use crate::workspace::{WorkspaceRoot, WorkspaceRootError};

    let temporary = TempDirectory::new("workspace-clones");
    let root = WorkspaceRoot::open(temporary.path())
        .unwrap_or_else(|_| panic!("open workspace root for clone evidence"));
    let original_metadata = rustix::fs::fstat(root.descriptor()).unwrap();
    let original_identity = (
        i128::from(original_metadata.st_dev),
        i128::from(original_metadata.st_ino),
    );
    let mut clone_identities = Vec::new();
    let tools = root
        .into_tools_with_clone(|descriptor| {
            let descriptor_metadata = rustix::fs::fstat(descriptor).unwrap();
            assert_eq!(
                (
                    i128::from(descriptor_metadata.st_dev),
                    i128::from(descriptor_metadata.st_ino)
                ),
                original_identity
            );
            let clone = descriptor.try_clone().map_err(|_| WorkspaceRootError)?;
            let clone_metadata = rustix::fs::fstat(&clone).unwrap();
            clone_identities.push((
                i128::from(clone_metadata.st_dev),
                i128::from(clone_metadata.st_ino),
            ));
            Ok(clone)
        })
        .unwrap_or_else(|_| panic!("compose workspace tools for clone evidence"));

    assert_eq!(clone_identities, vec![original_identity; 16]);
    let names = [
        tools.copy_file.spec().name,
        tools.create_folder.spec().name,
        tools.delete_file.spec().name,
        tools.edit_file.spec().name,
        tools.file_info.spec().name,
        tools.glob_files.spec().name,
        tools.grep_files.spec().name,
        tools.install_skill.spec().name,
        tools.list_files.spec().name,
        tools.open_file.spec().name,
        tools.read_file.spec().name,
        tools.rename_file.spec().name,
        tools.skill.spec().name,
        tools.write_file.spec().name,
    ];
    assert_eq!(
        names.map(|name| name.to_string()),
        [
            "copy_file".to_owned(),
            "create_folder".to_owned(),
            "delete_file".to_owned(),
            "edit_file".to_owned(),
            "file_info".to_owned(),
            "glob_files".to_owned(),
            "grep_files".to_owned(),
            "install_skill".to_owned(),
            "list_files".to_owned(),
            "open_file".to_owned(),
            "read_file".to_owned(),
            "rename_file".to_owned(),
            "skill".to_owned(),
            "write_file".to_owned(),
        ]
    );
}
