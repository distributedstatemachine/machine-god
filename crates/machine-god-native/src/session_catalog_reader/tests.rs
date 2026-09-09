use super::*;
use crate::{
    NATIVE_SESSION_METADATA_KEY, NativeSessionCatalog, NativeSessionMetadata, NativeSessionOrigin,
};
use futures_executor::block_on;
use machine_god_core::{SessionId, SessionIncarnationId, SessionRecord, SessionStore};
use std::{
    fs,
    path::Path,
    sync::{Condvar, Mutex},
    task::{Context, Poll},
    time::Duration,
};

#[derive(Default)]
pub(super) struct Probe {
    state: Mutex<(bool, bool)>,
    wake: Condvar,
    exit: Option<Arc<Probe>>,
}
impl Probe {
    pub(super) fn enter(&self) {
        if let Some(exit) = &self.exit {
            thread_local! { static EXIT: std::cell::RefCell<Option<Exit>> = const { std::cell::RefCell::new(None) }; }
            EXIT.with(|slot| *slot.borrow_mut() = Some(Exit(exit.clone())));
        }
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
                Duration::from_secs(10),
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
struct Exit(Arc<Probe>);
impl Drop for Exit {
    fn drop(&mut self) {
        self.0.enter();
    }
}
struct Fixture {
    root: PathBuf,
    store: Arc<FileSessionStore>,
    scope: NativeOwnedWorkerScope,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "machine-god-owned-catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let store = Arc::new(FileSessionStore::open(&root).unwrap());
        Self {
            root,
            store,
            scope: NativeOwnedWorkerScope::new(),
        }
    }
    fn reader(&self) -> NativeSessionCatalogReader {
        NativeSessionCatalogReader::new(
            self.store.clone(),
            PathBuf::from("/workspace"),
            self.scope.clone(),
        )
    }
    fn save(&self, id: &str, time: Option<i64>, workspace: &str) {
        let mut record = SessionRecord::empty(
            SessionId::new(id).unwrap(),
            SessionIncarnationId::new(format!("life-{id}")).unwrap(),
        );
        if let Some(time) = time {
            record.metadata.insert(
                NATIVE_SESSION_METADATA_KEY.into(),
                NativeSessionMetadata::new(Path::new(workspace), time, NativeSessionOrigin::Cli)
                    .unwrap()
                    .to_value(),
            );
        }
        block_on(self.store.save(record, None)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.scope.close();
        self.scope.completion().wait_on_worker().unwrap();
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn list(
    reader: &NativeSessionCatalogReader,
    scope: NativeSessionCatalogScope,
    limit: usize,
) -> Result<NativeSessionCatalogPage, NativeSessionCatalogReadError> {
    block_on(reader.list(scope, limit, None, CancellationToken::new()))
}

#[test]
fn unpolled_and_precancelled_reads_do_not_enter_workers_or_touch_directory() {
    let fixture = Fixture::new();
    let mut reader = fixture.reader();
    let probe = Arc::new(Probe::default());
    Arc::get_mut(&mut reader.inner).unwrap().probe = Some(probe.clone());
    drop(reader.list(
        NativeSessionCatalogScope::All,
        10,
        None,
        CancellationToken::new(),
    ));
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        block_on(reader.list(NativeSessionCatalogScope::All, 10, None, cancel)).unwrap_err(),
        NativeSessionCatalogReadError::Cancelled
    );
    assert!(!probe.state.lock().unwrap().0);
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}

#[test]
fn invalid_limits_fail_before_admission_and_closed_scope_returns_unavailable() {
    let fixture = Fixture::new();
    let reader = fixture.reader();
    fixture.scope.close();
    for limit in [0, 101, usize::MAX] {
        assert!(
            matches!(list(&reader, NativeSessionCatalogScope::All, limit), Err(NativeSessionCatalogReadError::Catalog(error)) if error.kind() == crate::NativeSessionCatalogErrorKind::InvalidQuery)
        );
    }
    assert_eq!(
        list(&reader, NativeSessionCatalogScope::All, 1).unwrap_err(),
        NativeSessionCatalogReadError::Unavailable
    );
    assert!(!reader.inner.active.load(Ordering::Acquire));
}

#[test]
fn owned_scan_preserves_order_scope_unknowns_cursor_and_invalid_counts() {
    let fixture = Fixture::new();
    fixture.save("new", Some(20), "/workspace");
    fixture.save("old", Some(10), "/workspace");
    fixture.save("other", Some(30), "/other");
    fixture.save("unknown", None, "/workspace");
    fixture.save("broken", Some(40), "/workspace");
    let broken = fs::read_dir(&fixture.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && fs::read_to_string(path).unwrap().contains("life-broken")
        })
        .unwrap();
    fs::write(broken, b"{invalid").unwrap();
    let reader = fixture.reader();
    let page = list(&reader, NativeSessionCatalogScope::All, 2).unwrap();
    assert_eq!(
        page.entries()
            .iter()
            .map(|e| e.id().as_str())
            .collect::<Vec<_>>(),
        ["other", "new"]
    );
    assert_eq!(page.skipped_invalid(), 1);
    assert_eq!(page.unknown_activity_count(), 1);
    assert_eq!(page.matched_count(), 4);
    assert!(page.scan_complete());
    let cursor = page.next_cursor().unwrap();
    let second = block_on(reader.list(
        NativeSessionCatalogScope::All,
        2,
        Some(cursor),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(
        second
            .entries()
            .iter()
            .map(|e| e.id().as_str())
            .collect::<Vec<_>>(),
        ["old", "unknown"]
    );
    assert!(second.next_cursor().is_none());
    let scoped = list(&reader, NativeSessionCatalogScope::CurrentWorkspace, 10).unwrap();
    assert_eq!(
        scoped
            .entries()
            .iter()
            .map(|e| e.id().as_str())
            .collect::<Vec<_>>(),
        ["new", "old"]
    );
    let ordinary = block_on(
        NativeSessionCatalog::new(fixture.store.clone()).list(
            NativeSessionCatalogQuery::new(2)
                .unwrap()
                .with_invalid_records(NativeSessionCatalogInvalidRecords::SkipAndReport),
        ),
    )
    .unwrap();
    assert_eq!(format!("{ordinary:?}"), format!("{page:?}"));
}

#[test]
fn locked_candidate_returns_busy_not_skipped_or_partial_and_unlock_restores_listing() {
    let fixture = Fixture::new();
    fixture.save("one", Some(1), "/workspace");
    let lock_path = fs::read_dir(&fixture.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "lock"))
        .unwrap();
    let lock = fs::File::open(lock_path).unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let reader = fixture.reader();
    assert_eq!(
        list(&reader, NativeSessionCatalogScope::All, 10).unwrap_err(),
        NativeSessionCatalogReadError::Busy
    );
    drop(lock);
    assert_eq!(
        list(&reader, NativeSessionCatalogScope::All, 10)
            .unwrap()
            .entries()
            .len(),
        1
    );
}

#[test]
fn abandoned_response_cancels_only_private_job_and_retains_admission_until_scan_returns() {
    let fixture = Fixture::new();
    let mut reader = fixture.reader();
    let probe = Arc::new(Probe::default());
    Arc::get_mut(&mut reader.inner).unwrap().probe = Some(probe.clone());
    let clone = reader.clone();
    let cancel = CancellationToken::new();
    let mut response = reader.list(NativeSessionCatalogScope::All, 10, None, cancel.clone());
    assert!(matches!(
        response
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop())),
        Poll::Pending
    ));
    probe.wait();
    drop(response);
    assert!(!cancel.is_cancelled());
    assert_eq!(
        list(&clone, NativeSessionCatalogScope::All, 10).unwrap_err(),
        NativeSessionCatalogReadError::Busy
    );
    fixture.scope.close();
    assert!(!fixture.scope.completion().is_complete());
    probe.release();
    fixture.scope.completion().wait_on_worker().unwrap();
    assert!(!reader.inner.active.load(Ordering::Acquire));
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}

#[test]
fn cancellation_between_read_chunks_releases_the_record_lock_without_a_partial_page() {
    let fixture = Fixture::new();
    let mut record = SessionRecord::empty(
        SessionId::new("large").unwrap(),
        SessionIncarnationId::new("large-life").unwrap(),
    );
    record.messages.push(machine_god_core::Message::text(
        machine_god_core::Role::User,
        "x".repeat(32_000),
    ));
    block_on(fixture.store.save(record, None)).unwrap();
    let cancel = CancellationToken::new();
    let hook_cancel = cancel.clone();
    let read_bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = read_bytes.clone();
    let control = crate::session_store::FileSessionScanControl {
        cancel,
        abandoned: CancellationToken::new(),
        after_read: Some(Arc::new(move |bytes| {
            observed.store(bytes, Ordering::Release);
            hook_cancel.cancel();
        })),
    };
    let result = crate::session_catalog::list_from_store_with_control(
        &fixture.store,
        &NativeSessionCatalogQuery::default()
            .with_invalid_records(NativeSessionCatalogInvalidRecords::SkipAndReport),
        Some(&control),
    );
    assert_eq!(
        result.unwrap_err(),
        NativeSessionCatalogReadError::Cancelled
    );
    assert_eq!(read_bytes.load(Ordering::Acquire), 8192);
    assert_eq!(
        list(&fixture.reader(), NativeSessionCatalogScope::All, 10)
            .unwrap()
            .entries()
            .len(),
        1
    );
}

#[test]
fn response_ready_is_not_actual_worker_join_and_reader_drop_does_not_close_scope() {
    let fixture = Fixture::new();
    let mut reader = fixture.reader();
    let exit = Arc::new(Probe::default());
    let entry = Arc::new(Probe {
        exit: Some(exit.clone()),
        ..Probe::default()
    });
    entry.release();
    Arc::get_mut(&mut reader.inner).unwrap().probe = Some(entry);
    assert!(
        list(&reader, NativeSessionCatalogScope::All, 10)
            .unwrap()
            .entries()
            .is_empty()
    );
    exit.wait();
    drop(reader);
    assert!(block_on(fixture.scope.run(|| 42)).is_ok());
    fixture.scope.close();
    assert!(!fixture.scope.completion().is_complete());
    exit.release();
    fixture.scope.completion().wait_on_worker().unwrap();
}
