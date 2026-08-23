#![cfg(any(target_os = "linux", target_os = "macos"))]

use super::*;

#[test]
fn unique_match_budget_accepts_exact_steps_and_rejects_one_fewer() {
    let cancellation = CancellationToken::new();
    let mut batches = Vec::new();
    assert_eq!(
        find_unique_match_with_budget(b"abc", b"b", &cancellation, usize::MAX, |batch| {
            batches.push(batch);
        })
        .unwrap(),
        1
    );
    assert!(batches.iter().all(|batch| *batch <= 1_024));
    let exact_steps = batches.iter().sum::<usize>();
    assert_eq!(exact_steps, 4);
    assert_eq!(
        find_unique_match_with_budget(b"abc", b"b", &cancellation, exact_steps, |_| {}).unwrap(),
        1
    );
    assert_eq!(
        find_unique_match_with_budget(b"abc", b"b", &cancellation, exact_steps - 1, |_| {})
            .unwrap_err()
            .code,
        "edit_file_match_work_exceeded"
    );
}

#[test]
fn unique_match_is_overlap_aware_and_reports_only_fixed_outcomes() {
    let cancellation = CancellationToken::new();
    assert_eq!(
        find_unique_match_with_budget(
            "prefix-λ\0-suffix".as_bytes(),
            "λ\0".as_bytes(),
            &cancellation,
            1_000,
            |_| {},
        )
        .unwrap(),
        7
    );
    assert_eq!(
        find_unique_match_with_budget(b"abcdef", b"missing", &cancellation, 1_000, |_| {})
            .unwrap_err()
            .code,
        "edit_file_match_not_found"
    );
    for (preimage, pattern) in [(b"old-old".as_slice(), b"old".as_slice()), (b"aaa", b"aa")] {
        assert_eq!(
            find_unique_match_with_budget(preimage, pattern, &cancellation, 1_000, |_| {})
                .unwrap_err()
                .code,
            "edit_file_match_ambiguous"
        );
    }
}

#[test]
fn worst_case_legal_match_work_is_linear_and_below_the_public_ceiling() {
    let mut pattern = vec![b'a'; MAX_EDIT_FILE_OLD_STRING_BYTES / 2];
    *pattern.last_mut().unwrap() = b'b';
    let preimage = vec![b'a'; MAX_EDIT_FILE_EXISTING_BYTES];
    let mut charged = 0_usize;
    let error = find_unique_match_with_budget(
        &preimage,
        &pattern,
        &CancellationToken::new(),
        MAX_EDIT_FILE_MATCH_WORK_STEPS,
        |batch| {
            assert!(batch <= 1_024);
            charged += batch;
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "edit_file_match_not_found");
    assert!(charged < MAX_EDIT_FILE_MATCH_WORK_STEPS);
    assert!(charged <= 3 * (preimage.len() + pattern.len()));
}

#[test]
fn matching_checks_cancellation_at_no_more_than_1024_charged_step_intervals() {
    let cancellation = CancellationToken::new();
    let cancellation_from_batch = cancellation.clone();
    let mut batches = Vec::new();
    let error = find_unique_match_with_budget(
        &vec![b'a'; 10_000],
        b"b",
        &cancellation,
        MAX_EDIT_FILE_MATCH_WORK_STEPS,
        |batch| {
            batches.push(batch);
            cancellation_from_batch.cancel();
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "edit_file_cancelled");
    assert_eq!(batches, [1_024]);
}

#[test]
fn postimage_construction_accepts_exact_cap_rejects_one_over_and_batches_copies() {
    let preimage = format!("{}OLD{}", "a".repeat(10_000), "b".repeat(10_000));
    let replacement = "λ".repeat(2_000);
    let result_len = 10_000 + replacement.len() + 10_000;
    let mut batches = Vec::new();
    let result = build_postimage_with_budget(
        preimage.as_bytes(),
        10_000,
        3,
        replacement.as_bytes(),
        &CancellationToken::new(),
        result_len,
        |batch| batches.push(batch),
    )
    .unwrap();
    assert_eq!(result.len(), result_len);
    assert!(
        batches
            .iter()
            .all(|batch| *batch <= MAX_EDIT_FILE_CHUNK_BYTES)
    );
    assert_eq!(batches.iter().sum::<usize>(), result_len);
    assert_eq!(
        build_postimage_with_budget(
            preimage.as_bytes(),
            10_000,
            3,
            replacement.as_bytes(),
            &CancellationToken::new(),
            result_len - 1,
            |_| {},
        )
        .unwrap_err()
        .code,
        "edit_file_result_too_large"
    );
}

#[test]
fn postimage_construction_cancellation_wins_after_a_partial_batch() {
    let cancellation = CancellationToken::new();
    let cancellation_from_batch = cancellation.clone();
    let mut batches = Vec::new();
    let error = build_postimage_with_budget(
        &vec![b'a'; 20_000],
        19_999,
        1,
        b"replacement",
        &cancellation,
        MAX_EDIT_FILE_RESULTING_BYTES,
        |batch| {
            batches.push(batch);
            cancellation_from_batch.cancel();
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "edit_file_cancelled");
    assert_eq!(batches, [MAX_EDIT_FILE_CHUNK_BYTES]);
}

#[test]
fn serialized_success_guard_accepts_exact_size_and_rejects_one_under() {
    let output = build_success_output_with_limit("nested/file.txt", 123, usize::MAX).unwrap();
    let exact = serde_json::to_vec(&output).unwrap().len();
    assert_eq!(
        build_success_output_with_limit("nested/file.txt", 123, exact).unwrap(),
        output
    );
    assert_eq!(
        build_success_output_with_limit("nested/file.txt", 123, exact - 1)
            .unwrap_err()
            .code,
        "edit_file_write_failed"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod supported {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::os::fd::AsFd;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;
    use std::path::PathBuf;
    #[cfg(target_os = "macos")]
    use std::process::Command;

    use super::*;

    struct TempFile {
        path: PathBuf,
    }

    struct TempDirectory {
        path: PathBuf,
    }

    #[cfg(target_os = "macos")]
    struct MacAclCleanup(PathBuf);

    #[cfg(target_os = "macos")]
    impl Drop for MacAclCleanup {
        fn drop(&mut self) {
            let _ = Command::new("/bin/chmod").arg("-N").arg(&self.0).status();
        }
    }

    impl TempFile {
        fn new(bytes: &[u8]) -> Self {
            for suffix in 0..1_000_u64 {
                let path = std::env::temp_dir().join(format!(
                    "machine-god-edit-private-{}-{suffix}",
                    std::process::id()
                ));
                match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(mut file) => {
                        use std::io::Write as _;
                        file.write_all(bytes).unwrap();
                        return Self { path };
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create private test file: {error}"),
                }
            }
            panic!("allocate private test file")
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    impl TempDirectory {
        fn new(label: &str) -> Self {
            for suffix in 0..1_000_u64 {
                let path = std::env::temp_dir().join(format!(
                    "machine-god-edit-pipeline-{label}-{}-{suffix}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("create private test directory: {error}"),
                }
            }
            panic!("allocate private test directory")
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn descriptor(&self) -> OwnedFd {
            rustix::fs::open(self.path(), directory_open_flags(), Mode::empty()).unwrap()
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn assert_no_staged_files(root: &Path) {
        let staged = fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().starts_with(TEMP_NAME_PREFIX))
            .collect::<Vec<_>>();
        assert!(staged.is_empty(), "staged files were retained: {staged:?}");
    }

    fn overwrite_same_length_at(parent: BorrowedFd<'_>, name: &str, replacement: &[u8]) {
        let file = rustix::fs::openat(
            parent,
            name,
            OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .unwrap();
        let before = rustix::fs::fstat(&file).unwrap();
        assert_eq!(usize::try_from(before.st_size).unwrap(), replacement.len());
        write_content(file.as_fd(), replacement, &CancellationToken::new()).unwrap();
        let after = rustix::fs::fstat(&file).unwrap();
        assert!(same_identity(&before, &after));
        assert_eq!(after.st_size, before.st_size);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pipeline_clears_file_inherited_acl_before_long_staging_and_publication() {
        let temporary = TempDirectory::new("inherited-acl");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();

        let status = Command::new("/bin/chmod")
            .args(["+a", "everyone allow read,write,append,file_inherit"])
            .arg(temporary.path())
            .status()
            .expect("macOS chmod executable is available");
        assert!(
            status.success(),
            "failed to install file-inheritable ACL fixture: {status}"
        );
        let _acl_cleanup = MacAclCleanup(temporary.path().to_owned());

        let witness_path = temporary.path().join("inheritance-witness");
        fs::write(&witness_path, b"witness").unwrap();
        let witness = rustix::fs::open(
            &witness_path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let witness_acl = calcifer_macos_acl::read_acl(witness.as_fd()).unwrap();
        assert!(witness_acl.entries.iter().any(|entry| {
            entry.tag == calcifer_macos_acl::TAG_ALLOW
                && entry.flags & calcifer_macos_acl::FLAG_INHERITED != 0
        }));
        drop(witness);
        fs::remove_file(&witness_path).unwrap();

        let staged_observations = Cell::new(0_usize);
        let output = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |parent, staged_name| {
                    staged_observations.set(staged_observations.get() + 1);
                    let staged = rustix::fs::openat(
                        parent,
                        staged_name,
                        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .unwrap();
                    let metadata = rustix::fs::fstat(&staged).unwrap();
                    assert_eq!(metadata.st_mode & 0o777, 0o600);
                    let acl = calcifer_macos_acl::read_acl(staged.as_fd()).unwrap();
                    assert!(acl.is_empty(), "staged ACL was not cleared: {acl:?}");
                },
                || {},
                || {},
                native_publish_staged,
                native_sync_parent,
            )
            .unwrap();
        assert_eq!(staged_observations.get(), 1);
        assert!(!output.is_error);

        let parent_acl = calcifer_macos_acl::read_acl(temporary.descriptor().as_fd()).unwrap();
        assert!(
            parent_acl
                .entries
                .iter()
                .any(|entry| entry.tag == calcifer_macos_acl::TAG_ALLOW)
        );
        let published = rustix::fs::open(
            &target,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let published_acl = calcifer_macos_acl::read_acl(published.as_fd()).unwrap();
        assert!(
            published_acl.is_empty(),
            "published ACL was not cleared: {published_acl:?}"
        );
        assert_eq!(
            rustix::fs::fstat(&published).unwrap().st_mode & 0o777,
            0o640
        );
        assert_eq!(fs::read(&target).unwrap(), b"new content");
        assert_no_staged_files(temporary.path());
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum EvidenceOperation {
        OpenTarget,
        Pread,
        Fstat,
        Statat,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum EvidenceFault {
        Error,
        Interrupted,
        EarlyEof,
        Cancel,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct EvidenceReadFault {
        phase: ReadPhase,
        operation: EvidenceOperation,
        first_ordinal: usize,
        count: usize,
        fault: EvidenceFault,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AclReadFault {
        Error,
        Unsafe,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum EvidenceCheckpoint {
        StageError,
        StageCancel,
        FinalSyncCorrupt,
        FinalVerificationError,
        FinalVerificationCancel,
        RenameCancel,
    }

    struct ScriptedEvidence {
        cancellation: CancellationToken,
        walk_fault: Option<(WalkPhase, WalkStep, EvidenceFault)>,
        read_fault: Option<EvidenceReadFault>,
        read_operation_calls: RefCell<Vec<(ReadPhase, EvidenceOperation, usize)>>,
        clear_acl_error: bool,
        clear_acl_calls: Cell<usize>,
        acl_read_fault: Option<(ReadPhase, usize, AclReadFault)>,
        acl_read_calls: RefCell<Vec<(ReadPhase, usize)>>,
        stage_open_error: bool,
        checkpoint: Option<EvidenceCheckpoint>,
        selected_calls: Cell<usize>,
        final_stage_sync_calls: Cell<usize>,
        published_pread_calls: Cell<usize>,
    }

    impl ScriptedEvidence {
        fn new(cancellation: &CancellationToken) -> Self {
            Self {
                cancellation: cancellation.clone(),
                walk_fault: None,
                read_fault: None,
                read_operation_calls: RefCell::new(Vec::new()),
                clear_acl_error: false,
                clear_acl_calls: Cell::new(0),
                acl_read_fault: None,
                acl_read_calls: RefCell::new(Vec::new()),
                stage_open_error: false,
                checkpoint: None,
                selected_calls: Cell::new(0),
                final_stage_sync_calls: Cell::new(0),
                published_pread_calls: Cell::new(0),
            }
        }

        fn set_read_fault(
            &mut self,
            phase: ReadPhase,
            operation: EvidenceOperation,
            first_ordinal: usize,
            count: usize,
            fault: EvidenceFault,
        ) {
            assert!(first_ordinal > 0);
            assert!(count > 0);
            self.read_fault = Some(EvidenceReadFault {
                phase,
                operation,
                first_ordinal,
                count,
                fault,
            });
        }

        fn selected_read(
            &self,
            phase: ReadPhase,
            operation: EvidenceOperation,
        ) -> Option<EvidenceFault> {
            let ordinal = {
                let mut calls = self.read_operation_calls.borrow_mut();
                if let Some((_, _, count)) =
                    calls
                        .iter_mut()
                        .find(|(recorded_phase, recorded_operation, _)| {
                            *recorded_phase == phase && *recorded_operation == operation
                        })
                {
                    *count += 1;
                    *count
                } else {
                    calls.push((phase, operation, 1));
                    1
                }
            };
            self.read_fault.and_then(|selected| {
                let selected_offset = ordinal.checked_sub(selected.first_ordinal)?;
                (phase == selected.phase
                    && operation == selected.operation
                    && selected_offset < selected.count)
                    .then_some(selected.fault)
            })
        }

        fn read_call_count(&self, phase: ReadPhase, operation: EvidenceOperation) -> usize {
            self.read_operation_calls
                .borrow()
                .iter()
                .find_map(|(recorded_phase, recorded_operation, count)| {
                    (*recorded_phase == phase && *recorded_operation == operation).then_some(*count)
                })
                .unwrap_or(0)
        }

        fn acl_read_ordinal(&self, phase: ReadPhase) -> usize {
            let mut calls = self.acl_read_calls.borrow_mut();
            if let Some((_, count)) = calls
                .iter_mut()
                .find(|(recorded_phase, _)| *recorded_phase == phase)
            {
                *count += 1;
                *count
            } else {
                calls.push((phase, 1));
                1
            }
        }

        fn acl_read_call_count(&self, phase: ReadPhase) -> usize {
            self.acl_read_calls
                .borrow()
                .iter()
                .find_map(|(recorded_phase, count)| (*recorded_phase == phase).then_some(*count))
                .unwrap_or(0)
        }

        fn record_selected_call(&self) {
            self.selected_calls.set(self.selected_calls.get() + 1);
        }
    }

    impl EditFileEvidence for ScriptedEvidence {
        fn open_walk(
            &mut self,
            phase: WalkPhase,
            step: WalkStep,
            parent: BorrowedFd<'_>,
            component: &str,
        ) -> Result<OwnedFd, rustix::io::Errno> {
            let selected = self
                .walk_fault
                .is_some_and(|(selected_phase, selected_step, _)| {
                    phase == selected_phase && step == selected_step
                });
            if selected {
                self.record_selected_call();
                let (_, _, fault) = self.walk_fault.unwrap();
                if matches!(fault, EvidenceFault::Error | EvidenceFault::Interrupted) {
                    return Err(if fault == EvidenceFault::Interrupted {
                        rustix::io::Errno::INTR
                    } else {
                        rustix::io::Errno::IO
                    });
                }
                let opened =
                    rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty());
                if fault == EvidenceFault::Cancel {
                    self.cancellation.cancel();
                }
                return opened;
            }
            rustix::fs::openat(parent, component, directory_open_flags(), Mode::empty())
        }

        fn open_target(
            &mut self,
            phase: ReadPhase,
            parent: BorrowedFd<'_>,
            basename: &str,
        ) -> Result<OwnedFd, rustix::io::Errno> {
            if let Some(fault) = self.selected_read(phase, EvidenceOperation::OpenTarget) {
                self.record_selected_call();
                return Err(if fault == EvidenceFault::Interrupted {
                    rustix::io::Errno::INTR
                } else {
                    rustix::io::Errno::IO
                });
            }
            rustix::fs::openat(
                parent,
                basename,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
        }

        fn open_stage(
            &mut self,
            parent: BorrowedFd<'_>,
            name: &str,
        ) -> Result<OwnedFd, rustix::io::Errno> {
            if self.stage_open_error {
                self.record_selected_call();
                return Err(rustix::io::Errno::IO);
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

        fn pread(
            &mut self,
            phase: ReadPhase,
            file: BorrowedFd<'_>,
            buffer: &mut [u8],
            offset: u64,
        ) -> Result<usize, rustix::io::Errno> {
            if phase == ReadPhase::Published {
                self.published_pread_calls
                    .set(self.published_pread_calls.get() + 1);
            }
            if let Some(fault) = self.selected_read(phase, EvidenceOperation::Pread) {
                self.record_selected_call();
                return match fault {
                    EvidenceFault::Error => Err(rustix::io::Errno::IO),
                    EvidenceFault::Interrupted => Err(rustix::io::Errno::INTR),
                    EvidenceFault::EarlyEof => Ok(0),
                    EvidenceFault::Cancel => {
                        let result = rustix::io::pread(file, buffer, offset);
                        self.cancellation.cancel();
                        result
                    }
                };
            }
            rustix::io::pread(file, buffer, offset)
        }

        fn fstat(
            &mut self,
            phase: ReadPhase,
            file: BorrowedFd<'_>,
        ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
            if let Some(fault) = self.selected_read(phase, EvidenceOperation::Fstat) {
                self.record_selected_call();
                return Err(if fault == EvidenceFault::Interrupted {
                    rustix::io::Errno::INTR
                } else {
                    rustix::io::Errno::IO
                });
            }
            rustix::fs::fstat(file)
        }

        fn statat(
            &mut self,
            phase: ReadPhase,
            parent: BorrowedFd<'_>,
            name: &str,
        ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
            if let Some(fault) = self.selected_read(phase, EvidenceOperation::Statat) {
                self.record_selected_call();
                return Err(if fault == EvidenceFault::Interrupted {
                    rustix::io::Errno::INTR
                } else {
                    rustix::io::Errno::IO
                });
            }
            rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        }

        fn clear_staged_acl(&mut self, _file: BorrowedFd<'_>) -> Result<(), ToolError> {
            self.clear_acl_calls.set(self.clear_acl_calls.get() + 1);
            if self.clear_acl_error {
                self.record_selected_call();
                Err(write_failed())
            } else {
                Ok(())
            }
        }

        fn staged_acl_is_empty(
            &mut self,
            _file: BorrowedFd<'_>,
            phase: ReadPhase,
        ) -> Result<bool, ToolError> {
            let ordinal = self.acl_read_ordinal(phase);
            match self.acl_read_fault {
                Some((selected_phase, selected_ordinal, fault))
                    if phase == selected_phase && ordinal == selected_ordinal =>
                {
                    self.record_selected_call();
                    match fault {
                        AclReadFault::Error => Err(map_read_phase_failure(phase)),
                        AclReadFault::Unsafe => Ok(false),
                    }
                }
                _ => Ok(true),
            }
        }

        fn after_stage_created(
            &mut self,
            _parent: BorrowedFd<'_>,
            _file: BorrowedFd<'_>,
            _name: &str,
            _cancellation: &CancellationToken,
        ) -> Result<(), ToolError> {
            match self.checkpoint {
                Some(EvidenceCheckpoint::StageError) => {
                    self.record_selected_call();
                    Err(write_failed())
                }
                Some(EvidenceCheckpoint::StageCancel) => {
                    self.record_selected_call();
                    self.cancellation.cancel();
                    Ok(())
                }
                _ => Ok(()),
            }
        }

        fn after_final_stage_sync(
            &mut self,
            parent: BorrowedFd<'_>,
            _file: BorrowedFd<'_>,
            name: &str,
            _cancellation: &CancellationToken,
        ) -> Result<(), ToolError> {
            self.final_stage_sync_calls
                .set(self.final_stage_sync_calls.get() + 1);
            if self.checkpoint == Some(EvidenceCheckpoint::FinalSyncCorrupt) {
                self.record_selected_call();
                overwrite_same_length_at(parent, name, b"bad content");
            }
            Ok(())
        }

        fn after_final_stage_verification(
            &mut self,
            _parent: BorrowedFd<'_>,
            _file: BorrowedFd<'_>,
            _name: &str,
            _cancellation: &CancellationToken,
        ) -> Result<(), ToolError> {
            match self.checkpoint {
                Some(EvidenceCheckpoint::FinalVerificationError) => {
                    self.record_selected_call();
                    Err(write_failed())
                }
                Some(EvidenceCheckpoint::FinalVerificationCancel) => {
                    self.record_selected_call();
                    self.cancellation.cancel();
                    Ok(())
                }
                _ => Ok(()),
            }
        }

        fn after_rename(
            &mut self,
            _parent: BorrowedFd<'_>,
            _file: BorrowedFd<'_>,
            _basename: &str,
            _cancellation: &CancellationToken,
        ) -> Result<(), ToolError> {
            if self.checkpoint == Some(EvidenceCheckpoint::RenameCancel) {
                self.record_selected_call();
                self.cancellation.cancel();
            }
            Ok(())
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CleanupOperation {
        SetMode,
        Fstat,
        Statat,
        Unlink,
    }

    struct ScriptedCleanupEvidence {
        operations: RefCell<Vec<CleanupOperation>>,
        descriptor_fingerprint: RefCell<Option<FileFingerprint>>,
        path_fingerprint: RefCell<Option<FileFingerprint>>,
        fail_set_mode: bool,
        fail_unlink: bool,
    }

    impl ScriptedCleanupEvidence {
        fn dual_failure() -> Self {
            Self {
                operations: RefCell::new(Vec::new()),
                descriptor_fingerprint: RefCell::new(None),
                path_fingerprint: RefCell::new(None),
                fail_set_mode: true,
                fail_unlink: true,
            }
        }
    }

    impl EditFileCleanupEvidence for ScriptedCleanupEvidence {
        fn set_mode(&self, file: BorrowedFd<'_>, mode: Mode) -> Result<(), rustix::io::Errno> {
            self.operations.borrow_mut().push(CleanupOperation::SetMode);
            assert_eq!(mode.as_raw_mode(), 0o600);
            if self.fail_set_mode {
                Err(rustix::io::Errno::IO)
            } else {
                rustix::fs::fchmod(file, mode)
            }
        }

        fn fstat(&self, file: BorrowedFd<'_>) -> Result<rustix::fs::Stat, rustix::io::Errno> {
            self.operations.borrow_mut().push(CleanupOperation::Fstat);
            let metadata = rustix::fs::fstat(file)?;
            self.descriptor_fingerprint
                .replace(Some(FileFingerprint::from_stat(&metadata)));
            Ok(metadata)
        }

        fn statat(
            &self,
            parent: BorrowedFd<'_>,
            name: &str,
        ) -> Result<rustix::fs::Stat, rustix::io::Errno> {
            self.operations.borrow_mut().push(CleanupOperation::Statat);
            let metadata = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
            self.path_fingerprint
                .replace(Some(FileFingerprint::from_stat(&metadata)));
            Ok(metadata)
        }

        fn unlink(&self, parent: BorrowedFd<'_>, name: &str) -> Result<(), rustix::io::Errno> {
            self.operations.borrow_mut().push(CleanupOperation::Unlink);
            if self.fail_unlink {
                Err(rustix::io::Errno::IO)
            } else {
                rustix::fs::unlinkat(parent, name, AtFlags::empty())
            }
        }
    }

    fn assert_scripted_pipeline_failure(
        label: &str,
        cancellation: &CancellationToken,
        evidence: &mut ScriptedEvidence,
        expected_code: &str,
        retryable: bool,
        expected_selected_calls: usize,
        postcommit: bool,
    ) {
        let temporary = TempDirectory::new(label);
        let parent = temporary.path().join("nested");
        fs::create_dir(&parent).unwrap();
        let target = parent.join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let publish_calls = Cell::new(0_usize);
        let parent_sync_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence(
                "nested/target.txt",
                b"old",
                b"new",
                cancellation,
                evidence,
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {},
                || {},
                |parent, staged_name, basename| {
                    publish_calls.set(publish_calls.get() + 1);
                    native_publish_staged(parent, staged_name, basename)
                },
                |parent| {
                    parent_sync_calls.set(parent_sync_calls.get() + 1);
                    native_sync_parent(parent)
                },
            )
            .unwrap_err();
        assert_eq!(error.code, expected_code, "failed case: {label}");
        assert_eq!(error.retryable, retryable, "failed case: {label}");
        assert_eq!(
            evidence.selected_calls.get(),
            expected_selected_calls,
            "failed case: {label}"
        );
        assert_eq!(
            publish_calls.get(),
            usize::from(postcommit),
            "failed case: {label}"
        );
        assert_eq!(
            parent_sync_calls.get(),
            usize::from(postcommit),
            "failed case: {label}"
        );
        assert_eq!(
            fs::read(&target).unwrap(),
            if postcommit {
                b"new content".as_slice()
            } else {
                b"old content".as_slice()
            },
            "failed case: {label}"
        );
        assert_no_staged_files(&parent);
    }

    #[test]
    fn pipeline_root_and_intermediate_walk_faults_and_cancellation_are_phase_exact() {
        for phase in [WalkPhase::Initial, WalkPhase::Revalidate] {
            for step in [WalkStep::Root, WalkStep::Intermediate(0)] {
                for fault in [EvidenceFault::Error, EvidenceFault::Cancel] {
                    let cancellation = CancellationToken::new();
                    let mut evidence = ScriptedEvidence::new(&cancellation);
                    evidence.walk_fault = Some((phase, step, fault));
                    let cancelled = fault == EvidenceFault::Cancel;
                    let expected_code = if cancelled {
                        "edit_file_cancelled"
                    } else if phase == WalkPhase::Initial {
                        "edit_file_unavailable"
                    } else {
                        "edit_file_target_changed"
                    };
                    assert_scripted_pipeline_failure(
                        &format!("walk-{phase:?}-{step:?}-{fault:?}"),
                        &cancellation,
                        &mut evidence,
                        expected_code,
                        !cancelled,
                        1,
                        false,
                    );
                }
            }
        }
    }

    #[test]
    fn pipeline_initial_and_revalidation_reads_map_error_interrupt_eof_and_cancel() {
        for phase in [ReadPhase::Initial, ReadPhase::Revalidate] {
            for fault in [
                EvidenceFault::Error,
                EvidenceFault::Interrupted,
                EvidenceFault::EarlyEof,
                EvidenceFault::Cancel,
            ] {
                let cancellation = CancellationToken::new();
                let mut evidence = ScriptedEvidence::new(&cancellation);
                evidence.set_read_fault(
                    phase,
                    EvidenceOperation::Pread,
                    1,
                    if fault == EvidenceFault::Interrupted {
                        MAX_INTERRUPTED_SYSCALL_ATTEMPTS
                    } else {
                        1
                    },
                    fault,
                );
                let cancelled = fault == EvidenceFault::Cancel;
                let expected_code = if cancelled {
                    "edit_file_cancelled"
                } else if phase == ReadPhase::Initial {
                    "edit_file_unavailable"
                } else {
                    "edit_file_target_changed"
                };
                assert_scripted_pipeline_failure(
                    &format!("target-read-{phase:?}-{fault:?}"),
                    &cancellation,
                    &mut evidence,
                    expected_code,
                    !cancelled,
                    if fault == EvidenceFault::Interrupted {
                        MAX_INTERRUPTED_SYSCALL_ATTEMPTS
                    } else {
                        1
                    },
                    false,
                );
            }
        }
    }

    #[test]
    fn pipeline_target_open_and_path_stat_faults_map_by_read_phase() {
        for phase in [ReadPhase::Initial, ReadPhase::Revalidate] {
            for operation in [EvidenceOperation::OpenTarget, EvidenceOperation::Statat] {
                for fault in [EvidenceFault::Error, EvidenceFault::Interrupted] {
                    let cancellation = CancellationToken::new();
                    let mut evidence = ScriptedEvidence::new(&cancellation);
                    evidence.set_read_fault(phase, operation, 1, 1, fault);
                    assert_scripted_pipeline_failure(
                        &format!("target-metadata-{phase:?}-{operation:?}-{fault:?}"),
                        &cancellation,
                        &mut evidence,
                        if phase == ReadPhase::Initial {
                            "edit_file_unavailable"
                        } else {
                            "edit_file_target_changed"
                        },
                        true,
                        1,
                        false,
                    );
                }
            }
        }
    }

    #[test]
    fn pipeline_stage_open_and_post_create_failure_or_cancel_leave_no_residue() {
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(&cancellation);
        evidence.stage_open_error = true;
        assert_scripted_pipeline_failure(
            "stage-open-error",
            &cancellation,
            &mut evidence,
            "edit_file_unavailable",
            true,
            1,
            false,
        );

        for (checkpoint, code, retryable) in [
            (
                EvidenceCheckpoint::StageError,
                "edit_file_write_failed",
                true,
            ),
            (
                EvidenceCheckpoint::StageCancel,
                "edit_file_cancelled",
                false,
            ),
        ] {
            let cancellation = CancellationToken::new();
            let mut evidence = ScriptedEvidence::new(&cancellation);
            evidence.checkpoint = Some(checkpoint);
            assert_scripted_pipeline_failure(
                &format!("post-create-{checkpoint:?}"),
                &cancellation,
                &mut evidence,
                code,
                retryable,
                1,
                false,
            );
        }
    }

    #[test]
    fn pipeline_staged_and_published_stat_path_and_read_fault_matrix_is_exact() {
        let cases = [
            (EvidenceOperation::Fstat, EvidenceFault::Error, 1),
            (EvidenceOperation::Fstat, EvidenceFault::Interrupted, 1),
            (EvidenceOperation::Statat, EvidenceFault::Error, 1),
            (EvidenceOperation::Statat, EvidenceFault::Interrupted, 1),
            (EvidenceOperation::Pread, EvidenceFault::Error, 1),
            (
                EvidenceOperation::Pread,
                EvidenceFault::Interrupted,
                MAX_INTERRUPTED_SYSCALL_ATTEMPTS,
            ),
            (EvidenceOperation::Pread, EvidenceFault::EarlyEof, 1),
        ];
        for phase in [ReadPhase::Staged, ReadPhase::Published] {
            for (operation, fault, expected_calls) in cases {
                let cancellation = CancellationToken::new();
                let mut evidence = ScriptedEvidence::new(&cancellation);
                evidence.set_read_fault(phase, operation, 1, expected_calls, fault);
                let published = phase == ReadPhase::Published;
                assert_scripted_pipeline_failure(
                    &format!("stage-publish-{phase:?}-{operation:?}-{fault:?}"),
                    &cancellation,
                    &mut evidence,
                    if published {
                        "edit_file_commit_ambiguous"
                    } else {
                        "edit_file_write_failed"
                    },
                    !published,
                    expected_calls,
                    published,
                );
            }
        }
    }

    #[test]
    fn pipeline_descriptor_and_late_fstat_faults_target_exact_ordinals() {
        let cases = [
            (ReadPhase::Initial, 2, "edit_file_unavailable", true, false),
            (
                ReadPhase::Revalidate,
                2,
                "edit_file_target_changed",
                true,
                false,
            ),
            (ReadPhase::Staged, 18, "edit_file_write_failed", true, false),
            (ReadPhase::Staged, 19, "edit_file_write_failed", true, false),
            (
                ReadPhase::Published,
                3,
                "edit_file_commit_ambiguous",
                false,
                true,
            ),
            (
                ReadPhase::Published,
                4,
                "edit_file_commit_ambiguous",
                false,
                true,
            ),
        ];
        for (phase, ordinal, expected_code, retryable, postcommit) in cases {
            let cancellation = CancellationToken::new();
            let mut evidence = ScriptedEvidence::new(&cancellation);
            evidence.set_read_fault(
                phase,
                EvidenceOperation::Fstat,
                ordinal,
                1,
                EvidenceFault::Error,
            );
            assert_scripted_pipeline_failure(
                &format!("fstat-{phase:?}-ordinal-{ordinal}"),
                &cancellation,
                &mut evidence,
                expected_code,
                retryable,
                1,
                postcommit,
            );
            assert_eq!(
                evidence.read_call_count(phase, EvidenceOperation::Fstat),
                ordinal,
                "fault did not land on the intended {phase:?} fstat"
            );
        }
    }

    #[test]
    fn pipeline_acl_clear_read_and_unsafe_outcomes_map_at_exact_ordinals() {
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(&cancellation);
        evidence.clear_acl_error = true;
        assert_scripted_pipeline_failure(
            "acl-clear-error",
            &cancellation,
            &mut evidence,
            "edit_file_write_failed",
            true,
            1,
            false,
        );
        assert_eq!(evidence.clear_acl_calls.get(), 1);
        assert_eq!(evidence.acl_read_call_count(ReadPhase::Staged), 0);

        for fault in [AclReadFault::Error, AclReadFault::Unsafe] {
            let cancellation = CancellationToken::new();
            let mut evidence = ScriptedEvidence::new(&cancellation);
            evidence.acl_read_fault = Some((ReadPhase::Staged, 1, fault));
            assert_scripted_pipeline_failure(
                &format!("acl-create-read-{fault:?}"),
                &cancellation,
                &mut evidence,
                "edit_file_write_failed",
                true,
                1,
                false,
            );
            assert_eq!(evidence.clear_acl_calls.get(), 1);
            assert_eq!(evidence.acl_read_call_count(ReadPhase::Staged), 1);
            assert_eq!(evidence.final_stage_sync_calls.get(), 0);
        }

        for fault in [AclReadFault::Error, AclReadFault::Unsafe] {
            let cancellation = CancellationToken::new();
            let mut evidence = ScriptedEvidence::new(&cancellation);
            evidence.acl_read_fault = Some((ReadPhase::Staged, 9, fault));
            assert_scripted_pipeline_failure(
                &format!("acl-final-staged-read-{fault:?}"),
                &cancellation,
                &mut evidence,
                "edit_file_write_failed",
                true,
                1,
                false,
            );
            assert_eq!(evidence.final_stage_sync_calls.get(), 1);
            assert_eq!(evidence.acl_read_call_count(ReadPhase::Staged), 9);
        }

        for fault in [AclReadFault::Error, AclReadFault::Unsafe] {
            let cancellation = CancellationToken::new();
            let mut evidence = ScriptedEvidence::new(&cancellation);
            evidence.acl_read_fault = Some((ReadPhase::Published, 2, fault));
            assert_scripted_pipeline_failure(
                &format!("acl-published-read-{fault:?}"),
                &cancellation,
                &mut evidence,
                "edit_file_commit_ambiguous",
                false,
                1,
                true,
            );
            assert_eq!(evidence.final_stage_sync_calls.get(), 1);
            assert_eq!(evidence.acl_read_call_count(ReadPhase::Staged), 9);
            assert_eq!(evidence.acl_read_call_count(ReadPhase::Published), 2);
            assert!(evidence.published_pread_calls.get() > 0);
        }
    }

    #[test]
    fn pipeline_cancellation_after_final_verification_precedes_rename_and_cleans_stage() {
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(&cancellation);
        evidence.checkpoint = Some(EvidenceCheckpoint::FinalVerificationCancel);
        assert_scripted_pipeline_failure(
            "cancel-after-final-verification",
            &cancellation,
            &mut evidence,
            "edit_file_cancelled",
            false,
            1,
            false,
        );
        assert!(cancellation.is_cancelled());
        assert_eq!(evidence.final_stage_sync_calls.get(), 1);
        assert_eq!(evidence.acl_read_call_count(ReadPhase::Staged), 9);
        assert_eq!(evidence.acl_read_call_count(ReadPhase::Published), 0);
    }

    #[test]
    fn pipeline_corruption_after_final_chmod_and_sync_fails_before_rename() {
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(&cancellation);
        evidence.checkpoint = Some(EvidenceCheckpoint::FinalSyncCorrupt);
        assert_scripted_pipeline_failure(
            "corrupt-after-final-sync",
            &cancellation,
            &mut evidence,
            "edit_file_write_failed",
            true,
            1,
            false,
        );
    }

    #[test]
    fn pipeline_cancellation_after_real_rename_is_ignored_while_verify_and_sync_run() {
        let temporary = TempDirectory::new("cancel-after-rename");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let old_identity = rustix::fs::stat(&target).unwrap();
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(&cancellation);
        evidence.checkpoint = Some(EvidenceCheckpoint::RenameCancel);
        let publish_calls = Cell::new(0_usize);
        let parent_sync_calls = Cell::new(0_usize);
        let output = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence(
                "target.txt",
                b"old",
                b"new",
                &cancellation,
                &mut evidence,
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {},
                || {},
                |parent, staged_name, basename| {
                    publish_calls.set(publish_calls.get() + 1);
                    native_publish_staged(parent, staged_name, basename)
                },
                |parent| {
                    parent_sync_calls.set(parent_sync_calls.get() + 1);
                    native_sync_parent(parent)
                },
            )
            .unwrap();
        assert!(!output.is_error);
        assert!(cancellation.is_cancelled());
        assert_eq!(evidence.selected_calls.get(), 1);
        assert!(evidence.published_pread_calls.get() > 0);
        assert_eq!(publish_calls.get(), 1);
        assert_eq!(parent_sync_calls.get(), 1);
        assert_eq!(fs::read(&target).unwrap(), b"new content");
        assert!(!same_identity(
            &old_identity,
            &rustix::fs::stat(&target).unwrap()
        ));
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn cleanup_attempts_unlink_after_mode_reset_failure_and_discloses_dual_failure_residue() {
        let temporary = TempDirectory::new("cleanup-dual-failure");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let parent = temporary.descriptor();
        let staged_name = ".machine-god-edit-owned-residue";
        let staged_path = temporary.path().join(staged_name);
        let staged = rustix::fs::openat(
            parent.as_fd(),
            staged_name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .unwrap();
        rustix::fs::fchmod(&staged, Mode::from_raw_mode(0o751)).unwrap();
        write_content(staged.as_fd(), b"owned residue", &CancellationToken::new()).unwrap();
        let mode_calls = Cell::new(0_usize);
        let unlink_calls = Cell::new(0_usize);

        cleanup_unpublished_file_with(
            parent.as_fd(),
            staged.as_fd(),
            staged_name,
            |_, requested_mode| {
                mode_calls.set(mode_calls.get() + 1);
                assert_eq!(requested_mode.as_raw_mode(), 0o600);
                Err(rustix::io::Errno::IO)
            },
            |_, requested_name| {
                unlink_calls.set(unlink_calls.get() + 1);
                assert_eq!(requested_name, staged_name);
                Err(rustix::io::Errno::IO)
            },
        );

        assert_eq!(mode_calls.get(), 1);
        assert_eq!(unlink_calls.get(), 1);
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_eq!(fs::read(&staged_path).unwrap(), b"owned residue");
        assert_eq!(
            fs::metadata(&staged_path).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }

    #[test]
    fn pipeline_drop_attempts_dual_failure_cleanup_after_final_verification() {
        let temporary = TempDirectory::new("pipeline-drop-dual-failure");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o751)).unwrap();
        let original_target = FileFingerprint::from_stat(&rustix::fs::stat(&target).unwrap());
        let cancellation = CancellationToken::new();
        let mut evidence = ScriptedEvidence::new(&cancellation);
        evidence.checkpoint = Some(EvidenceCheckpoint::FinalVerificationError);
        let cleanup = ScriptedCleanupEvidence::dual_failure();
        let publish_calls = Cell::new(0_usize);
        let parent_sync_calls = Cell::new(0_usize);

        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with_evidence_and_cleanup(
                "target.txt",
                b"old",
                b"new",
                &cancellation,
                &mut evidence,
                &cleanup,
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {},
                || {},
                |parent, staged_name, basename| {
                    publish_calls.set(publish_calls.get() + 1);
                    native_publish_staged(parent, staged_name, basename)
                },
                |parent| {
                    parent_sync_calls.set(parent_sync_calls.get() + 1);
                    native_sync_parent(parent)
                },
            )
            .unwrap_err();

        assert_eq!(error.code, "edit_file_write_failed");
        assert!(error.retryable);
        assert_eq!(evidence.selected_calls.get(), 1);
        assert_eq!(evidence.final_stage_sync_calls.get(), 1);
        assert_eq!(evidence.acl_read_call_count(ReadPhase::Staged), 9);
        assert_eq!(publish_calls.get(), 0);
        assert_eq!(parent_sync_calls.get(), 0);
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_eq!(
            FileFingerprint::from_stat(&rustix::fs::stat(&target).unwrap()),
            original_target
        );
        assert_eq!(
            cleanup.operations.borrow().as_slice(),
            [
                CleanupOperation::SetMode,
                CleanupOperation::Fstat,
                CleanupOperation::Statat,
                CleanupOperation::Unlink,
            ]
        );

        let staged_paths = fs::read_dir(temporary.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(TEMP_NAME_PREFIX)
            })
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        assert_eq!(staged_paths.len(), 1);
        let residue = &staged_paths[0];
        assert_eq!(fs::read(residue).unwrap(), b"new content");
        assert_eq!(
            fs::metadata(residue).unwrap().permissions().mode() & 0o777,
            0o751
        );
        let residue_fingerprint = FileFingerprint::from_stat(&rustix::fs::stat(residue).unwrap());
        assert_eq!(
            *cleanup.descriptor_fingerprint.borrow(),
            Some(residue_fingerprint)
        );
        assert_eq!(
            *cleanup.path_fingerprint.borrow(),
            Some(residue_fingerprint)
        );
    }

    #[test]
    fn bounded_stable_read_handles_partial_reads_and_cumulative_interrupts() {
        let temporary = TempFile::new(b"stable partial contents");
        let file = rustix::fs::open(
            &temporary.path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let calls = Cell::new(0_usize);
        let (bytes, fingerprint) = read_bounded_stable_with(
            file.as_fd(),
            &CancellationToken::new(),
            |buffer, offset| {
                let call = calls.get();
                calls.set(call + 1);
                if call < 10 && !call.is_multiple_of(2) {
                    Err(rustix::io::Errno::INTR)
                } else {
                    let end = buffer.len().min(3);
                    rustix::io::pread(file.as_fd(), &mut buffer[..end], offset)
                }
            },
            || rustix::fs::fstat(&file),
        )
        .unwrap();
        assert_eq!(bytes, b"stable partial contents");
        assert_eq!(
            fingerprint,
            FileFingerprint::from_stat(&rustix::fs::fstat(&file).unwrap())
        );
        assert!(calls.get() > 5);
    }

    #[test]
    fn bounded_stable_read_rejects_exact_interruption_exhaustion_and_overreported_progress() {
        let temporary = TempFile::new(b"unchanged");
        let file = rustix::fs::open(
            &temporary.path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let calls = Cell::new(0_usize);
        let error = read_bounded_stable_with(
            file.as_fd(),
            &CancellationToken::new(),
            |_, _| {
                calls.set(calls.get() + 1);
                Err(rustix::io::Errno::INTR)
            },
            || rustix::fs::fstat(&file),
        )
        .unwrap_err();
        assert_eq!(calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert_eq!(error.code, "edit_file_unavailable");

        let overreported = read_bounded_stable_with(
            file.as_fd(),
            &CancellationToken::new(),
            |buffer, _| Ok(buffer.len() + 1),
            || rustix::fs::fstat(&file),
        )
        .unwrap_err();
        assert_eq!(overreported.code, "edit_file_unavailable");
    }

    #[test]
    fn bounded_stable_read_detects_metadata_change_and_post_read_cancellation() {
        let temporary = TempFile::new(b"stable");
        let file = rustix::fs::open(
            &temporary.path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let initial = rustix::fs::fstat(&file).unwrap();
        let mut changed = initial;
        changed.st_mtime = changed.st_mtime.saturating_add(1);
        let stat_calls = Cell::new(0_usize);
        let error = read_bounded_stable_with(
            file.as_fd(),
            &CancellationToken::new(),
            |buffer, offset| rustix::io::pread(file.as_fd(), buffer, offset),
            || {
                let call = stat_calls.get();
                stat_calls.set(call + 1);
                Ok(if call == 0 { initial } else { changed })
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "edit_file_target_changed");

        let file = rustix::fs::open(
            &temporary.path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let cancellation_from_read = cancellation.clone();
        let reads = Cell::new(0_usize);
        let error = read_bounded_stable_with(
            file.as_fd(),
            &cancellation,
            |_, _| {
                reads.set(reads.get() + 1);
                cancellation_from_read.cancel();
                Ok(1)
            },
            || rustix::fs::fstat(&file),
        )
        .unwrap_err();
        assert_eq!(reads.get(), 1);
        assert_eq!(error.code, "edit_file_cancelled");
    }

    #[test]
    fn bounded_read_phase_faults_are_mapped_and_final_interruption_cancellation_wins() {
        let temporary = TempFile::new(b"stable");
        let file = rustix::fs::open(
            &temporary.path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        for (phase, code) in [
            (ReadPhase::Staged, "edit_file_write_failed"),
            (ReadPhase::Revalidate, "edit_file_target_changed"),
            (ReadPhase::Published, "edit_file_commit_ambiguous"),
        ] {
            let error = read_bounded_stable_for_phase_with(
                &CancellationToken::new(),
                MAX_EDIT_FILE_EXISTING_BYTES,
                phase,
                |_, _| Err(rustix::io::Errno::IO),
                || rustix::fs::fstat(&file),
            )
            .unwrap_err();
            assert_eq!(error.code, code);
        }

        let cancellation = CancellationToken::new();
        let cancellation_on_final = cancellation.clone();
        let calls = Cell::new(0_usize);
        let error = read_bounded_stable_with(
            file.as_fd(),
            &cancellation,
            |_, _| {
                let call = calls.get() + 1;
                calls.set(call);
                if call == MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    cancellation_on_final.cancel();
                }
                Err(rustix::io::Errno::INTR)
            },
            || rustix::fs::fstat(&file),
        )
        .unwrap_err();
        assert_eq!(calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert_eq!(error.code, "edit_file_cancelled");

        let cancellation = CancellationToken::new();
        let cancellation_on_final = cancellation.clone();
        let calls = Cell::new(0_usize);
        let error = read_bounded_stable_for_phase_with(
            &cancellation,
            MAX_EDIT_FILE_RESULTING_BYTES,
            ReadPhase::Published,
            |_, _| {
                let call = calls.get() + 1;
                calls.set(call);
                if call == MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                    cancellation_on_final.cancel();
                }
                Err(rustix::io::Errno::INTR)
            },
            || rustix::fs::fstat(&file),
        )
        .unwrap_err();
        assert_eq!(calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert!(cancellation.is_cancelled());
        assert_eq!(error.code, "edit_file_commit_ambiguous");
        assert!(!error.retryable);
    }

    #[test]
    fn stable_size_rejects_early_eof_for_initial_staged_and_revalidation_reads() {
        let temporary = TempFile::new(b"stable");
        let file = rustix::fs::open(
            &temporary.path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        for (phase, expected_code) in [
            (ReadPhase::Initial, "edit_file_unavailable"),
            (ReadPhase::Staged, "edit_file_write_failed"),
            (ReadPhase::Revalidate, "edit_file_target_changed"),
            (ReadPhase::Published, "edit_file_commit_ambiguous"),
        ] {
            let zero_calls = Cell::new(0_usize);
            let error = read_bounded_stable_for_phase_with(
                &CancellationToken::new(),
                MAX_EDIT_FILE_EXISTING_BYTES,
                phase,
                |_, _| {
                    zero_calls.set(zero_calls.get() + 1);
                    Ok(0)
                },
                || rustix::fs::fstat(&file),
            )
            .unwrap_err();
            assert_eq!(zero_calls.get(), 1);
            assert_eq!(error.code, expected_code);

            let short_calls = Cell::new(0_usize);
            let error = read_bounded_stable_for_phase_with(
                &CancellationToken::new(),
                MAX_EDIT_FILE_EXISTING_BYTES,
                phase,
                |buffer, offset| {
                    short_calls.set(short_calls.get() + 1);
                    if offset == 0 {
                        buffer[..2].copy_from_slice(b"st");
                        Ok(2)
                    } else {
                        Ok(0)
                    }
                },
                || rustix::fs::fstat(&file),
            )
            .unwrap_err();
            assert_eq!(short_calls.get(), 2);
            assert_eq!(error.code, expected_code);
        }
    }

    #[test]
    fn entropy_partial_progress_interrupts_and_total_call_bound_are_exact() {
        let calls = Cell::new(0_usize);
        let name = random_temp_name_with(&CancellationToken::new(), |remaining| {
            let call = calls.get();
            calls.set(call + 1);
            if call.is_multiple_of(2) {
                Ok(1.min(remaining.len()))
            } else {
                Err(rustix::io::Errno::INTR)
            }
        })
        .unwrap();
        assert_eq!(calls.get(), MAX_ENTROPY_SYSCALL_ATTEMPTS);
        assert_eq!(name, format!("{TEMP_NAME_PREFIX}{}", "00".repeat(16)));

        let cumulative_calls = Cell::new(0_usize);
        let error = random_temp_name_with(&CancellationToken::new(), |remaining| {
            let call = cumulative_calls.get();
            cumulative_calls.set(call + 1);
            if call < 10 && call.is_multiple_of(2) {
                Ok(1.min(remaining.len()))
            } else {
                Err(rustix::io::Errno::INTR)
            }
        })
        .unwrap_err();
        assert_eq!(cumulative_calls.get(), 21);
        assert_eq!(error.code, "edit_file_unavailable");

        for count in [0_usize, 17] {
            let error =
                random_temp_name_with(&CancellationToken::new(), |_| Ok(count)).unwrap_err();
            assert_eq!(error.code, "edit_file_unavailable");
        }
    }

    #[test]
    fn entropy_cancellation_wins_final_interruption_and_partial_success() {
        let cancellation = CancellationToken::new();
        let cancellation_on_final = cancellation.clone();
        let calls = Cell::new(0_usize);
        let error = random_temp_name_with(&cancellation, |_| {
            let call = calls.get() + 1;
            calls.set(call);
            if call == MAX_INTERRUPTED_SYSCALL_ATTEMPTS {
                cancellation_on_final.cancel();
            }
            Err(rustix::io::Errno::INTR)
        })
        .unwrap_err();
        assert_eq!(calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert_eq!(error.code, "edit_file_cancelled");

        let cancellation = CancellationToken::new();
        let cancellation_after_partial = cancellation.clone();
        let calls = Cell::new(0_usize);
        let error = random_temp_name_with(&cancellation, |_| {
            calls.set(calls.get() + 1);
            cancellation_after_partial.cancel();
            Ok(1)
        })
        .unwrap_err();
        assert_eq!(calls.get(), 1);
        assert_eq!(error.code, "edit_file_cancelled");
    }

    #[test]
    fn eight_temp_collisions_are_preserved_and_never_equal_the_target() {
        let temporary = TempDirectory::new("collisions");
        let collision_name = ".machine-god-edit-collision";
        fs::write(temporary.path().join(collision_name), b"foreign").unwrap();
        let parent = temporary.descriptor();
        let attempts = Cell::new(0_usize);
        let error = create_staged_file_with(
            parent.as_fd(),
            "target.txt",
            &CancellationToken::new(),
            |attempt| {
                assert_eq!(attempt, attempts.get());
                attempts.set(attempts.get() + 1);
                Ok(collision_name.to_owned())
            },
        )
        .unwrap_err();
        assert_eq!(attempts.get(), MAX_EDIT_FILE_TEMP_ATTEMPTS);
        assert_eq!(error.code, "edit_file_unavailable");
        assert_eq!(
            fs::read(temporary.path().join(collision_name)).unwrap(),
            b"foreign"
        );

        let basename_attempts = Cell::new(0_usize);
        let error = create_staged_file_with(
            parent.as_fd(),
            "target.txt",
            &CancellationToken::new(),
            |_| {
                basename_attempts.set(basename_attempts.get() + 1);
                Ok("target.txt".to_owned())
            },
        )
        .unwrap_err();
        assert_eq!(basename_attempts.get(), MAX_EDIT_FILE_TEMP_ATTEMPTS);
        assert_eq!(error.code, "edit_file_unavailable");
        assert!(!temporary.path().join("target.txt").exists());
    }

    #[test]
    fn bounded_write_and_precommit_sync_use_cumulative_interruption_limits() {
        let content = vec![b'x'; MAX_EDIT_FILE_CHUNK_BYTES * 2 + 1];
        let chunk_lengths = RefCell::new(Vec::new());
        let mut consumed = 0_usize;
        write_content_with(&content, &CancellationToken::new(), |chunk| {
            chunk_lengths.borrow_mut().push(chunk.len());
            let written = chunk.len().min(3);
            consumed += written;
            Ok(written)
        })
        .unwrap();
        assert_eq!(consumed, content.len());
        assert!(
            chunk_lengths
                .borrow()
                .iter()
                .all(|length| *length <= MAX_EDIT_FILE_CHUNK_BYTES)
        );

        let write_calls = Cell::new(0_usize);
        let error = write_content_with(b"x", &CancellationToken::new(), |_| {
            write_calls.set(write_calls.get() + 1);
            Err(rustix::io::Errno::INTR)
        })
        .unwrap_err();
        assert_eq!(write_calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert_eq!(error.code, "edit_file_write_failed");

        let sync_calls = Cell::new(0_usize);
        let error = sync_before_commit_with(&CancellationToken::new(), || {
            sync_calls.set(sync_calls.get() + 1);
            Err(rustix::io::Errno::INTR)
        })
        .unwrap_err();
        assert_eq!(sync_calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert_eq!(error.code, "edit_file_write_failed");
    }

    #[test]
    fn pipeline_write_failure_preserves_target_and_cleans_stage() {
        let temporary = TempDirectory::new("write-failure");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                |_, content, cancellation| {
                    write_content_with(content, cancellation, |_| {
                        calls.set(calls.get() + 1);
                        Err(rustix::io::Errno::INTR)
                    })
                },
                sync_before_commit,
                |_, _| {},
                || {},
                || {},
                native_publish_staged,
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert_eq!(error.code, "edit_file_write_failed");
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_same_length_corrupt_stage_is_detected_by_bounded_reread() {
        let temporary = TempDirectory::new("corrupt-stage");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                |file, expected, cancellation| {
                    assert_eq!(expected, b"new content");
                    write_content(file, b"bad content", cancellation)
                },
                sync_before_commit,
                |_, _| {},
                || {},
                || {},
                native_publish_staged,
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_write_failed");
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_same_inode_same_size_stage_mutation_before_revalidation_fails_precommit() {
        let temporary = TempDirectory::new("stage-mutation-before-revalidation");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let publish_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |parent, staged_name| {
                    overwrite_same_length_at(parent, staged_name, b"bad content");
                },
                || {},
                || {},
                |parent, staged_name, basename| {
                    publish_calls.set(publish_calls.get() + 1);
                    native_publish_staged(parent, staged_name, basename)
                },
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_write_failed");
        assert!(error.retryable);
        assert_eq!(publish_calls.get(), 0);
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_same_inode_same_size_stage_mutation_before_rename_fails_precommit() {
        let temporary = TempDirectory::new("stage-mutation-before-rename");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let parent = temporary.descriptor();
        let staged_name = RefCell::new(None::<String>);
        let publish_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, name| {
                    staged_name.replace(Some(name.to_owned()));
                },
                || {},
                || {
                    let name = staged_name
                        .borrow()
                        .as_deref()
                        .expect("staged revalidation must precede the rename hook")
                        .to_owned();
                    overwrite_same_length_at(parent.as_fd(), &name, b"bad content");
                },
                |parent, staged_name, basename| {
                    publish_calls.set(publish_calls.get() + 1);
                    native_publish_staged(parent, staged_name, basename)
                },
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_write_failed");
        assert!(error.retryable);
        assert_eq!(publish_calls.get(), 0);
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_mode_and_staged_sync_failures_preserve_target_and_clean_stage() {
        for failed_mode_call in 0..=1 {
            let temporary = TempDirectory::new(&format!("mode-failure-{failed_mode_call}"));
            let target = temporary.path().join("target.txt");
            fs::write(&target, b"old content").unwrap();
            let mode_calls = Cell::new(0_usize);
            let error = EditFileTool::open(temporary.path())
                .unwrap()
                .execute_supported_with(
                    "target.txt",
                    b"old",
                    b"new",
                    &CancellationToken::new(),
                    |file, mode| {
                        let call = mode_calls.get();
                        mode_calls.set(call + 1);
                        if call == failed_mode_call {
                            Err(rustix::io::Errno::IO)
                        } else {
                            native_set_mode(file, mode)
                        }
                    },
                    write_content,
                    sync_before_commit,
                    |_, _| {},
                    || {},
                    || {},
                    native_publish_staged,
                    native_sync_parent,
                )
                .unwrap_err();
            assert_eq!(mode_calls.get(), failed_mode_call + 1);
            assert_eq!(error.code, "edit_file_write_failed");
            assert_eq!(fs::read(&target).unwrap(), b"old content");
            assert_no_staged_files(temporary.path());
        }

        let temporary = TempDirectory::new("staged-sync-failure");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let sync_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                |_, cancellation| {
                    sync_before_commit_with(cancellation, || {
                        sync_calls.set(sync_calls.get() + 1);
                        Err(rustix::io::Errno::INTR)
                    })
                },
                |_, _| {},
                || {},
                || {},
                native_publish_staged,
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(sync_calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert_eq!(error.code, "edit_file_write_failed");
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_rename_failure_is_precommit_but_published_path_mismatch_is_ambiguous() {
        let temporary = TempDirectory::new("rename-failure");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {},
                || {},
                |_, _, _| Err(rustix::io::Errno::ACCESS),
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_permission_denied");
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());

        let temporary = TempDirectory::new("published-mismatch");
        let target = temporary.path().join("target.txt");
        let displaced = temporary.path().join("published-stage");
        fs::write(&target, b"old content").unwrap();
        let root = temporary.path().to_owned();
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {},
                || {},
                |parent, staged_name, basename| {
                    native_publish_staged(parent, staged_name, basename)?;
                    rustix::fs::renameat(parent, basename, parent, "published-stage")?;
                    fs::write(root.join(basename), b"intruder")
                        .map_err(|_| rustix::io::Errno::IO)?;
                    Ok(())
                },
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_commit_ambiguous");
        assert!(!error.retryable);
        assert_eq!(fs::read(&target).unwrap(), b"intruder");
        assert_eq!(fs::read(&displaced).unwrap(), b"new content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_same_length_published_inode_corruption_is_ambiguous_and_syncs_parent() {
        let temporary = TempDirectory::new("published-content-corruption");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let staged_identity = Cell::new(None::<(i128, i128)>);
        let published_identity = Cell::new(None::<(i128, i128)>);
        let parent_sync_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |parent, staged_name| {
                    let metadata =
                        rustix::fs::statat(parent, staged_name, AtFlags::SYMLINK_NOFOLLOW).unwrap();
                    staged_identity.set(Some((
                        i128::from(metadata.st_dev),
                        i128::from(metadata.st_ino),
                    )));
                },
                || {},
                || {},
                |parent, staged_name, basename| {
                    native_publish_staged(parent, staged_name, basename)?;
                    let metadata = rustix::fs::statat(parent, basename, AtFlags::SYMLINK_NOFOLLOW)?;
                    published_identity.set(Some((
                        i128::from(metadata.st_dev),
                        i128::from(metadata.st_ino),
                    )));
                    overwrite_same_length_at(parent, basename, b"bad content");
                    Ok(())
                },
                |parent| {
                    parent_sync_calls.set(parent_sync_calls.get() + 1);
                    native_sync_parent(parent)
                },
            )
            .unwrap_err();
        assert_eq!(staged_identity.get(), published_identity.get());
        assert_eq!(error.code, "edit_file_commit_ambiguous");
        assert!(!error.retryable);
        assert_eq!(parent_sync_calls.get(), 1);
        assert_eq!(fs::read(&target).unwrap(), b"bad content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_staged_name_swap_preserves_intruder_and_resets_owned_inode_mode() {
        let temporary = TempDirectory::new("staged-name-swap");
        let target = temporary.path().join("target.txt");
        let displaced = temporary.path().join("displaced-stage");
        fs::write(&target, b"old content").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o777)).unwrap();
        let root = temporary.path().to_owned();
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |parent, staged_name| {
                    rustix::fs::renameat(parent, staged_name, parent, "displaced-stage").unwrap();
                    fs::write(root.join(staged_name), b"intruder").unwrap();
                    fs::set_permissions(root.join(staged_name), fs::Permissions::from_mode(0o640))
                        .unwrap();
                },
                || {},
                || {},
                native_publish_staged,
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_write_failed");
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_eq!(fs::read(&displaced).unwrap(), b"new content");
        assert_eq!(
            fs::metadata(&displaced).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let intruder = fs::read_dir(temporary.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(TEMP_NAME_PREFIX)
            })
            .unwrap();
        assert_eq!(fs::read(intruder.path()).unwrap(), b"intruder");
        assert_eq!(
            intruder.metadata().unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn pipeline_target_change_before_final_verification_fails_closed() {
        let temporary = TempDirectory::new("target-change");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let raced_target = target.clone();
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || fs::write(&raced_target, b"raced replacement").unwrap(),
                || {},
                native_publish_staged,
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_target_changed");
        assert_eq!(fs::read(&target).unwrap(), b"raced replacement");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_cancellation_at_final_verification_preserves_target_and_cleans_stage() {
        let temporary = TempDirectory::new("final-verification-cancel");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let cancellation = CancellationToken::new();
        let cancellation_before_verification = cancellation.clone();
        let publish_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &cancellation,
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {
                    cancellation_before_verification.cancel();
                },
                || {},
                |parent, staged_name, basename| {
                    publish_calls.set(publish_calls.get() + 1);
                    native_publish_staged(parent, staged_name, basename)
                },
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_cancelled");
        assert!(!error.retryable);
        assert_eq!(publish_calls.get(), 0);
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_native_publish_retains_a_parent_moved_outside_the_public_path() {
        let temporary = TempDirectory::new("moved-parent-race");
        let workspace = temporary.path().join("workspace");
        let original_parent = workspace.join("nested");
        let moved_parent = temporary.path().join("outside-workspace");
        fs::create_dir_all(&original_parent).unwrap();
        fs::write(original_parent.join("target.txt"), b"retained old").unwrap();
        let raced_original_parent = original_parent.clone();
        let raced_moved_parent = moved_parent.clone();
        let before_rename_calls = Cell::new(0_usize);
        let output = EditFileTool::open(&workspace)
            .unwrap()
            .execute_supported_with(
                "nested/target.txt",
                b"old",
                b"new",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {},
                || {
                    before_rename_calls.set(before_rename_calls.get() + 1);
                    fs::rename(&raced_original_parent, &raced_moved_parent).unwrap();
                    fs::create_dir(&raced_original_parent).unwrap();
                    fs::write(raced_original_parent.join("target.txt"), b"replacement old")
                        .unwrap();
                },
                native_publish_staged,
                native_sync_parent,
            )
            .unwrap();
        assert_eq!(before_rename_calls.get(), 1);
        assert!(!output.is_error);
        assert_eq!(
            fs::read(moved_parent.join("target.txt")).unwrap(),
            b"retained new"
        );
        assert_eq!(
            fs::read(original_parent.join("target.txt")).unwrap(),
            b"replacement old"
        );
        assert_no_staged_files(&original_parent);
        assert_no_staged_files(&moved_parent);
    }

    #[test]
    fn pipeline_pre_rename_cancellation_preserves_target_and_cleans_stage() {
        let temporary = TempDirectory::new("pre-rename-cancel");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let cancellation = CancellationToken::new();
        let cancellation_before_rename = cancellation.clone();
        let publish_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &cancellation,
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {},
                || {
                    cancellation_before_rename.cancel();
                },
                |parent, staged_name, basename| {
                    publish_calls.set(publish_calls.get() + 1);
                    native_publish_staged(parent, staged_name, basename)
                },
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_cancelled");
        assert_eq!(publish_calls.get(), 0);
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_cancellation_from_staged_hook_wins_before_metadata_or_publication() {
        let temporary = TempDirectory::new("staged-hook-cancel");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let cancellation = CancellationToken::new();
        let cancellation_from_hook = cancellation.clone();
        let publish_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"new",
                &cancellation,
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {
                    cancellation_from_hook.cancel();
                },
                || {},
                || {},
                |parent, staged_name, basename| {
                    publish_calls.set(publish_calls.get() + 1);
                    native_publish_staged(parent, staged_name, basename)
                },
                native_sync_parent,
            )
            .unwrap_err();
        assert_eq!(error.code, "edit_file_cancelled");
        assert_eq!(publish_calls.get(), 0);
        assert_eq!(fs::read(&target).unwrap(), b"old content");
        assert_no_staged_files(temporary.path());
    }

    #[test]
    fn pipeline_post_rename_sync_exhaustion_is_nonretryable_commit_ambiguity() {
        let temporary = TempDirectory::new("post-rename-sync");
        let target = temporary.path().join("target.txt");
        fs::write(&target, b"old content").unwrap();
        let sync_calls = Cell::new(0_usize);
        let error = EditFileTool::open(temporary.path())
            .unwrap()
            .execute_supported_with(
                "target.txt",
                b"old",
                b"committed",
                &CancellationToken::new(),
                native_set_mode,
                write_content,
                sync_before_commit,
                |_, _| {},
                || {},
                || {},
                native_publish_staged,
                |_| {
                    sync_calls.set(sync_calls.get() + 1);
                    Err(rustix::io::Errno::INTR)
                },
            )
            .unwrap_err();
        assert_eq!(sync_calls.get(), MAX_INTERRUPTED_SYSCALL_ATTEMPTS);
        assert_eq!(error.code, "edit_file_commit_ambiguous");
        assert!(!error.retryable);
        assert_eq!(fs::read(&target).unwrap(), b"committed content");
        assert_no_staged_files(temporary.path());
    }
}
