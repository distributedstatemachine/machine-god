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

    use super::*;

    struct TempFile {
        path: PathBuf,
    }

    struct TempDirectory {
        path: PathBuf,
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
