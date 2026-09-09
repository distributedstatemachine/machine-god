//! Explicit, process-local undo authority for cooperating workspace tools.
//!
//! This is not a durable journal, a transcript rollback, or pathname CAS.
use std::fmt;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use machine_god_core::CancellationToken;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use machine_god_core::{ToolError, ToolErrorKind};

/// Maximum retained operations, evicting oldest committed operations first.
pub const MAX_FILE_UNDO_ENTRIES: usize = 100;
/// Maximum retained bytes for any one preimage.
pub const MAX_FILE_UNDO_PREIMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Derived bound: each of the 100 entries retains at most one preimage.
pub const MAX_FILE_UNDO_RETAINED_BYTES: usize =
    MAX_FILE_UNDO_ENTRIES * MAX_FILE_UNDO_PREIMAGE_BYTES;
/// Digest-only observation bound; includes the delivered copy tool's full limit.
pub const MAX_FILE_UNDO_OBSERVATION_BYTES: usize = 16 * 1024 * 1024;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use machine_god_core::{
        SessionId, SessionIncarnationId, Tool, ToolCallId, ToolContext, TurnId,
    };
    use serde_json::json;
    use std::fs::{self, File};
    use std::future::Future;
    use std::path::PathBuf;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    use std::task::{Context, Poll, Wake, Waker};

    pub(super) struct Temp(pub(super) PathBuf);
    impl Temp {
        pub(super) fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "mg-undo-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => panic!("{e}"),
                }
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    fn ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        match std::pin::pin!(future).as_mut().poll(&mut context) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("synchronous native effect unexpectedly pending"),
        }
    }
    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("undo").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("undo-incarnation").unwrap(),
            turn_id: TurnId::new("undo-turn").unwrap(),
            call_id: ToolCallId::new("undo-call").unwrap(),
        }
    }
    fn write(temp: &Temp, tracker: &Arc<FileUndoTracker>, path: &str, content: &str) {
        let tool = crate::WriteFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(tool.execute(
            context(),
            json!({"path":path,"content":content}),
            CancellationToken::new(),
        ))
        .unwrap();
    }

    #[test]
    fn file_undo_clear_reservation_fences_all_five_tools_and_abort_preserves_history() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        write(&temp, &tracker, "file", "before");
        fs::write(temp.0.join("source"), "source").unwrap();
        let reservation = tracker.reserve_clear().unwrap();
        assert!(matches!(tracker.reserve_clear(), Err(FileUndoError::Busy)));
        assert_eq!(tracker.clear(), Err(FileUndoError::Busy));
        assert_eq!(
            tracker.latest_unavailable_reason(),
            Err(FileUndoError::Busy)
        );
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::Busy)
        );
        let calls: Vec<(Box<dyn Tool>, serde_json::Value)> = vec![
            (
                Box::new(
                    crate::WriteFileTool::open(&temp.0)
                        .unwrap()
                        .with_undo_tracker(tracker.clone()),
                ),
                json!({"path":"file","content":"changed"}),
            ),
            (
                Box::new(
                    crate::EditFileTool::open(&temp.0)
                        .unwrap()
                        .with_undo_tracker(tracker.clone()),
                ),
                json!({"path":"file","old_string":"before","new_string":"changed"}),
            ),
            (
                Box::new(
                    crate::DeleteFileTool::open(&temp.0)
                        .unwrap()
                        .with_undo_tracker(tracker.clone()),
                ),
                json!({"path":"file"}),
            ),
            (
                Box::new(
                    crate::RenameFileTool::open(&temp.0)
                        .unwrap()
                        .with_undo_tracker(tracker.clone()),
                ),
                json!({"old_path":"file","new_path":"renamed"}),
            ),
            (
                Box::new(
                    crate::CopyFileTool::open(&temp.0)
                        .unwrap()
                        .with_undo_tracker(tracker.clone()),
                ),
                json!({"source":"source","destination":"copied"}),
            ),
        ];
        for (tool, arguments) in calls {
            let error =
                ready(tool.execute(context(), arguments, CancellationToken::new())).unwrap_err();
            assert_eq!(error.code, "file_undo_tracking_failed");
        }
        let root = File::open(&temp.0).unwrap();
        assert_eq!(
            tracker.copy_replace(&root, "source", "file", &CancellationToken::new()),
            Err(FileUndoError::Busy)
        );
        assert_eq!(
            tracker.rename_replace(&root, "source", "file", &CancellationToken::new()),
            Err(FileUndoError::Busy)
        );
        assert_eq!(fs::read_to_string(temp.0.join("file")).unwrap(), "before");
        assert_eq!(fs::read_to_string(temp.0.join("source")).unwrap(), "source");
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 2);
        drop(reservation);
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Ok(FileUndoOutcome::Removed("file".into()))
        );
        assert_eq!(fs::read_to_string(temp.0.join("source")).unwrap(), "source");
    }

    #[test]
    fn file_undo_clear_reservation_preserves_unavailable_marker_until_commit() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        File::create(temp.0.join("file"))
            .unwrap()
            .set_len((MAX_FILE_UNDO_PREIMAGE_BYTES + 1) as u64)
            .unwrap();
        write(&temp, &tracker, "file", "after");
        let expected = Some(FileUndoUnavailableReason::PreimageTooLarge);
        assert_eq!(tracker.latest_unavailable_reason().unwrap(), expected);
        let reservation = tracker.reserve_clear().unwrap();
        drop(reservation);
        assert_eq!(tracker.latest_unavailable_reason().unwrap(), expected);
        tracker.reserve_clear().unwrap().commit();
        assert_eq!(tracker.latest_unavailable_reason().unwrap(), None);
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Ok(FileUndoOutcome::Empty)
        );
        assert_eq!(fs::read_to_string(temp.0.join("file")).unwrap(), "after");
    }

    #[test]
    fn file_undo_clear_reservation_is_send_and_commit_only_forgets_authority() {
        fn assert_send<T: Send>() {}
        assert_send::<FileUndoClearReservation>();
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        write(&temp, &tracker, "file", "retained");
        fs::write(temp.0.join(".machine-god-undo-recovery"), "do not remove").unwrap();
        let reservation = tracker.reserve_clear().unwrap();
        std::thread::spawn(move || reservation.commit())
            .join()
            .unwrap();
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Ok(FileUndoOutcome::Empty)
        );
        assert_eq!(fs::read_to_string(temp.0.join("file")).unwrap(), "retained");
        assert_eq!(
            fs::read_to_string(temp.0.join(".machine-god-undo-recovery")).unwrap(),
            "do not remove"
        );
        write(&temp, &tracker, "file", "new");
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(fs::read_to_string(temp.0.join("file")).unwrap(), "retained");
        let next = tracker.reserve_clear().unwrap();
        drop(next);
        tracker.clear().unwrap();
    }

    #[test]
    fn file_undo_clear_reservation_abort_unwind_and_poison_remain_fail_closed() {
        let temp = Temp::new();
        for commit in [false, true] {
            let tracker = Arc::new(FileUndoTracker::new());
            write(&temp, &tracker, "file", "retained");
            let reservation = tracker.reserve_clear().unwrap();
            let poisoned = tracker.clone();
            assert!(
                std::thread::spawn(move || {
                    let _state = poisoned.state.lock().unwrap();
                    panic!("deliberate private-state poison");
                })
                .join()
                .is_err()
            );
            if commit {
                reservation.commit();
            } else {
                drop(reservation);
            }
            assert!(!tracker.clear_reserved.load(Ordering::Acquire));
            assert_eq!(tracker.clear(), Err(FileUndoError::Busy));
            assert!(matches!(tracker.reserve_clear(), Err(FileUndoError::Busy)));
            assert_eq!(fs::read_to_string(temp.0.join("file")).unwrap(), "retained");
        }
        let tracker = Arc::new(FileUndoTracker::new());
        write(&temp, &tracker, "unwind", "retained");
        let reservation = tracker.reserve_clear().unwrap();
        assert!(
            std::thread::spawn(move || {
                let _reservation = reservation;
                panic!("abort handoff");
            })
            .join()
            .is_err()
        );
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Ok(FileUndoOutcome::Removed("unwind".into()))
        );
    }

    #[test]
    fn file_undo_write_creation_replacement_and_interleaved_history() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        write(&temp, &tracker, "a", "one");
        write(&temp, &tracker, "b", "other");
        write(&temp, &tracker, "a", "two");
        let cancel = CancellationToken::new();
        assert_eq!(
            tracker.undo_last(&cancel),
            Ok(FileUndoOutcome::Restored("a".into()))
        );
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"one");
        assert_eq!(
            tracker.undo_last(&cancel),
            Ok(FileUndoOutcome::Removed("b".into()))
        );
        assert_eq!(
            tracker.undo_last(&cancel),
            Ok(FileUndoOutcome::Removed("a".into()))
        );
        assert_eq!(tracker.undo_last(&cancel), Ok(FileUndoOutcome::Empty));
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 0);
    }

    #[test]
    fn file_undo_rename_survives_tracked_destination_reconstruction() {
        for delete in [false, true] {
            let temp = Temp::new();
            let tracker = Arc::new(FileUndoTracker::new());
            write(&temp, &tracker, "source", "original");
            let rename = crate::RenameFileTool::open(&temp.0)
                .unwrap()
                .with_undo_tracker(tracker.clone());
            ready(rename.execute(
                context(),
                json!({"old_path":"source","new_path":"destination"}),
                CancellationToken::new(),
            ))
            .unwrap();
            if delete {
                let tool = crate::DeleteFileTool::open(&temp.0)
                    .unwrap()
                    .with_undo_tracker(tracker.clone());
                ready(tool.execute(
                    context(),
                    json!({"path":"destination"}),
                    CancellationToken::new(),
                ))
                .unwrap();
            } else {
                write(&temp, &tracker, "destination", "replacement");
            }
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Ok(FileUndoOutcome::Restored("destination".into()))
            );
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Ok(FileUndoOutcome::Restored("source".into()))
            );
            assert_eq!(fs::read(temp.0.join("source")).unwrap(), b"original");
            assert!(!temp.0.join("destination").exists());
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Ok(FileUndoOutcome::Removed("source".into()))
            );
        }
    }

    #[test]
    fn file_undo_reconstructed_rename_rejects_external_same_content_replacement() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        write(&temp, &tracker, "source", "original");
        let rename = crate::RenameFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(rename.execute(
            context(),
            json!({"old_path":"source","new_path":"destination"}),
            CancellationToken::new(),
        ))
        .unwrap();
        write(&temp, &tracker, "destination", "replacement");
        tracker.undo_last(&CancellationToken::new()).unwrap();
        fs::write(temp.0.join("external"), b"original").unwrap();
        fs::rename(temp.0.join("external"), temp.0.join("destination")).unwrap();
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::Changed)
        );
        assert!(!temp.0.join("source").exists());
        assert_eq!(fs::read(temp.0.join("destination")).unwrap(), b"original");
    }

    #[test]
    fn file_undo_edit_delete_and_directory_restore() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        fs::write(temp.0.join("a"), b"before").unwrap();
        let edit = crate::EditFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(edit.execute(
            context(),
            json!({"path":"a","old_string":"before","new_string":"after"}),
            CancellationToken::new(),
        ))
        .unwrap();
        let delete = crate::DeleteFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(delete.execute(context(), json!({"path":"a"}), CancellationToken::new())).unwrap();
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"after");
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"before");
        fs::create_dir(temp.0.join("empty")).unwrap();
        ready(delete.execute(context(), json!({"path":"empty"}), CancellationToken::new()))
            .unwrap();
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert!(temp.0.join("empty").is_dir());
    }

    #[test]
    fn file_undo_regular_rename_and_explicit_overwritten_destination() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        fs::write(temp.0.join("a"), b"source").unwrap();
        let rename = crate::RenameFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(rename.execute(
            context(),
            json!({"old_path":"a","new_path":"b"}),
            CancellationToken::new(),
        ))
        .unwrap();
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"source");
        fs::write(temp.0.join("b"), b"destination").unwrap();
        tracker
            .rename_replace(
                &File::open(&temp.0).unwrap(),
                "a",
                "b",
                &CancellationToken::new(),
            )
            .unwrap();
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"source");
        assert_eq!(fs::read(temp.0.join("b")).unwrap(), b"destination");
    }

    #[test]
    fn file_undo_copy_creation_and_explicit_overwritten_destination() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        fs::write(temp.0.join("source"), b"\xff\0source").unwrap();
        let copy = crate::CopyFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(copy.execute(
            context(),
            json!({"source":"source","destination":"destination"}),
            CancellationToken::new(),
        ))
        .unwrap();
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert!(!temp.0.join("destination").exists());
        fs::write(temp.0.join("destination"), b"old").unwrap();
        tracker
            .copy_replace(
                &File::open(&temp.0).unwrap(),
                "source",
                "destination",
                &CancellationToken::new(),
            )
            .unwrap();
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(fs::read(temp.0.join("destination")).unwrap(), b"old");
        assert_eq!(fs::read(temp.0.join("source")).unwrap(), b"\xff\0source");
    }

    #[test]
    fn file_undo_external_modification_and_symlink_fail_closed() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        write(&temp, &tracker, "a", "tracked");
        fs::write(temp.0.join("a"), b"external").unwrap();
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::Changed)
        );
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"external");
        fs::remove_file(temp.0.join("a")).unwrap();
        fs::write(temp.0.join("sentinel"), b"safe").unwrap();
        std::os::unix::fs::symlink("sentinel", temp.0.join("a")).unwrap();
        assert!(tracker.undo_last(&CancellationToken::new()).is_err());
        assert_eq!(fs::read(temp.0.join("sentinel")).unwrap(), b"safe");
    }

    #[test]
    fn file_undo_cancel_and_unpolled_are_inert() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        let tool = crate::WriteFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        drop(tool.execute(
            context(),
            json!({"path":"a","content":"new"}),
            CancellationToken::new(),
        ));
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Ok(FileUndoOutcome::Empty)
        );
        write(&temp, &tracker, "a", "tracked");
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(tracker.undo_last(&cancel), Err(FileUndoError::Cancelled));
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"tracked");
    }

    #[test]
    fn file_undo_failed_mutation_not_recorded_and_capacity_evicts_oldest() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        let delete = crate::DeleteFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        assert!(
            ready(delete.execute(
                context(),
                json!({"path":"missing"}),
                CancellationToken::new()
            ))
            .is_err()
        );
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Ok(FileUndoOutcome::Empty)
        );
        for i in 0..101 {
            write(&temp, &tracker, &format!("file-{i}"), "content");
        }
        for _ in 0..100 {
            tracker.undo_last(&CancellationToken::new()).unwrap();
        }
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Ok(FileUndoOutcome::Empty)
        );
        assert_eq!(fs::read(temp.0.join("file-0")).unwrap(), b"content");
    }

    #[test]
    fn file_undo_oversized_preimage_allows_forward_and_stops_inverse_honestly() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        let file = File::create(temp.0.join("large")).unwrap();
        file.set_len(MAX_FILE_UNDO_PREIMAGE_BYTES as u64 + 1)
            .unwrap();
        let tool = crate::WriteFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(tool.execute(
            context(),
            json!({"path":"large","content":"replacement"}),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(fs::read(temp.0.join("large")).unwrap(), b"replacement");
        assert_eq!(
            tracker.latest_unavailable_reason().unwrap(),
            Some(FileUndoUnavailableReason::PreimageTooLarge)
        );
        write(&temp, &tracker, "a", "new");
        assert_eq!(tracker.latest_unavailable_reason().unwrap(), None);
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()).unwrap(),
            FileUndoOutcome::Removed("a".into())
        );
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::NotUndoable(
                FileUndoUnavailableReason::PreimageTooLarge
            ))
        );
        assert_eq!(fs::read(temp.0.join("large")).unwrap(), b"replacement");
        tracker.clear().unwrap();
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Ok(FileUndoOutcome::Empty)
        );
    }

    #[test]
    fn file_undo_unreadable_preimage_does_not_remove_forward_write_authority() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        fs::write(temp.0.join("private"), b"prior").unwrap();
        fs::set_permissions(temp.0.join("private"), fs::Permissions::from_mode(0o000)).unwrap();
        // A privileged test runner can still read mode 000; exercise the exact
        // unavailable-capture state deterministically in the native test below.
        let unavailable = File::open(temp.0.join("private")).is_err();
        write(&temp, &tracker, "private", "forward");
        if unavailable {
            assert_eq!(
                tracker.latest_unavailable_reason().unwrap(),
                Some(FileUndoUnavailableReason::SnapshotUnavailable)
            );
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Err(FileUndoError::NotUndoable(
                    FileUndoUnavailableReason::SnapshotUnavailable
                ))
            );
        }
        fs::set_permissions(temp.0.join("private"), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fs::read(temp.0.join("private")).unwrap(), b"forward");
    }

    #[test]
    fn file_undo_copy_preserves_full_sixteen_mib_source_contract() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        let root = File::open(&temp.0).unwrap();
        let source = File::create(temp.0.join("source")).unwrap();
        let copy = crate::CopyFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        for size in [
            MAX_FILE_UNDO_PREIMAGE_BYTES + 1,
            crate::MAX_COPY_FILE_SOURCE_BYTES,
        ] {
            source.set_len(size as u64).unwrap();
            ready(copy.execute(
                context(),
                json!({"source":"source","destination":"copy"}),
                CancellationToken::new(),
            ))
            .unwrap();
            assert_eq!(
                fs::metadata(temp.0.join("copy")).unwrap().len(),
                size as u64
            );
            assert_eq!(tracker.latest_unavailable_reason().unwrap(), None);
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()).unwrap(),
                FileUndoOutcome::Removed("copy".into())
            );
            fs::write(temp.0.join("destination"), b"prior").unwrap();
            tracker
                .copy_replace(&root, "source", "destination", &CancellationToken::new())
                .unwrap();
            assert_eq!(
                fs::metadata(temp.0.join("destination")).unwrap().len(),
                size as u64
            );
            tracker.undo_last(&CancellationToken::new()).unwrap();
            assert_eq!(fs::read(temp.0.join("destination")).unwrap(), b"prior");
            assert_eq!(
                fs::metadata(temp.0.join("source")).unwrap().len(),
                size as u64
            );
        }
    }

    #[test]
    fn file_undo_oversized_delete_and_replacement_copy_keep_nonundoable_history() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        let root = File::open(&temp.0).unwrap();
        let oversized = File::create(temp.0.join("destination")).unwrap();
        oversized
            .set_len(MAX_FILE_UNDO_PREIMAGE_BYTES as u64 + 1)
            .unwrap();
        fs::write(temp.0.join("source"), b"forward").unwrap();
        tracker
            .copy_replace(&root, "source", "destination", &CancellationToken::new())
            .unwrap();
        assert_eq!(fs::read(temp.0.join("destination")).unwrap(), b"forward");
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::NotUndoable(
                FileUndoUnavailableReason::PreimageTooLarge
            ))
        );
        tracker.clear().unwrap();
        File::create(temp.0.join("large"))
            .unwrap()
            .set_len(MAX_FILE_UNDO_PREIMAGE_BYTES as u64 + 1)
            .unwrap();
        let delete = crate::DeleteFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(delete.execute(context(), json!({"path":"large"}), CancellationToken::new()))
            .unwrap();
        assert!(!temp.0.join("large").exists());
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::NotUndoable(
                FileUndoUnavailableReason::PreimageTooLarge
            ))
        );
    }

    #[test]
    fn file_undo_large_rename_moves_exact_object_without_source_capture() {
        use std::os::unix::fs::MetadataExt;
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        let source = File::create(temp.0.join("source")).unwrap();
        source
            .set_len((MAX_FILE_UNDO_OBSERVATION_BYTES * 2) as u64)
            .unwrap();
        let identity = source.metadata().unwrap().ino();
        let rename = crate::RenameFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(rename.execute(
            context(),
            json!({"old_path":"source","new_path":"destination"}),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(tracker.latest_unavailable_reason().unwrap(), None);
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(fs::metadata(temp.0.join("source")).unwrap().ino(), identity);
        assert_eq!(
            fs::metadata(temp.0.join("source")).unwrap().len(),
            (MAX_FILE_UNDO_OBSERVATION_BYTES * 2) as u64
        );
        ready(rename.execute(
            context(),
            json!({"old_path":"source","new_path":"destination"}),
            CancellationToken::new(),
        ))
        .unwrap();
        source
            .set_len((MAX_FILE_UNDO_OBSERVATION_BYTES * 2 + 1) as u64)
            .unwrap();
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::Changed)
        );
        assert!(!temp.0.join("source").exists());
        ready(rename.execute(
            context(),
            json!({"old_path":"destination","new_path":"third"}),
            CancellationToken::new(),
        ))
        .unwrap();
        tracker.undo_last(&CancellationToken::new()).unwrap();
        // Undoing a later tracked rename must not rebase an older observation
        // over the intervening external size/timestamp change.
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::Changed)
        );
        assert!(temp.0.join("destination").exists());
    }

    #[test]
    fn file_undo_unreadable_rename_source_keeps_forward_dispatch() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        fs::write(temp.0.join("source"), b"unreadable").unwrap();
        fs::set_permissions(temp.0.join("source"), fs::Permissions::from_mode(0o000)).unwrap();
        fs::write(temp.0.join("baseline"), b"unreadable").unwrap();
        fs::set_permissions(temp.0.join("baseline"), fs::Permissions::from_mode(0o000)).unwrap();
        let unavailable = File::open(temp.0.join("source")).is_err();
        let baseline = ready(crate::RenameFileTool::open(&temp.0).unwrap().execute(
            context(),
            json!({"old_path":"baseline","new_path":"baseline_moved"}),
            CancellationToken::new(),
        ));
        let rename = crate::RenameFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        let outcome = ready(rename.execute(
            context(),
            json!({"old_path":"source","new_path":"destination"}),
            CancellationToken::new(),
        ));
        if let Err(baseline) = baseline {
            // Some platforms deny even metadata-only source pinning. Tracking
            // must preserve that delivered denial, not broaden rename authority.
            assert_eq!(outcome.unwrap_err().code, baseline.code);
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()).unwrap(),
                FileUndoOutcome::Empty
            );
            return;
        }
        outcome.unwrap();
        if unavailable {
            assert_eq!(
                tracker.latest_unavailable_reason().unwrap(),
                Some(FileUndoUnavailableReason::SnapshotUnavailable)
            );
        }
        let root = File::open(&temp.0).unwrap();
        tracker
            .rename_replace(&root, "destination", "replaced", &CancellationToken::new())
            .unwrap();
        if unavailable {
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Err(FileUndoError::NotUndoable(
                    FileUndoUnavailableReason::SnapshotUnavailable
                ))
            );
        }
        assert!(!temp.0.join("source").exists());
        assert!(!temp.0.join("destination").exists());
        assert!(temp.0.join("replaced").exists());
    }

    #[test]
    fn file_undo_parent_replacement_cannot_redirect_retained_authority() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        fs::create_dir(temp.0.join("parent")).unwrap();
        write(&temp, &tracker, "parent/a", "tracked");
        fs::rename(temp.0.join("parent"), temp.0.join("moved")).unwrap();
        fs::create_dir(temp.0.join("parent")).unwrap();
        fs::write(temp.0.join("parent/a"), b"sentinel").unwrap();
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::Changed)
        );
        assert_eq!(fs::read(temp.0.join("parent/a")).unwrap(), b"sentinel");
        assert_eq!(fs::read(temp.0.join("moved/a")).unwrap(), b"tracked");
    }

    #[test]
    fn file_undo_preserves_ordinary_modes_without_restoring_special_bits() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        fs::write(temp.0.join("executable"), b"original").unwrap();
        fs::set_permissions(temp.0.join("executable"), fs::Permissions::from_mode(0o751)).unwrap();
        write(&temp, &tracker, "executable", "new");
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(
            fs::metadata(temp.0.join("executable"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o751
        );
        let delete = crate::DeleteFileTool::open(&temp.0)
            .unwrap()
            .with_undo_tracker(tracker.clone());
        ready(delete.execute(
            context(),
            json!({"path":"executable"}),
            CancellationToken::new(),
        ))
        .unwrap();
        tracker.undo_last(&CancellationToken::new()).unwrap();
        assert_eq!(
            fs::metadata(temp.0.join("executable"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o751
        );
    }

    #[test]
    fn file_undo_same_bytes_leaf_replacement_still_fails_closed() {
        let temp = Temp::new();
        let tracker = Arc::new(FileUndoTracker::new());
        write(&temp, &tracker, "a", "same");
        fs::write(temp.0.join("replacement"), b"same").unwrap();
        fs::rename(temp.0.join("replacement"), temp.0.join("a")).unwrap();
        assert_eq!(
            tracker.undo_last(&CancellationToken::new()),
            Err(FileUndoError::Changed)
        );
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"same");
    }

    #[test]
    fn file_undo_explicit_copy_and_rename_reject_same_inode_aliases() {
        let temp = Temp::new();
        let tracker = FileUndoTracker::new();
        fs::write(temp.0.join("a"), b"same-object").unwrap();
        fs::hard_link(temp.0.join("a"), temp.0.join("b")).unwrap();
        let root = File::open(&temp.0).unwrap();
        assert_eq!(
            tracker.copy_replace(&root, "a", "b", &CancellationToken::new()),
            Err(FileUndoError::Rejected)
        );
        assert_eq!(
            tracker.rename_replace(&root, "a", "b", &CancellationToken::new()),
            Err(FileUndoError::Rejected)
        );
        assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"same-object");
        assert_eq!(fs::read(temp.0.join("b")).unwrap(), b"same-object");
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 2);
    }
}

/// Fixed, redacted undo failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileUndoError {
    /// Another cooperating mutation currently owns the tracker.
    Busy,
    /// The supplied path or root is invalid or unsupported.
    Rejected,
    /// An observed resource changed; no safe replay is available.
    Changed,
    /// A bounded snapshot or allocation exceeded its limit.
    ResourceLimit,
    /// Filesystem work failed before any undo publication.
    Unavailable,
    /// Cancellation was observed before irreversible work.
    Cancelled,
    /// Publication may have occurred; manual observation is required.
    Ambiguous,
    /// Forward publication succeeded, but its inverse could not be captured.
    NotUndoable(FileUndoUnavailableReason),
}

/// Fixed explanation for a committed operation that cannot safely be undone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileUndoUnavailableReason {
    /// The prior file exceeds the per-preimage capture bound.
    PreimageTooLarge,
    /// The required snapshot could not be obtained safely.
    SnapshotUnavailable,
}

impl fmt::Display for FileUndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "file undo tracker is busy",
            Self::Rejected => "file undo authority is unavailable",
            Self::Changed => "tracked file state changed",
            Self::ResourceLimit => "file undo resource limit exceeded",
            Self::Unavailable => "file undo is unavailable",
            Self::Cancelled => "file undo was cancelled",
            Self::Ambiguous => {
                "file undo outcome is uncertain; inspect retained recovery artifacts"
            }
            Self::NotUndoable(FileUndoUnavailableReason::PreimageTooLarge) => {
                "committed operation is not undoable: prior file exceeds the preimage limit"
            }
            Self::NotUndoable(FileUndoUnavailableReason::SnapshotUnavailable) => {
                "committed operation is not undoable: required snapshot was unavailable"
            }
        })
    }
}
impl std::error::Error for FileUndoError {}

impl FileUndoError {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn tool(self) -> ToolError {
        let kind = if self == Self::Cancelled {
            ToolErrorKind::Cancelled
        } else {
            ToolErrorKind::Execution
        };
        ToolError::new(
            kind,
            "file_undo_tracking_failed",
            "file mutation undo tracking failed",
            false,
        )
    }
}

/// Successful process-local undo observation. Routed endpoints retain their
/// original logical labels; standalone operations use workspace-relative paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileUndoOutcome {
    /// No retained operations exist.
    Empty,
    /// A prior file, directory, or rename source was restored.
    Restored(String),
    /// The file created by the tracked operation was removed.
    Removed(String),
}

/// Optional shared tracker. Construction is inert; injection grants bounded
/// preimage reads and undo authority in addition to ordinary tool authority.
#[derive(Default)]
pub struct FileUndoTracker {
    clear_reserved: AtomicBool,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    state: Mutex<native::State>,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    state: Mutex<()>,
}

impl fmt::Debug for FileUndoTracker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileUndoTracker").finish_non_exhaustive()
    }
}

/// Exclusive owned admission for forgetting undo history. It holds no mutex
/// guard and may cross an await or move to another thread. Dropping without
/// committing releases admission without changing entries or barriers.
#[must_use]
pub struct FileUndoClearReservation {
    tracker: Arc<FileUndoTracker>,
    active: bool,
}

impl fmt::Debug for FileUndoClearReservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileUndoClearReservation { .. }")
    }
}

impl FileUndoClearReservation {
    /// Infallibly forgets retained entries and barriers, without modifying files
    /// or deleting recovery artifacts. Retained descriptors drop outside the
    /// tracker mutex. Existing mutex poison remains fail-closed for later calls.
    pub fn commit(mut self) {
        let mut state = self
            .tracker
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let previous = std::mem::take(&mut *state);
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            *state = ();
        }
        self.active = false;
        self.tracker.clear_reserved.store(false, Ordering::Release);
        drop(state);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        drop(previous);
    }
}

impl Drop for FileUndoClearReservation {
    fn drop(&mut self) {
        if self.active {
            let _state = self
                .tracker
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.tracker.clear_reserved.store(false, Ordering::Release);
        }
    }
}

impl FileUndoTracker {
    /// Reserves exclusive clear admission without forgetting history or touching
    /// files. The returned owned, nonclone reservation is `Send`.
    ///
    /// # Errors
    /// Returns `Busy` for active tracker work, another reservation or poison.
    pub fn reserve_clear(self: &Arc<Self>) -> Result<FileUndoClearReservation, FileUndoError> {
        let _state = self.state.try_lock().map_err(|_| FileUndoError::Busy)?;
        self.check_clear_reservation()?;
        self.clear_reserved.store(true, Ordering::Release);
        Ok(FileUndoClearReservation {
            tracker: Arc::clone(self),
            active: true,
        })
    }

    // Native callers check this only while holding the tracker state mutex,
    // making reservation and every tracked effect's admission mutually exclusive.
    fn check_clear_reservation(&self) -> Result<(), FileUndoError> {
        if self.clear_reserved.load(Ordering::Acquire) {
            Err(FileUndoError::Busy)
        } else {
            Ok(())
        }
    }

    /// Copies a regular file over an absent or regular destination under explicit
    /// native authority, retaining the destination's preimage. Unlike the default
    /// copy tool this explicitly allows replacement. Both snapshots are bounded.
    ///
    /// # Errors
    /// Returns fixed snapshot, authority, cancellation, or publication errors.
    pub fn copy_replace(
        &self,
        workspace: &std::fs::File,
        source: &str,
        destination: &str,
        cancellation: &CancellationToken,
    ) -> Result<(), FileUndoError> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            native::copy_replace(self, workspace, source, destination, cancellation)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (workspace, source, destination, cancellation);
            self.check_clear_reservation()?;
            Err(FileUndoError::Rejected)
        }
    }
    /// Creates an empty, effect-inert tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets retained undo authority without changing files.
    ///
    /// # Errors
    /// Returns `Busy` if another operation owns the tracker.
    pub fn clear(&self) -> Result<(), FileUndoError> {
        let mut state = self.state.try_lock().map_err(|_| FileUndoError::Busy)?;
        self.check_clear_reservation()?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            *state = native::State::default();
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            *state = ();
        }
        Ok(())
    }

    /// Reports why the latest proven committed mutation lacks a safe inverse.
    /// This does not consume history or perform filesystem work.
    ///
    /// # Errors
    /// Returns `Busy` during another operation, or `Ambiguous` for uncertain work.
    pub fn latest_unavailable_reason(
        &self,
    ) -> Result<Option<FileUndoUnavailableReason>, FileUndoError> {
        let state = self.state.try_lock().map_err(|_| FileUndoError::Busy)?;
        self.check_clear_reservation()?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            if state.ambiguous {
                return Err(FileUndoError::Ambiguous);
            }
            Ok(state.latest_unavailable_reason())
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            drop(state);
            Ok(None)
        }
    }

    /// Undoes the latest committed operation using retained native authority.
    /// No task is spawned; cancellation cannot interrupt an in-flight syscall.
    ///
    /// # Errors
    /// Changed targets fail closed. After the first native rename/mkdir,
    /// failures are ambiguous and the entry cannot be replayed automatically.
    pub fn undo_last(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<FileUndoOutcome, FileUndoError> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            native::undo(self, cancellation)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = cancellation;
            self.check_clear_reservation()?;
            Err(FileUndoError::Rejected)
        }
    }

    /// Explicitly renames a regular source over an absent or regular destination,
    /// retaining both inverse effects. This does not change `RenameFileTool`'s
    /// no-overwrite schema or defaults. The supplied open directory is authority.
    ///
    /// # Errors
    /// Rejects unsafe paths, changed state, snapshot overflow, cancellation, or
    /// unavailable effects. An uncertain publication returns `Ambiguous`.
    pub fn rename_replace(
        &self,
        workspace: &std::fs::File,
        old_path: &str,
        new_path: &str,
        cancellation: &CancellationToken,
    ) -> Result<(), FileUndoError> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            native::rename_replace(self, workspace, old_path, new_path, cancellation)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (workspace, old_path, new_path, cancellation);
            self.check_clear_reservation()?;
            Err(FileUndoError::Rejected)
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use native::Operation;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod native {
    use super::{
        CancellationToken, FileUndoError, FileUndoOutcome, FileUndoTracker,
        FileUndoUnavailableReason, MAX_FILE_UNDO_ENTRIES, MAX_FILE_UNDO_OBSERVATION_BYTES,
        MAX_FILE_UNDO_PREIMAGE_BYTES,
    };
    use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags, Stat};
    use sha2::{Digest, Sha256};
    use std::collections::VecDeque;
    use std::fmt::Write;
    use std::sync::MutexGuard;

    #[derive(Default)]
    pub(super) struct State {
        entries: VecDeque<Entry>,
        bytes: usize,
        pub(super) ambiguous: bool,
    }
    impl State {
        pub(super) fn latest_unavailable_reason(&self) -> Option<FileUndoUnavailableReason> {
            self.entries.back().and_then(|entry| {
                entry.before.iter().find_map(|snapshot| match snapshot {
                    Snapshot::Unavailable(reason) => Some(*reason),
                    _ => None,
                })
            })
        }
    }

    #[derive(Clone, Copy)]
    pub(crate) enum Operation<'a> {
        Replace(&'a str),
        Delete(&'a str),
        Rename(&'a str, &'a str),
    }

    struct Entry {
        paths: Vec<Location>,
        before: Vec<Snapshot>,
        after: Vec<Snapshot>,
        rename: bool,
        blocked: bool,
    }
    struct Location {
        root: OwnedFd,
        path: String,
        logical_path: String,
        parent: OwnedFd,
        name: String,
    }
    enum Snapshot {
        Missing,
        Unavailable(FileUndoUnavailableReason),
        Directory {
            fd: OwnedFd,
            stat: Stat,
        },
        File {
            fd: OwnedFd,
            stat: Stat,
            digest: Option<[u8; 32]>,
            bytes: Vec<u8>,
        },
    }
    impl Snapshot {
        fn bytes(&self) -> usize {
            match self {
                Self::File { bytes, .. } => bytes.len(),
                _ => 0,
            }
        }
        fn fd(&self) -> Option<BorrowedFd<'_>> {
            match self {
                Self::Missing | Self::Unavailable(_) => None,
                Self::Directory { fd, .. } | Self::File { fd, .. } => Some(fd.as_fd()),
            }
        }
    }
    impl Entry {
        fn bytes(&self) -> usize {
            self.before.iter().map(Snapshot::bytes).sum()
        }
    }

    pub(crate) struct Transaction<'a> {
        state: MutexGuard<'a, State>,
        entry: Option<Entry>,
    }

    impl FileUndoTracker {
        pub(crate) fn begin(
            &self,
            root: BorrowedFd<'_>,
            operation: Operation<'_>,
            cancellation: &CancellationToken,
        ) -> Result<Transaction<'_>, FileUndoError> {
            check(cancellation)?;
            if matches!(operation, Operation::Rename(old, new) if old == new) {
                return Err(FileUndoError::Rejected);
            }
            self.begin_roots(root, root, operation, None, cancellation)
        }

        pub(crate) fn begin_copy(
            &self,
            target: &crate::file_approval::NativeFileEndpoint,
            cancellation: &CancellationToken,
        ) -> Result<Transaction<'_>, FileUndoError> {
            self.begin_roots(
                target.root().as_fd(),
                target.root().as_fd(),
                Operation::Replace(target.relative_path()),
                Some((target.logical_path(), target.logical_path())),
                cancellation,
            )
        }

        pub(crate) fn begin_rename(
            &self,
            source: &crate::file_approval::NativeFileEndpoint,
            target: &crate::file_approval::NativeFileEndpoint,
            cancellation: &CancellationToken,
        ) -> Result<Transaction<'_>, FileUndoError> {
            self.begin_roots(
                source.root().as_fd(),
                target.root().as_fd(),
                Operation::Rename(source.relative_path(), target.relative_path()),
                Some((source.logical_path(), target.logical_path())),
                cancellation,
            )
        }

        fn begin_roots(
            &self,
            root: BorrowedFd<'_>,
            destination_root: BorrowedFd<'_>,
            operation: Operation<'_>,
            labels: Option<(&str, &str)>,
            cancellation: &CancellationToken,
        ) -> Result<Transaction<'_>, FileUndoError> {
            check(cancellation)?;
            let mut state = self.state.try_lock().map_err(|_| FileUndoError::Busy)?;
            self.check_clear_reservation()?;
            if state.ambiguous {
                return Err(FileUndoError::Ambiguous);
            }
            state
                .entries
                .try_reserve(1)
                .map_err(|_| FileUndoError::ResourceLimit)?;
            let (mut paths, rename) = match operation {
                Operation::Replace(path) | Operation::Delete(path) => {
                    (vec![locate(root, path)?], false)
                }
                Operation::Rename(old, new) => (
                    vec![locate(root, old)?, locate(destination_root, new)?],
                    true,
                ),
            };
            if rename
                && paths[0].name == paths[1].name
                && identity(
                    &rustix::fs::fstat(&paths[0].parent).map_err(unavailable)?,
                    &rustix::fs::fstat(&paths[1].parent).map_err(unavailable)?,
                )
            {
                return Err(FileUndoError::Rejected);
            }
            if let Some((source, target)) = labels {
                source.clone_into(&mut paths[0].logical_path);
                if rename {
                    target.clone_into(&mut paths[1].logical_path);
                }
            }
            let mut before = Vec::with_capacity(paths.len());
            for (index, path) in paths.iter().enumerate() {
                let snapshot = match capture(path, !rename || index == 1, cancellation) {
                    Ok(snapshot) => snapshot,
                    Err(FileUndoError::Cancelled) => return Err(FileUndoError::Cancelled),
                    Err(FileUndoError::ResourceLimit)
                        if (!rename || index == 1)
                            && rustix::fs::statat(
                                &path.parent,
                                path.name.as_str(),
                                AtFlags::SYMLINK_NOFOLLOW,
                            )
                            .is_ok_and(|stat| {
                                usize::try_from(stat.st_size)
                                    .is_ok_and(|size| size > MAX_FILE_UNDO_PREIMAGE_BYTES)
                            }) =>
                    {
                        Snapshot::Unavailable(FileUndoUnavailableReason::PreimageTooLarge)
                    }
                    Err(_) => Snapshot::Unavailable(FileUndoUnavailableReason::SnapshotUnavailable),
                };
                before.push(snapshot);
            }
            if rename
                && (!matches!(before[0], Snapshot::File { .. } | Snapshot::Unavailable(_))
                    || matches!(before[1], Snapshot::Directory { .. }))
            {
                return Err(FileUndoError::Rejected);
            }
            if rename
                && let (Some(source), Some(destination)) = (before[0].fd(), before[1].fd())
                && identity(
                    &rustix::fs::fstat(source).map_err(unavailable)?,
                    &rustix::fs::fstat(destination).map_err(unavailable)?,
                )
            {
                return Err(FileUndoError::Rejected);
            }
            Ok(Transaction {
                state,
                entry: Some(Entry {
                    paths,
                    before,
                    after: Vec::new(),
                    rename,
                    blocked: false,
                }),
            })
        }
    }

    impl Transaction<'_> {
        pub(crate) fn uncertain(&mut self) {
            self.state.ambiguous = true;
        }
        pub(crate) fn revalidate(
            &self,
            cancellation: &CancellationToken,
        ) -> Result<(), FileUndoError> {
            let entry = self.entry.as_ref().ok_or(FileUndoError::Ambiguous)?;
            validate_locations(entry)?;
            for (path, expected) in entry.paths.iter().zip(&entry.before) {
                if !matches!(expected, Snapshot::Unavailable(_)) {
                    verify(path, expected, cancellation)?;
                }
            }
            check(cancellation)
        }

        /// Called only after a syscall has positively reported publication.
        /// Pins the actual published inode; snapshot failure creates a barrier.
        pub(crate) fn committed(&mut self, published: Option<BorrowedFd<'_>>) {
            let Some(mut entry) = self.entry.take() else {
                return;
            };
            let no_cancel = CancellationToken::new();
            // A proven successful dispatch still occupies history when capture
            // was unavailable. It must never be mistaken for file creation.
            if entry
                .before
                .iter()
                .any(|s| matches!(s, Snapshot::Unavailable(_)))
            {
                self.push(entry);
                return;
            }
            let observation = (|| {
                for path in &entry.paths {
                    entry.after.push(capture(path, false, &no_cancel)?);
                }
                if let Some(published) = published {
                    let index = usize::from(entry.rename);
                    let actual = entry.after[index].fd().ok_or(FileUndoError::Changed)?;
                    if !identity(
                        &rustix::fs::fstat(published).map_err(unavailable)?,
                        &rustix::fs::fstat(actual).map_err(unavailable)?,
                    ) {
                        return Err(FileUndoError::Changed);
                    }
                } else if !matches!(entry.after[0], Snapshot::Missing) {
                    return Err(FileUndoError::Changed);
                }
                if entry.rename && !matches!(entry.after[0], Snapshot::Missing) {
                    return Err(FileUndoError::Changed);
                }
                Ok::<(), FileUndoError>(())
            })();
            entry.blocked = observation.is_err();
            if entry.blocked {
                self.state.ambiguous = true;
            }
            self.push(entry);
        }

        fn push(&mut self, entry: Entry) {
            let bytes = entry.bytes();
            while self.state.entries.len() >= MAX_FILE_UNDO_ENTRIES {
                if let Some(old) = self.state.entries.pop_front() {
                    self.state.bytes -= old.bytes();
                } else {
                    break;
                }
            }
            self.state.bytes += bytes;
            self.state.entries.push_back(entry);
        }
    }

    fn flags() -> OFlags {
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
    }
    fn unavailable(_: rustix::io::Errno) -> FileUndoError {
        FileUndoError::Unavailable
    }
    fn check(cancellation: &CancellationToken) -> Result<(), FileUndoError> {
        if cancellation.is_cancelled() {
            Err(FileUndoError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn identity(a: &Stat, b: &Stat) -> bool {
        a.st_dev == b.st_dev
            && a.st_ino == b.st_ino
            && FileType::from_raw_mode(a.st_mode) == FileType::from_raw_mode(b.st_mode)
    }
    fn stable(a: &Stat, b: &Stat) -> bool {
        identity(a, b)
            && a.st_mode == b.st_mode
            && a.st_size == b.st_size
            && a.st_mtime == b.st_mtime
            && a.st_mtime_nsec == b.st_mtime_nsec
            && a.st_ctime == b.st_ctime
            && a.st_ctime_nsec == b.st_ctime_nsec
    }

    fn locate(root: BorrowedFd<'_>, path: &str) -> Result<Location, FileUndoError> {
        if path.is_empty() || path.len() > 4096 || path.starts_with('/') || path.split('/').count() > 256 || path.split('/').any(|p| p.is_empty() || p == "." || p == "..") || path.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) { return Err(FileUndoError::Rejected); }
        let metadata = rustix::fs::fstat(root).map_err(unavailable)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() || metadata.st_nlink == 0 {
            return Err(FileUndoError::Rejected);
        }
        #[cfg(target_os = "macos")]
        {
            let root_path = rustix::fs::getpath(root).map_err(unavailable)?;
            if root_path.as_bytes() != b"/" {
                let name = root_path
                    .as_bytes()
                    .rsplit(|b| *b == b'/')
                    .next()
                    .filter(|s| !s.is_empty())
                    .ok_or(FileUndoError::Rejected)?;
                let name = std::ffi::CString::new(name).map_err(|_| FileUndoError::Rejected)?;
                let ancestor =
                    rustix::fs::openat(root, "..", flags(), Mode::empty()).map_err(unavailable)?;
                let linked = rustix::fs::statat(&ancestor, &name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(unavailable)?;
                if !identity(&metadata, &linked) {
                    return Err(FileUndoError::Changed);
                }
            }
        }
        let mut parent =
            rustix::fs::openat(root, ".", flags(), Mode::empty()).map_err(unavailable)?;
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                return Ok(Location {
                    root: rustix::io::fcntl_dupfd_cloexec(root, 3).map_err(unavailable)?,
                    path: path.to_owned(),
                    logical_path: path.to_owned(),
                    parent,
                    name: component.to_owned(),
                });
            }
            parent = rustix::fs::openat(&parent, component, flags(), Mode::empty())
                .map_err(unavailable)?;
        }
        Err(FileUndoError::Rejected)
    }

    fn validate_locations(entry: &Entry) -> Result<(), FileUndoError> {
        for path in &entry.paths {
            let current = locate(path.root.as_fd(), &path.path)?;
            if !identity(
                &rustix::fs::fstat(&current.parent).map_err(unavailable)?,
                &rustix::fs::fstat(&path.parent).map_err(unavailable)?,
            ) {
                return Err(FileUndoError::Changed);
            }
        }
        Ok(())
    }

    fn capture(
        path: &Location,
        retain: bool,
        cancellation: &CancellationToken,
    ) -> Result<Snapshot, FileUndoError> {
        check(cancellation)?;
        let stat =
            match rustix::fs::statat(&path.parent, path.name.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(rustix::io::Errno::NOENT) => return Ok(Snapshot::Missing),
                Err(_) => return Err(FileUndoError::Unavailable),
            };
        let kind = FileType::from_raw_mode(stat.st_mode);
        if !kind.is_file() && !kind.is_dir() {
            return Err(FileUndoError::Rejected);
        }
        let open_flags = if kind.is_dir() {
            flags()
        } else if !retain
            && usize::try_from(stat.st_size)
                .is_ok_and(|size| size > MAX_FILE_UNDO_OBSERVATION_BYTES)
        {
            crate::rename_file::source_open_flags()
        } else {
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
        };
        let fd = rustix::fs::openat(&path.parent, path.name.as_str(), open_flags, Mode::empty())
            .map_err(unavailable)?;
        let held = rustix::fs::fstat(&fd).map_err(unavailable)?;
        if !identity(&stat, &held) {
            return Err(FileUndoError::Changed);
        }
        if kind.is_dir() {
            return Ok(Snapshot::Directory { fd, stat: held });
        }
        if retain
            && usize::try_from(held.st_size).is_ok_and(|size| size > MAX_FILE_UNDO_PREIMAGE_BYTES)
        {
            return Err(FileUndoError::ResourceLimit);
        }
        // Large rename sources need no byte preimage: their exact retained
        // object moves back. Preserve that operation without unbounded hashing.
        let (digest, bytes) = if !retain
            && usize::try_from(held.st_size)
                .is_ok_and(|size| size > MAX_FILE_UNDO_OBSERVATION_BYTES)
        {
            (None, Vec::new())
        } else {
            let (digest, bytes) = read(fd.as_fd(), retain, cancellation)?;
            (Some(digest), bytes)
        };
        let after = rustix::fs::fstat(&fd).map_err(unavailable)?;
        let named = rustix::fs::statat(&path.parent, path.name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(unavailable)?;
        if !stable(&held, &after) || !identity(&after, &named) {
            return Err(FileUndoError::Changed);
        }
        Ok(Snapshot::File {
            fd,
            stat: after,
            digest,
            bytes,
        })
    }

    fn read(
        fd: BorrowedFd<'_>,
        retain: bool,
        cancellation: &CancellationToken,
    ) -> Result<([u8; 32], Vec<u8>), FileUndoError> {
        let mut bytes = Vec::new();
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 8192];
        let mut offset = 0usize;
        let mut interruptions = 0;
        let maximum = if retain {
            MAX_FILE_UNDO_PREIMAGE_BYTES
        } else {
            MAX_FILE_UNDO_OBSERVATION_BYTES
        };
        loop {
            check(cancellation)?;
            // Retained preimages never read beyond 10 MiB. Their surrounding
            // stable size/identity checks detect a growth race. Digest-only
            // observation may use a single bounded overflow witness.
            let limit = buffer.len().min(maximum + usize::from(!retain) - offset);
            let result = rustix::io::pread(fd, &mut buffer[..limit], offset as u64);
            check(cancellation)?;
            match result {
                Ok(0) => break,
                Ok(n) => {
                    offset += n;
                    if offset > maximum {
                        return Err(FileUndoError::ResourceLimit);
                    }
                    hash.update(&buffer[..n]);
                    if retain {
                        bytes
                            .try_reserve(n)
                            .map_err(|_| FileUndoError::ResourceLimit)?;
                        bytes.extend_from_slice(&buffer[..n]);
                    }
                }
                Err(rustix::io::Errno::INTR) if interruptions < 15 => interruptions += 1,
                Err(_) => return Err(FileUndoError::Unavailable),
            }
        }
        Ok((hash.finalize().into(), bytes))
    }

    fn verify(
        path: &Location,
        expected: &Snapshot,
        cancellation: &CancellationToken,
    ) -> Result<(), FileUndoError> {
        let current = capture(path, false, cancellation)?;
        let matches = match (expected, &current) {
            (Snapshot::Missing, Snapshot::Missing) => true,
            (Snapshot::Directory { stat: a, .. }, Snapshot::Directory { stat: b, .. }) => {
                identity(a, b) && a.st_mode == b.st_mode
            }
            (
                Snapshot::File {
                    stat: a,
                    digest: da,
                    ..
                },
                Snapshot::File {
                    stat: b,
                    digest: db,
                    ..
                },
            ) => stable(a, b) && da == db,
            _ => false,
        };
        if matches {
            Ok(())
        } else {
            Err(FileUndoError::Changed)
        }
    }

    fn sync(fd: BorrowedFd<'_>) -> Result<(), FileUndoError> {
        for _ in 0..16 {
            match rustix::fs::fsync(fd) {
                Ok(()) => return Ok(()),
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => break,
            }
        }
        Err(FileUndoError::Ambiguous)
    }

    fn temporary() -> Result<String, FileUndoError> {
        let mut random = [0u8; 16];
        #[cfg(target_os = "macos")]
        getrandom::fill(&mut random).map_err(|_| FileUndoError::Unavailable)?;
        #[cfg(target_os = "linux")]
        {
            let mut offset = 0;
            for _ in 0..31 {
                match rustix::rand::getrandom(
                    &mut random[offset..],
                    rustix::rand::GetRandomFlags::NONBLOCK,
                ) {
                    Ok(0) => return Err(FileUndoError::Unavailable),
                    Ok(n) => offset += n,
                    Err(rustix::io::Errno::INTR) => {}
                    Err(_) => return Err(FileUndoError::Unavailable),
                }
                if offset == random.len() {
                    break;
                }
            }
            if offset != random.len() {
                return Err(FileUndoError::Unavailable);
            }
        }
        let mut name = String::from(".machine-god-undo-");
        for byte in random {
            write!(name, "{byte:02x}").map_err(|_| FileUndoError::Unavailable)?;
        }
        Ok(name)
    }

    /// Move aside without replacing any name; never unlink a mismatched object.
    fn quarantine(path: &Location, expected: &Snapshot) -> Result<String, FileUndoError> {
        let no_cancel = CancellationToken::new();
        for _ in 0..8 {
            let name = temporary()?;
            if name == path.name {
                continue;
            }
            match rustix::fs::renameat_with(
                &path.parent,
                path.name.as_str(),
                &path.parent,
                name.as_str(),
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    let moved = Location {
                        root: rustix::io::fcntl_dupfd_cloexec(&path.root, 3)
                            .map_err(|_| FileUndoError::Ambiguous)?,
                        logical_path: path.logical_path.clone(),
                        path: path.path.clone(),
                        parent: rustix::io::dup(&path.parent)
                            .map_err(|_| FileUndoError::Ambiguous)?,
                        name: name.clone(),
                    };
                    // rename changes ctime, so require pinned identity, bytes and mode,
                    // not the pre-rename ctime stamp here.
                    let matches = match (expected, capture(&moved, false, &no_cancel)) {
                        (
                            Snapshot::File {
                                stat: a,
                                digest: da,
                                ..
                            },
                            Ok(Snapshot::File {
                                stat: b,
                                digest: db,
                                ..
                            }),
                        ) => {
                            identity(a, &b)
                                && a.st_mode == b.st_mode
                                && a.st_size == b.st_size
                                && a.st_mtime == b.st_mtime
                                && a.st_mtime_nsec == b.st_mtime_nsec
                                && da == &db
                        }
                        _ => false,
                    };
                    if !matches {
                        let _ = rustix::fs::renameat_with(
                            &path.parent,
                            name.as_str(),
                            &path.parent,
                            path.name.as_str(),
                            RenameFlags::NOREPLACE,
                        );
                        let _ = sync(path.parent.as_fd());
                        return Err(FileUndoError::Ambiguous);
                    }
                    return Ok(name);
                }
                Err(rustix::io::Errno::EXIST) => {}
                Err(_) => return Err(FileUndoError::Ambiguous),
            }
        }
        Err(FileUndoError::Unavailable)
    }

    struct StageCleanup<'a> {
        parent: BorrowedFd<'a>,
        file: BorrowedFd<'a>,
        name: &'a str,
        published: bool,
        directory: bool,
    }
    impl Drop for StageCleanup<'_> {
        fn drop(&mut self) {
            if self.published {
                return;
            }
            let _ = rustix::fs::fchmod(
                self.file,
                Mode::from_raw_mode(if self.directory { 0o700 } else { 0o600 }),
            );
            if let (Ok(held), Ok(named)) = (
                rustix::fs::fstat(self.file),
                rustix::fs::statat(self.parent, self.name, AtFlags::SYMLINK_NOFOLLOW),
            ) && identity(&held, &named)
            {
                let _ = rustix::fs::unlinkat(
                    self.parent,
                    self.name,
                    if self.directory {
                        AtFlags::REMOVEDIR
                    } else {
                        AtFlags::empty()
                    },
                );
            }
        }
    }

    fn restore(path: &Location, before: &Snapshot) -> Result<Option<OwnedFd>, FileUndoError> {
        restore_with_source(path, before, None, false)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one ordered staging and publication protocol"
    )]
    fn restore_with_source(
        path: &Location,
        before: &Snapshot,
        source: Option<BorrowedFd<'_>>,
        replace: bool,
    ) -> Result<Option<OwnedFd>, FileUndoError> {
        match before {
            Snapshot::Missing => Ok(None),
            Snapshot::Unavailable(reason) => Err(FileUndoError::NotUndoable(*reason)),
            Snapshot::Directory { stat, .. } => {
                let mut staged = None;
                for _ in 0..8 {
                    let name = temporary()?;
                    if name == path.name {
                        continue;
                    }
                    match rustix::fs::mkdirat(
                        &path.parent,
                        name.as_str(),
                        Mode::from_raw_mode(0o700),
                    ) {
                        Ok(()) => {
                            staged = Some(name);
                            break;
                        }
                        Err(rustix::io::Errno::EXIST) => {}
                        Err(_) => return Err(FileUndoError::Unavailable),
                    }
                }
                let name = staged.ok_or(FileUndoError::Unavailable)?;
                let fd = rustix::fs::openat(&path.parent, name.as_str(), flags(), Mode::empty())
                    .map_err(unavailable)?;
                let mut cleanup = StageCleanup {
                    parent: path.parent.as_fd(),
                    file: fd.as_fd(),
                    name: &name,
                    published: false,
                    directory: true,
                };
                #[cfg(target_os = "macos")]
                {
                    calcifer_macos_acl::clear_acl(fd.as_fd())
                        .map_err(|_| FileUndoError::Unavailable)?;
                    if !calcifer_macos_acl::read_acl(fd.as_fd())
                        .map_err(|_| FileUndoError::Unavailable)?
                        .is_empty()
                    {
                        return Err(FileUndoError::Changed);
                    }
                }
                rustix::fs::fchmod(&fd, Mode::from_raw_mode(stat.st_mode & 0o777))
                    .map_err(|_| FileUndoError::Ambiguous)?;
                let held = rustix::fs::fstat(&fd).map_err(unavailable)?;
                let named =
                    rustix::fs::statat(&path.parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(unavailable)?;
                if !identity(&held, &named) {
                    return Err(FileUndoError::Changed);
                }
                sync(fd.as_fd())?;
                rustix::fs::renameat_with(
                    &path.parent,
                    name.as_str(),
                    &path.parent,
                    path.name.as_str(),
                    RenameFlags::NOREPLACE,
                )
                .map_err(|_| FileUndoError::Ambiguous)?;
                cleanup.published = true;
                let named =
                    rustix::fs::statat(&path.parent, path.name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(unavailable)?;
                if !identity(&held, &named) {
                    return Err(FileUndoError::Ambiguous);
                }
                drop(cleanup);
                Ok(Some(fd))
            }
            Snapshot::File {
                stat,
                bytes,
                digest: expected_digest,
                ..
            } => {
                let mut staged = None;
                for _ in 0..8 {
                    let name = temporary()?;
                    if name == path.name {
                        continue;
                    }
                    match rustix::fs::openat(
                        &path.parent,
                        name.as_str(),
                        OFlags::RDWR
                            | OFlags::CREATE
                            | OFlags::EXCL
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC
                            | OFlags::NONBLOCK,
                        Mode::from_raw_mode(0o600),
                    ) {
                        Ok(fd) => {
                            staged = Some((name, fd));
                            break;
                        }
                        Err(rustix::io::Errno::EXIST) => {}
                        Err(_) => return Err(FileUndoError::Unavailable),
                    }
                }
                let (name, fd) = staged.ok_or(FileUndoError::Unavailable)?;
                let mut cleanup = StageCleanup {
                    parent: path.parent.as_fd(),
                    file: fd.as_fd(),
                    name: &name,
                    published: false,
                    directory: false,
                };
                #[cfg(target_os = "macos")]
                {
                    calcifer_macos_acl::clear_acl(fd.as_fd())
                        .map_err(|_| FileUndoError::Unavailable)?;
                }
                let mut offset = 0;
                let mut interruptions = 0;
                let mut buffer = [0u8; 8192];
                let total = if source.is_some() {
                    usize::try_from(stat.st_size).map_err(|_| FileUndoError::ResourceLimit)?
                } else {
                    bytes.len()
                };
                if total > MAX_FILE_UNDO_OBSERVATION_BYTES {
                    return Err(FileUndoError::ResourceLimit);
                }
                while offset < total {
                    let end = total.min(offset + buffer.len());
                    let chunk = if let Some(source) = source {
                        let mut filled = 0;
                        while filled < end - offset {
                            match rustix::io::pread(
                                source,
                                &mut buffer[filled..end - offset],
                                (offset + filled) as u64,
                            ) {
                                Ok(0) => return Err(FileUndoError::Changed),
                                Ok(n) => filled += n,
                                Err(rustix::io::Errno::INTR) if interruptions < 15 => {
                                    interruptions += 1;
                                }
                                Err(_) => return Err(FileUndoError::Unavailable),
                            }
                        }
                        &buffer[..filled]
                    } else {
                        &bytes[offset..end]
                    };
                    let mut written = 0;
                    while written < chunk.len() {
                        match rustix::io::write(&fd, &chunk[written..]) {
                            Ok(0) => return Err(FileUndoError::Unavailable),
                            Ok(n) => written += n,
                            Err(rustix::io::Errno::INTR) if interruptions < 15 => {
                                interruptions += 1;
                            }
                            Err(_) => return Err(FileUndoError::Unavailable),
                        }
                    }
                    offset = end;
                }
                rustix::fs::fchmod(&fd, Mode::from_raw_mode(stat.st_mode & 0o777))
                    .map_err(unavailable)?;
                sync(fd.as_fd())?;
                let held = rustix::fs::fstat(&fd).map_err(unavailable)?;
                let (digest, _) = read(fd.as_fd(), false, &CancellationToken::new())?;
                if Some(digest) != *expected_digest
                    || !stable(&held, &rustix::fs::fstat(&fd).map_err(unavailable)?)
                {
                    return Err(FileUndoError::Changed);
                }
                #[cfg(target_os = "macos")]
                if !calcifer_macos_acl::read_acl(fd.as_fd())
                    .map_err(|_| FileUndoError::Unavailable)?
                    .is_empty()
                {
                    return Err(FileUndoError::Changed);
                }
                let named =
                    rustix::fs::statat(&path.parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(unavailable)?;
                if !identity(&held, &named) {
                    return Err(FileUndoError::Changed);
                }
                rustix::fs::renameat_with(
                    &path.parent,
                    name.as_str(),
                    &path.parent,
                    path.name.as_str(),
                    if replace {
                        RenameFlags::empty()
                    } else {
                        RenameFlags::NOREPLACE
                    },
                )
                .map_err(|_| FileUndoError::Ambiguous)?;
                cleanup.published = true;
                let published = capture(path, false, &CancellationToken::new())?;
                match published {
                    Snapshot::File {
                        stat: actual,
                        digest: actual_digest,
                        ..
                    } if identity(&held, &actual) && actual_digest == Some(digest) => {}
                    _ => return Err(FileUndoError::Ambiguous),
                }
                drop(cleanup);
                Ok(Some(fd))
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one ordered inverse-publication and history-rebase protocol"
    )]
    pub(super) fn undo(
        tracker: &FileUndoTracker,
        cancellation: &CancellationToken,
    ) -> Result<FileUndoOutcome, FileUndoError> {
        check(cancellation)?;
        let mut state = tracker.state.try_lock().map_err(|_| FileUndoError::Busy)?;
        tracker.check_clear_reservation()?;
        if state.ambiguous {
            return Err(FileUndoError::Ambiguous);
        }
        let Some(entry) = state.entries.back_mut() else {
            return Ok(FileUndoOutcome::Empty);
        };
        if entry.blocked {
            return Err(FileUndoError::Ambiguous);
        }
        if let Some(reason) = entry.before.iter().find_map(|snapshot| match snapshot {
            Snapshot::Unavailable(reason) => Some(*reason),
            _ => None,
        }) {
            return Err(FileUndoError::NotUndoable(reason));
        }
        validate_locations(entry)?;
        for (path, expected) in entry.paths.iter().zip(&entry.after) {
            verify(path, expected, cancellation)?;
        }
        check(cancellation)?;
        // No clean cancellation beyond this boundary. Preserve a non-replayable
        // barrier on any multi-syscall failure, with owned artifacts left intact.
        entry.blocked = true;
        let no_cancel = CancellationToken::new();
        let result = (|| {
            if entry.rename {
                let source = &entry.paths[0];
                let destination = &entry.paths[1];
                let quarantine_name = quarantine(destination, &entry.after[1])?;
                rustix::fs::renameat_with(
                    &destination.parent,
                    quarantine_name.as_str(),
                    &source.parent,
                    source.name.as_str(),
                    RenameFlags::NOREPLACE,
                )
                .map_err(|_| FileUndoError::Ambiguous)?;
                restore(destination, &entry.before[1])?;
            } else {
                let path = &entry.paths[0];
                if matches!(entry.after[0], Snapshot::Missing) {
                    restore(path, &entry.before[0])?;
                } else {
                    let name = quarantine(path, &entry.after[0])?;
                    restore(path, &entry.before[0])?;
                    let quarantined = Location {
                        root: rustix::io::fcntl_dupfd_cloexec(&path.root, 3)
                            .map_err(|_| FileUndoError::Ambiguous)?,
                        logical_path: path.logical_path.clone(),
                        path: path.path.clone(),
                        parent: rustix::io::dup(&path.parent)
                            .map_err(|_| FileUndoError::Ambiguous)?,
                        name,
                    };
                    let actual = capture(&quarantined, false, &no_cancel)?;
                    let same = match (&entry.after[0], &actual) {
                        (
                            Snapshot::File {
                                stat: a,
                                digest: da,
                                ..
                            },
                            Snapshot::File {
                                stat: b,
                                digest: db,
                                ..
                            },
                        ) => identity(a, b) && da == db,
                        _ => false,
                    };
                    if !same {
                        return Err(FileUndoError::Ambiguous);
                    }
                    rustix::fs::unlinkat(
                        &quarantined.parent,
                        quarantined.name.as_str(),
                        AtFlags::empty(),
                    )
                    .map_err(|_| FileUndoError::Ambiguous)?;
                }
            }
            for (index, (path, before)) in entry.paths.iter().zip(&entry.before).enumerate() {
                let actual = capture(path, false, &no_cancel)?;
                let matches = match (before, actual) {
                    (Snapshot::Missing, Snapshot::Missing) => true,
                    (
                        Snapshot::Directory { stat: old, .. },
                        Snapshot::Directory { stat: new, .. },
                    ) => old.st_mode & 0o777 == new.st_mode & 0o777,
                    (
                        Snapshot::File {
                            stat: old,
                            digest: old_digest,
                            ..
                        },
                        Snapshot::File {
                            stat: new,
                            digest: new_digest,
                            ..
                        },
                    ) => {
                        old.st_mode & 0o777 == new.st_mode & 0o777
                            && old_digest == &new_digest
                            && (!entry.rename
                                || index != 0
                                // A later tracked inverse may legitimately
                                // reconstruct this file. Move the exact admitted
                                // postimage, not its historical preimage inode.
                                || matches!(&entry.after[1], Snapshot::File { stat: admitted, .. }
                                    if identity(admitted, &new)
                                        && admitted.st_size == new.st_size
                                        && admitted.st_mtime == new.st_mtime
                                        && admitted.st_mtime_nsec == new.st_mtime_nsec))
                    }
                    _ => false,
                };
                if !matches {
                    return Err(FileUndoError::Ambiguous);
                }
            }
            Ok::<(), FileUndoError>(())
        })();
        let mut sync_failed = false;
        for path in &entry.paths {
            sync_failed |= sync(path.parent.as_fd()).is_err();
        }
        if result.is_err() || sync_failed {
            state.ambiguous = true;
            return Err(FileUndoError::Ambiguous);
        }
        let outcome = if !entry.rename && matches!(entry.before[0], Snapshot::Missing) {
            FileUndoOutcome::Removed(entry.paths[0].logical_path.clone())
        } else {
            FileUndoOutcome::Restored(entry.paths[0].logical_path.clone())
        };
        let removed = state.entries.pop_back().ok_or(FileUndoError::Ambiguous)?;
        state.bytes -= removed.bytes();
        // Undo itself changes inode identity. Rebase exact older predecessor
        // observations on the same paths, whose pinned objects and bytes match.
        for previous in &mut state.entries {
            if previous.after.len() != previous.paths.len() {
                continue;
            }
            for (index, path) in previous.paths.iter().enumerate() {
                if let Some(restored_index) = removed.paths.iter().position(|p| {
                    p.path == path.path
                        && match (
                            rustix::fs::fstat(&p.parent),
                            rustix::fs::fstat(&path.parent),
                        ) {
                            (Ok(a), Ok(b)) => identity(&a, &b),
                            _ => false,
                        }
                }) {
                    let expected = &removed.before[restored_index];
                    let compatible = match (&previous.after[index], expected) {
                        (Snapshot::Missing, Snapshot::Missing) => true,
                        (
                            Snapshot::File {
                                stat: sa,
                                digest: a,
                                ..
                            },
                            Snapshot::File {
                                stat: sb,
                                digest: b,
                                ..
                            },
                        ) => stable(sa, sb) && a == b,
                        _ => false,
                    };
                    if compatible {
                        match capture(path, false, &no_cancel) {
                            Ok(snapshot) => previous.after[index] = snapshot,
                            Err(_) => previous.blocked = true,
                        }
                    }
                }
            }
        }
        Ok(outcome)
    }

    pub(super) fn rename_replace(
        tracker: &FileUndoTracker,
        root: &std::fs::File,
        old: &str,
        new: &str,
        cancellation: &CancellationToken,
    ) -> Result<(), FileUndoError> {
        let mut transaction =
            tracker.begin(root.as_fd(), Operation::Rename(old, new), cancellation)?;
        transaction.revalidate(cancellation)?;
        let entry = transaction
            .entry
            .as_ref()
            .ok_or(FileUndoError::Unavailable)?;
        let source = &entry.paths[0];
        let destination = &entry.paths[1];
        let pinned = if let Some(fd) = entry.before[0].fd() {
            rustix::io::dup(fd).map_err(unavailable)?
        } else {
            rustix::fs::openat(
                &source.parent,
                source.name.as_str(),
                crate::rename_file::source_open_flags(),
                Mode::empty(),
            )
            .map_err(unavailable)?
        };
        let source_stat = rustix::fs::fstat(&pinned).map_err(unavailable)?;
        if !FileType::from_raw_mode(source_stat.st_mode).is_file() {
            return Err(FileUndoError::Rejected);
        }
        match rustix::fs::statat(
            &destination.parent,
            destination.name.as_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(stat)
                if !FileType::from_raw_mode(stat.st_mode).is_file()
                    || identity(&stat, &source_stat) =>
            {
                return Err(FileUndoError::Rejected);
            }
            Ok(_) | Err(rustix::io::Errno::NOENT) => {}
            Err(error) => return Err(unavailable(error)),
        }
        check(cancellation)?;
        let outcome = rustix::fs::renameat(
            &source.parent,
            source.name.as_str(),
            &destination.parent,
            destination.name.as_str(),
        );
        match outcome {
            Ok(()) => {}
            Err(rustix::io::Errno::INTR) => {
                let _ = sync(source.parent.as_fd());
                let _ = sync(destination.parent.as_fd());
                transaction.uncertain();
                return Err(FileUndoError::Ambiguous);
            }
            Err(_) => return Err(FileUndoError::Unavailable),
        }
        let parents = [
            rustix::io::dup(&source.parent),
            rustix::io::dup(&destination.parent),
        ];
        transaction.committed(Some(pinned.as_fd()));
        for parent in parents {
            sync(parent.map_err(|_| FileUndoError::Ambiguous)?.as_fd())?;
        }
        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one ordered tracked replacement and uncertainty protocol"
    )]
    pub(super) fn copy_replace(
        tracker: &FileUndoTracker,
        root: &std::fs::File,
        source: &str,
        destination: &str,
        cancellation: &CancellationToken,
    ) -> Result<(), FileUndoError> {
        if source == destination {
            return Err(FileUndoError::Rejected);
        }
        let source = locate(root.as_fd(), source)?;
        let source_snapshot = capture(&source, false, cancellation)?;
        if !matches!(source_snapshot, Snapshot::File { .. }) {
            return Err(FileUndoError::Rejected);
        }
        if usize::try_from(
            rustix::fs::fstat(source_snapshot.fd().ok_or(FileUndoError::Rejected)?)
                .map_err(unavailable)?
                .st_size,
        )
        .map_err(|_| FileUndoError::ResourceLimit)?
            > MAX_FILE_UNDO_OBSERVATION_BYTES
        {
            return Err(FileUndoError::ResourceLimit);
        }
        let mut transaction =
            tracker.begin(root.as_fd(), Operation::Replace(destination), cancellation)?;
        transaction.revalidate(cancellation)?;
        verify(&source, &source_snapshot, cancellation)?;
        let current_source = locate(root.as_fd(), &source.path)?;
        if !identity(
            &rustix::fs::fstat(&current_source.parent).map_err(unavailable)?,
            &rustix::fs::fstat(&source.parent).map_err(unavailable)?,
        ) {
            return Err(FileUndoError::Changed);
        }
        let entry = transaction
            .entry
            .as_ref()
            .ok_or(FileUndoError::Unavailable)?;
        if matches!(entry.before[0], Snapshot::Directory { .. }) {
            return Err(FileUndoError::Rejected);
        }
        let destination = &entry.paths[0];
        let unavailable_preimage = matches!(entry.before[0], Snapshot::Unavailable(_));
        if unavailable_preimage {
            match rustix::fs::statat(
                &destination.parent,
                destination.name.as_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(stat)
                    if !FileType::from_raw_mode(stat.st_mode).is_file()
                        || identity(
                            &stat,
                            &rustix::fs::fstat(
                                source_snapshot.fd().ok_or(FileUndoError::Rejected)?,
                            )
                            .map_err(unavailable)?,
                        ) =>
                {
                    return Err(FileUndoError::Rejected);
                }
                Ok(_) | Err(rustix::io::Errno::NOENT) => {}
                Err(error) => return Err(unavailable(error)),
            }
        }
        if let Some(destination_fd) = entry.before[0].fd()
            && identity(
                &rustix::fs::fstat(destination_fd).map_err(unavailable)?,
                &rustix::fs::fstat(source_snapshot.fd().ok_or(FileUndoError::Rejected)?)
                    .map_err(unavailable)?,
            )
        {
            return Err(FileUndoError::Rejected);
        }
        check(cancellation)?;
        let mut published_observation = None;
        let result = (|| {
            let quarantined =
                if matches!(entry.before[0], Snapshot::Missing) || unavailable_preimage {
                    None
                } else {
                    Some(quarantine(destination, &entry.before[0])?)
                };
            // Keep the exact staged object, never infer publication ownership
            // from a later lookup that could observe a replacement inode.
            published_observation = restore_with_source(
                destination,
                &source_snapshot,
                source_snapshot.fd(),
                unavailable_preimage,
            )?;
            if let Some(name) = quarantined {
                // Parent-name races retain the same disclosed cooperating-writer
                // boundary as the existing tool staging cleanup.
                let actual = rustix::fs::statat(
                    &destination.parent,
                    name.as_str(),
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(unavailable)?;
                let expected =
                    rustix::fs::fstat(entry.before[0].fd().ok_or(FileUndoError::Ambiguous)?)
                        .map_err(unavailable)?;
                if !identity(&actual, &expected) {
                    return Err(FileUndoError::Ambiguous);
                }
                rustix::fs::unlinkat(&destination.parent, name.as_str(), AtFlags::empty())
                    .map_err(unavailable)?;
            }
            Ok::<(), FileUndoError>(())
        })();
        // Even failed multi-step replacement owns its parent durability attempt.
        let sync_result = sync(destination.parent.as_fd());
        if let Some(published) = &published_observation {
            transaction.committed(Some(published.as_fd()));
        }
        if result.is_ok()
            && sync_result.is_ok()
            && verify(&source, &source_snapshot, &CancellationToken::new()).is_ok()
        {
            Ok(())
        } else {
            transaction.uncertain();
            Err(FileUndoError::Ambiguous)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::file_undo::tests::Temp;
        use std::fs;

        #[test]
        fn file_undo_unavailable_capture_records_only_after_actual_publication() {
            let temp = Temp::new();
            let root = fs::File::open(&temp.0).unwrap();
            let tracker = FileUndoTracker::new();
            fs::write(temp.0.join("a"), b"prior").unwrap();
            let mut transaction = tracker
                .begin(
                    root.as_fd(),
                    Operation::Replace("a"),
                    &CancellationToken::new(),
                )
                .unwrap();
            transaction.entry.as_mut().unwrap().before[0] =
                Snapshot::Unavailable(FileUndoUnavailableReason::SnapshotUnavailable);
            transaction.revalidate(&CancellationToken::new()).unwrap();
            fs::write(temp.0.join("replacement"), b"forward").unwrap();
            let published = fs::File::open(temp.0.join("replacement")).unwrap();
            fs::rename(temp.0.join("replacement"), temp.0.join("a")).unwrap();
            transaction.committed(Some(published.as_fd()));
            drop(transaction);
            assert_eq!(
                tracker.latest_unavailable_reason().unwrap(),
                Some(FileUndoUnavailableReason::SnapshotUnavailable)
            );
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Err(FileUndoError::NotUndoable(
                    FileUndoUnavailableReason::SnapshotUnavailable
                ))
            );
            assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"forward");
            assert!(!tracker.state.lock().unwrap().ambiguous);
        }

        #[test]
        fn file_undo_replacement_copy_pins_published_stage_through_registration() {
            let temp = Temp::new();
            fs::write(temp.0.join("source"), b"same bytes").unwrap();
            let root = fs::File::open(&temp.0).unwrap();
            let tracker = FileUndoTracker::new();
            let cancellation = CancellationToken::new();
            let source = locate(root.as_fd(), "source").unwrap();
            let snapshot = capture(&source, true, &cancellation).unwrap();
            let mut transaction = tracker
                .begin(
                    root.as_fd(),
                    Operation::Replace("destination"),
                    &cancellation,
                )
                .unwrap();
            let destination = &transaction.entry.as_ref().unwrap().paths[0];
            let published = restore(destination, &snapshot).unwrap().unwrap();
            fs::write(temp.0.join("foreign"), b"same bytes").unwrap();
            fs::rename(temp.0.join("foreign"), temp.0.join("destination")).unwrap();
            transaction.committed(Some(published.as_fd()));
            drop(transaction);
            assert_eq!(tracker.state.lock().unwrap().entries.len(), 1);
            assert_eq!(
                tracker.undo_last(&cancellation),
                Err(FileUndoError::Ambiguous)
            );
            assert_eq!(fs::read(temp.0.join("destination")).unwrap(), b"same bytes");
        }

        #[test]
        fn file_undo_quarantine_preserves_changed_leaf_and_rolls_back_without_clobber() {
            let temp = Temp::new();
            fs::write(temp.0.join("a"), b"expected").unwrap();
            let root = fs::File::open(&temp.0).unwrap();
            let location = locate(root.as_fd(), "a").unwrap();
            let expected = capture(&location, false, &CancellationToken::new()).unwrap();
            fs::write(temp.0.join("replacement"), b"sentinel").unwrap();
            fs::rename(temp.0.join("replacement"), temp.0.join("a")).unwrap();
            assert_eq!(
                quarantine(&location, &expected),
                Err(FileUndoError::Ambiguous)
            );
            assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"sentinel");
            assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
        }

        #[test]
        fn file_undo_recreated_name_preserved_and_failed_stage_cleaned() {
            let temp = Temp::new();
            fs::write(temp.0.join("a"), b"preimage").unwrap();
            let root = fs::File::open(&temp.0).unwrap();
            let tracker = FileUndoTracker::new();
            let mut transaction = tracker
                .begin(
                    root.as_fd(),
                    Operation::Replace("a"),
                    &CancellationToken::new(),
                )
                .unwrap();
            let location = locate(root.as_fd(), "a").unwrap();
            let expected = capture(&location, true, &CancellationToken::new()).unwrap();
            let name = quarantine(&location, &expected).unwrap();
            fs::write(temp.0.join("a"), b"new-owner").unwrap();
            assert!(restore(&location, &expected).is_err());
            transaction.uncertain();
            drop(transaction);
            tracker.clear().unwrap();
            drop(tracker);
            assert_eq!(fs::read(temp.0.join("a")).unwrap(), b"new-owner");
            assert_eq!(fs::read(temp.0.join(name)).unwrap(), b"preimage");
            assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 2);
        }

        #[test]
        fn file_undo_clear_reservation_preserves_barriers_and_rejects_active_transaction() {
            let temp = Temp::new();
            let tracker = std::sync::Arc::new(FileUndoTracker::new());
            let root = fs::File::open(&temp.0).unwrap();
            let mut transaction = tracker
                .begin(
                    root.as_fd(),
                    Operation::Replace("file"),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert!(matches!(tracker.reserve_clear(), Err(FileUndoError::Busy)));
            transaction.uncertain();
            drop(transaction);
            let reservation = tracker.reserve_clear().unwrap();
            assert!(tracker.state.lock().unwrap().ambiguous);
            drop(reservation);
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Err(FileUndoError::Ambiguous)
            );
            let reservation = tracker.reserve_clear().unwrap();
            assert!(matches!(
                tracker.begin(
                    root.as_fd(),
                    Operation::Replace("file"),
                    &CancellationToken::new()
                ),
                Err(FileUndoError::Busy)
            ));
            reservation.commit();
            assert!(!tracker.state.lock().unwrap().ambiguous);
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Ok(FileUndoOutcome::Empty)
            );
        }

        #[test]
        fn file_undo_uncertain_publication_blocks_replay_without_recording_commit() {
            let temp = Temp::new();
            let tracker = FileUndoTracker::new();
            let root = fs::File::open(&temp.0).unwrap();
            let mut transaction = tracker
                .begin(
                    root.as_fd(),
                    Operation::Replace("a"),
                    &CancellationToken::new(),
                )
                .unwrap();
            transaction.uncertain();
            drop(transaction);
            assert_eq!(tracker.state.lock().unwrap().entries.len(), 0);
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Err(FileUndoError::Ambiguous)
            );
            assert!(
                tracker
                    .begin(
                        root.as_fd(),
                        Operation::Replace("a"),
                        &CancellationToken::new()
                    )
                    .is_err()
            );
            tracker.clear().unwrap();
            assert_eq!(
                tracker.undo_last(&CancellationToken::new()),
                Ok(FileUndoOutcome::Empty)
            );
        }

        #[test]
        fn file_undo_exact_preimage_limit_and_aggregate_retention_are_bounded() {
            let temp = Temp::new();
            let tracker = FileUndoTracker::new();
            let root = fs::File::open(&temp.0).unwrap();
            let first = fs::File::create(temp.0.join("a")).unwrap();
            first.set_len(MAX_FILE_UNDO_PREIMAGE_BYTES as u64).unwrap();
            let location = locate(root.as_fd(), "a").unwrap();
            let snapshot = capture(&location, true, &CancellationToken::new()).unwrap();
            assert_eq!(snapshot.bytes(), MAX_FILE_UNDO_PREIMAGE_BYTES);
            drop(snapshot);
            for name in ["a", "b"] {
                let file = fs::File::create(temp.0.join(name)).unwrap();
                file.set_len((MAX_FILE_UNDO_PREIMAGE_BYTES / 2 + 1) as u64)
                    .unwrap();
                let mut transaction = tracker
                    .begin(
                        root.as_fd(),
                        Operation::Delete(name),
                        &CancellationToken::new(),
                    )
                    .unwrap();
                transaction.revalidate(&CancellationToken::new()).unwrap();
                fs::remove_file(temp.0.join(name)).unwrap();
                transaction.committed(None);
            }
            let state = tracker.state.lock().unwrap();
            assert_eq!(state.entries.len(), 2);
            assert_eq!(state.bytes, MAX_FILE_UNDO_PREIMAGE_BYTES + 2);
            assert!(state.bytes <= crate::file_undo::MAX_FILE_UNDO_RETAINED_BYTES);
            drop(state);
            tracker.undo_last(&CancellationToken::new()).unwrap();
            tracker.undo_last(&CancellationToken::new()).unwrap();
            for name in ["a", "b"] {
                assert_eq!(
                    fs::metadata(temp.0.join(name)).unwrap().len(),
                    (MAX_FILE_UNDO_PREIMAGE_BYTES / 2 + 1) as u64
                );
            }
        }
    }
}
