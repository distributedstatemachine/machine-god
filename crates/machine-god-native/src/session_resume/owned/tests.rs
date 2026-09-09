use super::*;
use crate::{NativeModelPreferences, NativeSessionCatalog, NativeSessionMetadata};
use futures_executor::block_on;
use machine_god_core::Engine;
use machine_god_core::SessionIncarnationId;
use machine_god_testkit::{ScriptedModelProvider, ScriptedPermissionHandler};
use std::{fs, path::PathBuf, sync::atomic::AtomicU64};

struct Fixture {
    root: PathBuf,
    lifecycle: NativeSessionLifecycle,
    access: Arc<Access>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "mg-resume-owned-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let store = Arc::new(FileSessionStore::open(&root).unwrap());
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("fixture", []))
            .shared_session_store(store.clone())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let lifecycle = NativeSessionLifecycle::new(engine, store.clone()).unwrap();
        let access = Arc::new(Access {
            erased: store.clone(),
            store,
            workers: NativeOwnedWorkerScope::new(),
            control: Arc::new(FileSessionScanControl {
                cancel: CancellationToken::new(),
                abandoned: CancellationToken::new(),
                after_read: None,
            }),
            failure: Arc::new(AtomicU8::new(0)),
            before_io: None,
        });
        Self {
            root,
            lifecycle,
            access,
        }
    }
    fn seed(&self) -> NativeObservedSession {
        let id = SessionId::new("chosen").unwrap();
        let mut record = SessionRecord::empty(
            id.clone(),
            SessionIncarnationId::new("chosen-life").unwrap(),
        );
        record.metadata.insert(
            crate::NATIVE_SESSION_METADATA_KEY.into(),
            NativeSessionMetadata::new(Path::new("/original"), 1, crate::NativeSessionOrigin::Cli)
                .unwrap()
                .to_value(),
        );
        block_on(self.access.store.save(record, None)).unwrap();
        let entry = block_on(NativeSessionCatalog::new(self.access.store.clone()).exact(id))
            .unwrap()
            .unwrap();
        NativeObservedSession::from_entry(&entry)
    }
    fn resume(
        &self,
        observed: NativeObservedSession,
        cancel: CancellationToken,
    ) -> Result<NativeConversation, Error> {
        block_on(resume(
            &self.lifecycle,
            observed,
            Path::new("/workspace"),
            10,
            self.access.workers.clone(),
            cancel,
        ))
    }
    fn lock(&self) -> fs::File {
        let path = fs::read_dir(&self.root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "lock"))
            .unwrap();
        let file = fs::File::open(path).unwrap();
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive).unwrap();
        file
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.access.workers.close();
        self.access.workers.completion().wait_on_worker().unwrap();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn observed_owned_rebind_and_adopt_use_same_registry_and_store() {
    let f = Fixture::new();
    let observed = f.seed();
    let old = block_on(f.lifecycle.engine().load_session(observed.id.clone()))
        .unwrap()
        .unwrap();
    let result = f
        .resume(observed.clone(), CancellationToken::new())
        .unwrap();
    assert_eq!(result.record(), old.record());
    assert_eq!(
        result.record().revision,
        SessionRevision(observed.revision.0 + 1)
    );
    assert_eq!(
        result.record(),
        block_on(f.access.store.load(observed.id)).unwrap().unwrap()
    );
}

#[test]
fn legacy_candidate_preference_flush_uses_owned_cas_under_late_lock_contention() {
    let f = Fixture::new();
    let observed = f.seed();
    let conversation = f
        .resume(observed.clone(), CancellationToken::new())
        .unwrap();
    let runtime = crate::NativeConversationRuntime::new(
        conversation,
        NativeModelPreferences::new(
            "fixture/model",
            crate::NativeReasoningEffort::default(),
            false,
        )
        .unwrap(),
        None,
    )
    .unwrap();
    assert!(runtime.status().model_preferences_pending);
    let lock = f.lock();
    // Keep a shared open file description alive across the explicit unlock,
    // as a concurrent spawn can do until it reaches exec.
    let retained_lock = lock.try_clone().unwrap();
    assert!(
        block_on(runtime.flush_model_preferences_with_access(20, Some(f.access.clone()))).is_err()
    );
    assert!(runtime.status().model_preferences_pending);
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::Unlock).unwrap();
    drop(lock);
    assert!(
        block_on(runtime.flush_model_preferences_with_access(20, Some(f.access.clone()))).is_ok()
    );
    assert!(!runtime.status().model_preferences_pending);
    let record = block_on(f.access.store.load(observed.id)).unwrap().unwrap();
    assert_eq!(
        NativeModelPreferences::from_metadata(&record.metadata)
            .unwrap()
            .unwrap()
            .model(),
        "fixture/model"
    );
    drop(retained_lock);
}

#[test]
fn advisory_lock_contention_rejects_load_and_cas_as_busy_without_mutation() {
    let f = Fixture::new();
    let observed = f.seed();
    let before = block_on(f.access.store.load(observed.id.clone()))
        .unwrap()
        .unwrap();
    let lock = f.lock();
    let retained_lock = lock.try_clone().unwrap();
    assert_eq!(
        f.resume(observed.clone(), CancellationToken::new())
            .unwrap_err()
            .kind(),
        Kind::Busy
    );
    let error = block_on(f.access.save(before.clone(), Some(before.revision))).unwrap_err();
    assert_eq!(
        f.access
            .map_failure(map_engine(EngineError::Store(error)))
            .kind(),
        Kind::Busy
    );
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::Unlock).unwrap();
    drop(lock);
    assert_eq!(
        block_on(f.access.store.load(observed.id)).unwrap().unwrap(),
        before
    );
    drop(retained_lock);
}

#[test]
fn unpolled_and_precancelled_resume_are_effect_free_and_stale_target_conflicts() {
    let f = Fixture::new();
    let observed = f.seed();
    let before = block_on(f.access.store.load(observed.id.clone()))
        .unwrap()
        .unwrap();
    drop(resume(
        &f.lifecycle,
        observed.clone(),
        Path::new("/workspace"),
        10,
        f.access.workers.clone(),
        CancellationToken::new(),
    ));
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        f.resume(observed.clone(), cancel).unwrap_err().kind(),
        Kind::Cancelled
    );
    assert_eq!(
        block_on(f.access.store.load(observed.id.clone()))
            .unwrap()
            .unwrap(),
        before
    );
    block_on(f.access.store.save(before.clone(), Some(before.revision))).unwrap();
    assert_eq!(
        f.resume(observed, CancellationToken::new())
            .unwrap_err()
            .kind(),
        Kind::Conflict
    );
}

#[test]
fn wrong_store_adapter_is_rejected_before_adapter_load_save_or_reconciliation() {
    let f = Fixture::new();
    let other = Fixture::new();
    let observed = f.seed();
    other.access.workers.close();
    let requester = f.lifecycle.engine().requester();
    let rejected = block_on(requester.load_session_at_revision_with_access(
        observed.id.clone(),
        observed.incarnation_id,
        observed.revision,
        other.access.clone(),
    ));
    assert!(
        matches!(rejected, Err(EngineError::Protocol(ref message)) if message == "session store access identity mismatch")
    );
    let session = block_on(f.lifecycle.engine().load_session(observed.id))
        .unwrap()
        .unwrap();
    assert!(matches!(
        block_on(session.update_metadata_with_access(
            observed.revision,
            std::collections::BTreeMap::default(),
            other.access.clone()
        )),
        Err(EngineError::Protocol(_))
    ));
    assert!(matches!(
        block_on(
            session.check_metadata_revision_with_access(observed.revision, other.access.clone())
        ),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(fs::read_dir(&other.root).unwrap().count(), 0);
}

#[derive(Default)]
struct Gate {
    state: std::sync::Mutex<(bool, bool)>,
    wake: std::sync::Condvar,
}
impl Gate {
    fn enter(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.wake.notify_all();
        while !state.1 {
            state = self.wake.wait(state).unwrap();
        }
    }
    fn wait(&self) {
        let (state, timeout) = self
            .wake
            .wait_timeout_while(
                self.state.lock().unwrap(),
                std::time::Duration::from_secs(10),
                |state| !state.0,
            )
            .unwrap();
        assert!(!timeout.timed_out() && state.0);
    }
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.wake.notify_all();
    }
}
struct Exit(Arc<Gate>);
impl Drop for Exit {
    fn drop(&mut self) {
        self.0.enter();
    }
}

#[test]
fn dropped_selection_cancels_private_io_and_actual_tls_completion_remains_owned() {
    let mut f = Fixture::new();
    let observed = f.seed();
    let before = block_on(f.access.store.load(observed.id.clone()))
        .unwrap()
        .unwrap();
    let enter = Arc::new(Gate::default());
    let exit = Arc::new(Gate::default());
    let entry_hook = enter.clone();
    let exit_hook = exit.clone();
    Arc::get_mut(&mut f.access).unwrap().before_io = Some(Arc::new(move || {
        thread_local! { static EXIT: std::cell::RefCell<Option<Exit>> = const { std::cell::RefCell::new(None) }; }
        EXIT.with(|slot| *slot.borrow_mut() = Some(Exit(exit_hook.clone())));
        entry_hook.enter();
    }));
    let caller = f.access.control.cancel.clone();
    let mut response = Box::pin(run(
        &f.lifecycle,
        observed,
        Path::new("/workspace"),
        10,
        f.access.store.clone(),
        f.access.clone(),
    ));
    assert!(
        std::future::Future::poll(
            response.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop())
        )
        .is_pending()
    );
    enter.wait();
    drop(response);
    assert!(!caller.is_cancelled());
    assert!(f.access.control.abandoned.is_cancelled());
    f.access.workers.close();
    assert!(!f.access.workers.completion().is_complete());
    enter.release();
    exit.wait();
    assert!(!f.access.workers.completion().is_complete());
    exit.release();
    f.access.workers.completion().wait_on_worker().unwrap();
    assert_eq!(
        block_on(f.access.store.load(before.id.clone()))
            .unwrap()
            .unwrap(),
        before
    );
}

#[test]
fn engine_uncertainty_reconciliation_uses_controlled_adapter_not_blocking_original_store() {
    let f = Fixture::new();
    let observed = f.seed();
    let session = block_on(f.lifecycle.engine().load_session(observed.id.clone()))
        .unwrap()
        .unwrap();
    let lock = f.lock();
    let retained_lock = lock.try_clone().unwrap();
    // Failure conservatively arms core's reconciliation debt.
    assert!(
        block_on(session.update_metadata_with_access(
            observed.revision,
            session.record().metadata,
            f.access.clone()
        ))
        .is_err()
    );
    // The recheck must also return promptly while that same OS lock is held.
    assert!(
        block_on(session.check_metadata_revision_with_access(observed.revision, f.access.clone()))
            .is_err()
    );
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::Unlock).unwrap();
    drop(lock);
    assert_eq!(
        block_on(session.check_metadata_revision_with_access(observed.revision, f.access.clone()))
            .unwrap(),
        observed.revision
    );
    drop(retained_lock);
}

#[test]
#[ignore = "private subprocess fixture"]
fn advisory_lock_child() {
    use std::io::{Read, Write};
    let path = std::env::var_os("MACHINE_GOD_RESUME_TEST_LOCK").unwrap();
    let lock = fs::File::open(path).unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    println!("LOCK_READY");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_exact(&mut [0]).unwrap();
}

#[test]
fn another_process_holding_the_exact_lock_returns_busy_and_retry_succeeds() {
    use std::io::{BufRead, Write};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let f = Fixture::new();
    let observed = f.seed();
    let path = fs::read_dir(&f.root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "lock"))
        .unwrap();
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session_resume::owned::tests::advisory_lock_child",
                "--ignored",
                "--nocapture",
            ])
            .env("MACHINE_GOD_RESUME_TEST_LOCK", path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let mut stdout = std::io::BufReader::new(stdout);
    assert!(
        (&mut stdout)
            .lines()
            .take(10)
            .any(|line| line.unwrap() == "LOCK_READY")
    );
    assert_eq!(
        f.resume(observed.clone(), CancellationToken::new())
            .unwrap_err()
            .kind(),
        Kind::Busy
    );
    child.0.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.0.wait().unwrap().success());
    assert!(f.resume(observed, CancellationToken::new()).is_ok());
}
