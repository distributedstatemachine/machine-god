#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use futures_util::StreamExt;
use machine_god_core::{
    ContentBlock, Engine, EngineEvent, Message, ModelEvent, Role, Session, SessionId,
    SessionIncarnationId, SessionRecord, SessionRevision, SessionStore, SessionStoreErrorKind,
    StopReason, TurnEvent,
};
use machine_god_native::{
    FILE_SESSION_SCHEMA_VERSION, FileSessionStore, MAX_SESSION_INCARNATION_ATTEMPTS,
    NativeSessionLifecycle, NativeSessionLifecycleBuildErrorKind, NativeSessionLifecycleError,
    NativeSessionLifecycleErrorKind, SessionIncarnationSource, SessionIncarnationSourceError,
};
use machine_god_testkit::{ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler};
use serde_json::json;

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-session-lifecycle-{label}-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
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
            Err(error) => panic!("failed to remove a temporary directory: {error}"),
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("native session operation unexpectedly remained pending"),
    }
}

enum IncarnationStep {
    Id(SessionIncarnationId),
    Error,
}

type SourceHook = Box<dyn FnOnce() + Send>;

struct ScriptedIncarnationSource {
    steps: Mutex<VecDeque<IncarnationStep>>,
    calls: AtomicUsize,
    first_call_hook: Mutex<Option<SourceHook>>,
}

impl ScriptedIncarnationSource {
    fn ids(values: impl IntoIterator<Item = &'static str>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(
                values
                    .into_iter()
                    .map(|value| IncarnationStep::Id(SessionIncarnationId::new(value).unwrap()))
                    .collect(),
            ),
            calls: AtomicUsize::new(0),
            first_call_hook: Mutex::new(None),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(VecDeque::from([IncarnationStep::Error])),
            calls: AtomicUsize::new(0),
            first_call_hook: Mutex::new(None),
        })
    }

    fn with_hook(
        values: impl IntoIterator<Item = &'static str>,
        hook: impl FnOnce() + Send + 'static,
    ) -> Arc<Self> {
        let source = Self::ids(values);
        *source.first_call_hook.lock().unwrap() = Some(Box::new(hook));
        source
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for ScriptedIncarnationSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScriptedIncarnationSource")
            .field("calls", &self.calls())
            .finish_non_exhaustive()
    }
}

impl SessionIncarnationSource for ScriptedIncarnationSource {
    fn next_incarnation_id(&self) -> Result<SessionIncarnationId, SessionIncarnationSourceError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(hook) = self.first_call_hook.lock().unwrap().take() {
            hook();
        }
        match self.steps.lock().unwrap().pop_front() {
            Some(IncarnationStep::Id(id)) => Ok(id),
            Some(IncarnationStep::Error) | None => Err(SessionIncarnationSourceError::new()),
        }
    }
}

fn completed_step(text: &str) -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: text.to_owned(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}

fn engine(
    store: &Arc<FileSessionStore>,
    steps: impl IntoIterator<Item = ModelProviderStep>,
) -> (Engine, ScriptedModelProvider) {
    let provider = ScriptedModelProvider::new("session-lifecycle-test", steps);
    let shared_store: Arc<dyn SessionStore> = store.clone();
    let engine = Engine::builder()
        .provider(provider.clone())
        .shared_session_store(shared_store)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    (engine, provider)
}

fn lifecycle(
    store: &Arc<FileSessionStore>,
    source: Arc<ScriptedIncarnationSource>,
    steps: impl IntoIterator<Item = ModelProviderStep>,
) -> (NativeSessionLifecycle, ScriptedModelProvider) {
    let (engine, provider) = engine(store, steps);
    let source: Arc<dyn SessionIncarnationSource> = source;
    (
        NativeSessionLifecycle::shared_incarnation_source(engine, store.clone(), source).unwrap(),
        provider,
    )
}

fn id(value: &str) -> SessionId {
    SessionId::new(value).unwrap()
}

fn incarnation(value: &str) -> SessionIncarnationId {
    SessionIncarnationId::new(value).unwrap()
}

fn save_new(
    store: &FileSessionStore,
    session_id: SessionId,
    incarnation_id: SessionIncarnationId,
) -> SessionRecord {
    let mut record = SessionRecord::empty(session_id, incarnation_id);
    record.revision = ready(store.save(record.clone(), None)).unwrap();
    record
}

fn load(store: &FileSessionStore, session_id: SessionId) -> Option<SessionRecord> {
    ready(store.load(session_id)).unwrap()
}

fn data_path(root: &Path) -> PathBuf {
    let matches = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected exactly one session data file");
    matches.into_iter().next().unwrap()
}

fn directory_entries(root: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn persisted_bytes(record: &SessionRecord) -> Vec<u8> {
    format!(
        "{{\"schema_version\":{FILE_SESSION_SCHEMA_VERSION},\"record\":{}}}",
        serde_json::to_string(record).unwrap()
    )
    .into_bytes()
}

fn collect_turn(session: &Session, prompt: &str) -> (String, Vec<EngineEvent>) {
    futures_executor::block_on(async {
        let turn = session.prompt(prompt).await.unwrap();
        let turn_id = turn.id().to_string();
        let events = turn
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        (turn_id, events)
    })
}

fn lifecycle_error<T>(
    result: Result<T, NativeSessionLifecycleError>,
) -> NativeSessionLifecycleError {
    match result {
        Ok(value) => {
            drop(value);
            panic!("native session lifecycle operation unexpectedly succeeded");
        }
        Err(error) => error,
    }
}

fn assert_complete(events: &[EngineEvent]) {
    assert!(matches!(
        events.last().map(|event| &event.payload),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
}

#[test]
fn create_is_durable_and_duplicate_create_preserves_the_original_record() {
    let temporary = TemporaryDirectory::new("create");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let source = ScriptedIncarnationSource::ids(["create-life-one", "create-life-two"]);
    let (lifecycle, provider) = lifecycle(&store, Arc::clone(&source), []);
    let session_id = id("durable-create");

    let created = futures_executor::block_on(lifecycle.create(session_id.clone())).unwrap();
    let expected = SessionRecord {
        id: session_id.clone(),
        incarnation_id: incarnation("create-life-one"),
        revision: SessionRevision(1),
        next_turn_sequence: 1,
        messages: Vec::new(),
        metadata: Default::default(),
    };
    assert_eq!(created.record(), expected);
    assert_eq!(load(&store, session_id.clone()), Some(expected));
    assert!(provider.requests().is_empty());
    assert_eq!(source.calls(), 1);

    drop(created);
    let path = data_path(temporary.path());
    let prior = fs::read(&path).unwrap();
    let error = lifecycle_error(futures_executor::block_on(lifecycle.create(session_id)));
    assert_eq!(error.kind(), NativeSessionLifecycleErrorKind::AlreadyExists);
    assert_eq!(fs::read(path).unwrap(), prior);
    assert_eq!(
        source.calls(),
        1,
        "an existing record must be rejected before allocating an incarnation"
    );
    assert!(provider.requests().is_empty());
}

#[test]
fn resume_reconstructs_the_exact_transcript_and_continues_the_turn_allocator() {
    let temporary = TemporaryDirectory::new("resume");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let source = ScriptedIncarnationSource::ids(["resume-life"]);
    let (lifecycle, provider) = lifecycle(
        &store,
        source,
        [
            completed_step("first answer"),
            completed_step("second answer"),
        ],
    );
    let session_id = id("resume-transcript");
    let created = futures_executor::block_on(lifecycle.create(session_id.clone())).unwrap();

    let (first_turn, first_events) = collect_turn(&created, "first prompt");
    assert_eq!(first_turn, "turn-1");
    assert_complete(&first_events);
    let before_resume = load(&store, session_id.clone()).unwrap();
    assert_eq!(before_resume.revision, SessionRevision(3));
    assert_eq!(before_resume.next_turn_sequence, 2);
    assert_eq!(before_resume.messages.len(), 2);
    drop(created);

    let resumed = futures_executor::block_on(lifecycle.resume(session_id.clone())).unwrap();
    assert_eq!(resumed.record(), before_resume);
    let (second_turn, second_events) = collect_turn(&resumed, "second prompt");
    assert_eq!(second_turn, "turn-2");
    assert_complete(&second_events);

    let final_record = load(&store, session_id).unwrap();
    assert_eq!(final_record.revision, SessionRevision(5));
    assert_eq!(final_record.next_turn_sequence, 3);
    assert_eq!(final_record.messages.len(), 4);
    assert!(matches!(
        &final_record.messages[0].content[..],
        [ContentBlock::Text { text }] if text == "first prompt"
    ));
    assert!(matches!(
        &final_record.messages[1].content[..],
        [ContentBlock::Text { text }] if text == "first answer"
    ));
    assert!(matches!(
        &final_record.messages[2].content[..],
        [ContentBlock::Text { text }] if text == "second prompt"
    ));
    assert!(matches!(
        &final_record.messages[3].content[..],
        [ContentBlock::Text { text }] if text == "second answer"
    ));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.turn_id.to_string(), "turn-1");
    assert_eq!(requests[1].request.turn_id.to_string(), "turn-2");
    assert_eq!(requests[1].request.messages.len(), 3);
}

#[test]
fn replay_returns_a_validated_current_record_without_mutation() {
    let temporary = TemporaryDirectory::new("replay");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let mut expected = save_new(&store, id("replay-record"), incarnation("replay-life"));
    expected
        .messages
        .push(Message::text(Role::User, "durable prompt"));
    expected
        .metadata
        .insert("label".to_owned(), json!({"value": 7}));
    expected.revision = ready(store.save(expected.clone(), Some(expected.revision))).unwrap();
    let source = ScriptedIncarnationSource::ids([]);
    let (lifecycle, provider) = lifecycle(&store, source, []);
    let path = data_path(temporary.path());
    let entries = directory_entries(temporary.path());
    let prior = fs::read(&path).unwrap();

    let replayed = futures_executor::block_on(lifecycle.replay(expected.id.clone())).unwrap();
    assert_eq!(replayed, expected);
    assert_eq!(fs::read(path).unwrap(), prior);
    assert_eq!(directory_entries(temporary.path()), entries);
    assert!(provider.requests().is_empty());

    let missing = lifecycle_error(futures_executor::block_on(
        lifecycle.replay(id("missing-replay")),
    ));
    assert_eq!(missing.kind(), NativeSessionLifecycleErrorKind::NotFound);
    assert_eq!(directory_entries(temporary.path()), entries);
}

#[test]
fn reset_rotates_the_incarnation_clears_state_and_advances_physical_revision() {
    let temporary = TemporaryDirectory::new("reset");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let mut old = save_new(&store, id("reset-record"), incarnation("old-reset-life"));
    old.messages
        .push(Message::text(Role::User, "erase this transcript"));
    old.metadata.insert("private".to_owned(), json!(true));
    old.next_turn_sequence = 4;
    old.revision = ready(store.save(old.clone(), Some(old.revision))).unwrap();
    let source = ScriptedIncarnationSource::ids(["new-reset-life"]);
    let (lifecycle, provider) = lifecycle(&store, source, []);

    let reset = futures_executor::block_on(lifecycle.reset(old.id.clone())).unwrap();
    let reset_record = reset.record();
    assert_eq!(reset_record.id, old.id);
    assert_eq!(reset_record.incarnation_id, incarnation("new-reset-life"));
    assert_ne!(reset_record.incarnation_id, old.incarnation_id);
    assert_eq!(reset_record.revision, SessionRevision(old.revision.0 + 1));
    assert_eq!(reset_record.next_turn_sequence, 1);
    assert!(reset_record.messages.is_empty());
    assert!(reset_record.metadata.is_empty());
    assert_eq!(load(&store, reset_record.id.clone()), Some(reset_record));
    assert!(provider.requests().is_empty());

    let error = ready(store.save(old.clone(), Some(old.revision))).unwrap_err();
    assert_eq!(error.kind, SessionStoreErrorKind::Conflict);
    assert_eq!(error.code, "incarnation_conflict");
    assert!(!error.retryable);
}

#[test]
fn missing_and_unpolled_operations_have_no_identity_or_filesystem_effects() {
    let temporary = TemporaryDirectory::new("inert");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let source = ScriptedIncarnationSource::ids(["never-used-create", "never-used-reset"]);
    let (lifecycle, provider) = lifecycle(&store, Arc::clone(&source), []);

    drop(lifecycle.create(id("unpolled-create")));
    drop(lifecycle.resume(id("unpolled-resume")));
    drop(lifecycle.replay(id("unpolled-replay")));
    drop(lifecycle.reset(id("unpolled-reset")));
    assert_eq!(source.calls(), 0);
    assert!(directory_entries(temporary.path()).is_empty());

    let missing = lifecycle_error(futures_executor::block_on(
        lifecycle.reset(id("missing-reset")),
    ));
    assert_eq!(missing.kind(), NativeSessionLifecycleErrorKind::NotFound);
    assert_eq!(
        source.calls(),
        0,
        "missing reset must not allocate a lifetime"
    );
    assert!(directory_entries(temporary.path()).is_empty());
    assert!(provider.requests().is_empty());
}

#[test]
fn incarnation_source_failure_and_collision_preserve_the_prior_record() {
    let temporary = TemporaryDirectory::new("identity-failure");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let old = save_new(
        &store,
        id("identity-failure"),
        incarnation("existing-incarnation"),
    );
    let path = data_path(temporary.path());
    let prior = fs::read(&path).unwrap();

    let failing = ScriptedIncarnationSource::failing();
    let (lifecycle, _) = lifecycle(&store, Arc::clone(&failing), []);
    let error = lifecycle_error(futures_executor::block_on(lifecycle.reset(old.id.clone())));
    assert_eq!(
        error.kind(),
        NativeSessionLifecycleErrorKind::IncarnationSource
    );
    assert_eq!(failing.calls(), 1);
    assert_eq!(fs::read(&path).unwrap(), prior);

    let collisions = ScriptedIncarnationSource::ids([
        "existing-incarnation",
        "existing-incarnation",
        "existing-incarnation",
        "existing-incarnation",
        "existing-incarnation",
        "existing-incarnation",
        "existing-incarnation",
        "existing-incarnation",
    ]);
    let (lifecycle, _) = lifecycle(&store, Arc::clone(&collisions), []);
    let error = lifecycle_error(futures_executor::block_on(lifecycle.reset(old.id)));
    assert_eq!(
        error.kind(),
        NativeSessionLifecycleErrorKind::IncarnationSource
    );
    assert_eq!(collisions.calls(), MAX_SESSION_INCARNATION_ATTEMPTS);
    assert_eq!(fs::read(path).unwrap(), prior);
}

#[test]
fn a_live_local_incarnation_blocks_reset_without_changing_durable_bytes() {
    let temporary = TemporaryDirectory::new("live-local");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let old = save_new(&store, id("live-local"), incarnation("live-old-life"));
    let source = ScriptedIncarnationSource::ids(["live-new-life", "live-new-life-after-drop"]);
    let (lifecycle, _) = lifecycle(&store, source, []);
    let old_handle = futures_executor::block_on(lifecycle.resume(old.id.clone())).unwrap();
    let path = data_path(temporary.path());
    let prior = fs::read(&path).unwrap();

    let error = lifecycle_error(futures_executor::block_on(lifecycle.reset(old.id.clone())));
    assert_eq!(error.kind(), NativeSessionLifecycleErrorKind::LiveSession);
    assert_eq!(fs::read(&path).unwrap(), prior);

    drop(old_handle);
    let replacement = futures_executor::block_on(lifecycle.reset(old.id)).unwrap();
    assert_eq!(
        replacement.incarnation_id(),
        incarnation("live-new-life-after-drop")
    );
}

#[test]
fn corrupt_and_revision_exhausted_records_fail_closed_without_replacement() {
    {
        let temporary = TemporaryDirectory::new("corrupt-reset");
        let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
        let record = save_new(&store, id("corrupt-reset"), incarnation("corrupt-old-life"));
        let path = data_path(temporary.path());
        let corrupt = b"{PRIVATE_CORRUPT_SESSION_CONTENT";
        fs::write(&path, corrupt).unwrap();
        let source = ScriptedIncarnationSource::ids(["corrupt-new-life"]);
        let (lifecycle, _) = lifecycle(&store, Arc::clone(&source), []);

        let error = lifecycle_error(futures_executor::block_on(lifecycle.reset(record.id)));
        assert_eq!(error.kind(), NativeSessionLifecycleErrorKind::Corrupt);
        assert_eq!(source.calls(), 0);
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("PRIVATE_CORRUPT_SESSION_CONTENT"));
        assert!(!rendered.contains(temporary.path().to_string_lossy().as_ref()));
        assert_eq!(fs::read(path).unwrap(), corrupt);
    }

    {
        let temporary = TemporaryDirectory::new("exhausted-reset");
        let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
        let mut record = save_new(
            &store,
            id("exhausted-reset"),
            incarnation("exhausted-old-life"),
        );
        record.revision = SessionRevision(u64::MAX);
        let path = data_path(temporary.path());
        let exhausted = persisted_bytes(&record);
        fs::write(&path, &exhausted).unwrap();
        let source = ScriptedIncarnationSource::ids(["exhausted-new-life"]);
        let (lifecycle, _) = lifecycle(&store, source, []);

        let error = lifecycle_error(futures_executor::block_on(lifecycle.reset(record.id)));
        assert_eq!(error.kind(), NativeSessionLifecycleErrorKind::Engine);
        assert_eq!(fs::read(path).unwrap(), exhausted);
    }
}

#[test]
fn stale_reset_compare_and_swap_preserves_the_concurrently_committed_record() {
    let temporary = TemporaryDirectory::new("stale-reset");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let old = save_new(&store, id("stale-reset"), incarnation("stale-old-life"));
    let hook_store = Arc::clone(&store);
    let hook_id = old.id.clone();
    let source = ScriptedIncarnationSource::with_hook(["stale-new-life"], move || {
        let mut concurrent = load(&hook_store, hook_id).unwrap();
        concurrent
            .metadata
            .insert("concurrent".to_owned(), json!("winner"));
        let expected = concurrent.revision;
        let assigned = ready(hook_store.save(concurrent, Some(expected))).unwrap();
        assert_eq!(assigned, SessionRevision(expected.0 + 1));
    });
    let (lifecycle, _) = lifecycle(&store, source, []);

    let error = lifecycle_error(futures_executor::block_on(lifecycle.reset(old.id.clone())));
    assert_eq!(error.kind(), NativeSessionLifecycleErrorKind::Conflict);
    let durable = load(&store, old.id).unwrap();
    assert_eq!(durable.incarnation_id, old.incarnation_id);
    assert_eq!(durable.revision, SessionRevision(old.revision.0 + 1));
    assert_eq!(durable.metadata.get("concurrent"), Some(&json!("winner")));
}

#[test]
fn concurrent_prompt_and_reset_publish_one_complete_lifetime() {
    let temporary = TemporaryDirectory::new("prompt-reset-race");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let old = save_new(
        &store,
        id("prompt-reset-race"),
        incarnation("race-old-life"),
    );

    let (prompt_engine, _) = engine(&store, [completed_step("prompt winner")]);
    let old_handle = futures_executor::block_on(prompt_engine.load_session(old.id.clone()))
        .unwrap()
        .unwrap();

    let reached_source = Arc::new(Barrier::new(2));
    let release_source = Arc::new(Barrier::new(2));
    let reached = Arc::clone(&reached_source);
    let release = Arc::clone(&release_source);
    let source = ScriptedIncarnationSource::with_hook(["race-new-life"], move || {
        reached.wait();
        release.wait();
    });
    let (lifecycle, _) = lifecycle(&store, source, []);
    let reset_id = old.id.clone();
    let reset_worker =
        std::thread::spawn(move || futures_executor::block_on(lifecycle.reset(reset_id)));

    reached_source.wait();
    let (turn_id, events) = collect_turn(&old_handle, "prompt committed during reset");
    assert_eq!(turn_id, "turn-1");
    assert_complete(&events);
    release_source.wait();
    let reset_error = lifecycle_error(reset_worker.join().unwrap());
    let durable = load(&store, old.id).unwrap();
    assert_eq!(
        reset_error.kind(),
        NativeSessionLifecycleErrorKind::Conflict
    );
    assert_eq!(durable.incarnation_id, old.incarnation_id);
    assert_eq!(durable.messages.len(), 2);
    assert!(matches!(
        &durable.messages[0].content[..],
        [ContentBlock::Text { text }] if text == "prompt committed during reset"
    ));
    assert!(matches!(
        &durable.messages[1].content[..],
        [ContentBlock::Text { text }] if text == "prompt winner"
    ));
    assert_eq!(durable.next_turn_sequence, 2);
    assert!(
        directory_entries(temporary.path())
            .iter()
            .all(|name| !name.ends_with(".tmp"))
    );
}

#[test]
fn lifecycle_retains_the_exact_store_shared_with_its_engine() {
    let temporary = TemporaryDirectory::new("shared-store");
    let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
    let source = ScriptedIncarnationSource::ids([]);
    let (lifecycle, _) = lifecycle(&store, source, []);

    assert!(Arc::ptr_eq(lifecycle.session_store(), &store));
    assert!(std::ptr::eq(
        lifecycle.engine().session_store(),
        store.as_ref() as &dyn SessionStore,
    ));
}

#[test]
fn lifecycle_construction_rejects_a_mismatched_store_without_any_effect() {
    let first = TemporaryDirectory::new("mismatched-engine-store");
    let second = TemporaryDirectory::new("mismatched-lifecycle-store");
    let engine_store = Arc::new(FileSessionStore::open(first.path()).unwrap());
    let lifecycle_store = Arc::new(FileSessionStore::open(second.path()).unwrap());
    let (engine, provider) = engine(&engine_store, []);
    let source = ScriptedIncarnationSource::ids(["mismatch-never-allocated"]);
    let shared_source: Arc<dyn SessionIncarnationSource> = source.clone();

    let error =
        NativeSessionLifecycle::shared_incarnation_source(engine, lifecycle_store, shared_source)
            .unwrap_err();

    assert_eq!(
        error.kind(),
        NativeSessionLifecycleBuildErrorKind::MismatchedSessionStore
    );
    assert_eq!(error.kind().as_str(), "mismatched_session_store");
    assert_eq!(source.calls(), 0);
    assert!(provider.requests().is_empty());
    assert!(directory_entries(first.path()).is_empty());
    assert!(directory_entries(second.path()).is_empty());
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(first.path().to_string_lossy().as_ref()));
    assert!(!rendered.contains(second.path().to_string_lossy().as_ref()));
}

#[test]
fn lifecycle_failures_are_fixed_and_redact_ids_paths_and_record_content() {
    fn duplicate_render(label: &str, session_name: &str) -> String {
        let temporary = TemporaryDirectory::new(label);
        let store = Arc::new(FileSessionStore::open(temporary.path()).unwrap());
        let source = ScriptedIncarnationSource::ids(["first-private-life", "second-private-life"]);
        let (lifecycle, _) = lifecycle(&store, source, []);
        let session_id = id(session_name);
        let created = futures_executor::block_on(lifecycle.create(session_id.clone())).unwrap();
        drop(created);
        let error = lifecycle_error(futures_executor::block_on(lifecycle.create(session_id)));
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(label));
        assert!(!rendered.contains(session_name));
        assert!(!rendered.contains("second-private-life"));
        rendered
    }

    let first = duplicate_render("PRIVATE_ROOT_ALPHA", "PRIVATE_SESSION_ALPHA");
    let second = duplicate_render("PRIVATE_ROOT_BRAVO", "PRIVATE_SESSION_BRAVO");
    assert_eq!(first, second);
}
