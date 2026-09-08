use super::*;
use crate::{
    MAX_LIST_SESSION_DIRECTORY_ENTRIES, NATIVE_SESSION_METADATA_KEY, NativeConversationRuntime,
    NativeReasoningEffort, NativeSessionMetadata, NativeSessionOrigin,
};
use futures_executor::block_on;
use machine_god_core::{Engine, Message, Role, SessionRecord};
use machine_god_testkit::{ScriptedModelProvider, ScriptedPermissionHandler};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
    lifecycle: NativeSessionLifecycle,
    store: Arc<FileSessionStore>,
    provider: ScriptedModelProvider,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "machine-god-resume-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let store = Arc::new(FileSessionStore::open(&root).unwrap());
        let provider = ScriptedModelProvider::new("resume-test", []);
        let engine = Engine::builder()
            .provider(provider.clone())
            .shared_session_store(store.clone())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let lifecycle = NativeSessionLifecycle::new(engine, store.clone()).unwrap();
        Self {
            root,
            lifecycle,
            store,
            provider,
        }
    }
    fn save(&self, name: &str, time: i64, workspace: &str) {
        let mut record = SessionRecord::empty(
            id(name),
            SessionIncarnationId::new(format!("life-{name}")).unwrap(),
        );
        record.messages = vec![
            Message::text(Role::User, "preserved input"),
            Message::text(Role::Assistant, "preserved answer"),
        ];
        record
            .metadata
            .insert("unrelated".into(), json!({"preserve": true}));
        record.metadata.insert(
            NATIVE_SESSION_METADATA_KEY.into(),
            NativeSessionMetadata::new(Path::new(workspace), time, NativeSessionOrigin::Cli)
                .unwrap()
                .to_value(),
        );
        block_on(self.store.save(record, None)).unwrap();
    }
    fn prepare(&self, target: NativeResumeTarget) -> Result<NativePreparedResume, Error> {
        block_on(prepare_native_session_resume(
            &self.lifecycle,
            target,
            Path::new("/workspace"),
            100,
        ))
    }
    fn record(&self, name: &str) -> SessionRecord {
        block_on(self.store.load(id(name))).unwrap().unwrap()
    }
    fn change(&self, name: &str) {
        let mut record = self.record(name);
        let expected = record.revision;
        record.metadata.insert("changed".into(), json!(true));
        block_on(self.store.save(record, Some(expected))).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn id(name: &str) -> SessionId {
    SessionId::new(name).unwrap()
}

#[test]
fn latest_uses_authoritative_workspace_rank_and_ties_without_rebinding() {
    let fixture = Fixture::new();
    fixture.save("older", 1, "/workspace");
    fixture.save("a", 7, "/workspace");
    fixture.save("z", 7, "/workspace");
    fixture.save("foreign", 90, "/foreign");
    let before = fixture.record("z");
    let prepared = fixture.prepare(NativeResumeTarget::Latest).unwrap();
    assert_eq!(prepared.id(), &id("z"));
    assert_eq!(prepared.incarnation_id(), &before.incarnation_id);
    assert_eq!(prepared.revision(), before.revision);
    let conversation = block_on(prepared.adopt()).unwrap();
    assert_eq!(conversation.record(), before);
    assert_eq!(fixture.record("z"), before);
    assert!(fixture.provider.requests().is_empty());
}

#[test]
fn exact_is_independent_of_enumeration_and_rebinds_without_losing_history() {
    let fixture = Fixture::new();
    fixture.save("chosen", 1, "/elsewhere");
    for n in 0..=MAX_LIST_SESSION_DIRECTORY_ENTRIES {
        fs::write(fixture.root.join(format!("ignored-{n}")), []).unwrap();
    }
    assert_eq!(
        fixture
            .prepare(NativeResumeTarget::Latest)
            .unwrap_err()
            .kind(),
        Kind::SelectionIncomplete
    );
    let before = fixture.record("chosen");
    let prepared = fixture
        .prepare(NativeResumeTarget::Exact(id("chosen")))
        .unwrap();
    assert_eq!(prepared.revision(), SessionRevision(before.revision.0 + 1));
    let conversation = block_on(prepared.adopt()).unwrap();
    let after = conversation.record();
    assert_eq!(after.messages, before.messages);
    assert_eq!(after.next_turn_sequence, before.next_turn_sequence);
    assert_eq!(after.incarnation_id, before.incarnation_id);
    assert_eq!(after.metadata["unrelated"], before.metadata["unrelated"]);
    let metadata = NativeSessionMetadata::from_metadata(&after.metadata).unwrap();
    assert_eq!(metadata.workspace(), Some(Path::new("/workspace")));
    assert_eq!(metadata.origin(), Some(NativeSessionOrigin::Cli));
    assert_eq!(metadata.created_at_ms(), Some(1));
    assert_eq!(fixture.record("chosen"), after);
}

#[test]
fn exact_same_workspace_is_a_revision_preserving_noop() {
    let fixture = Fixture::new();
    fixture.save("same", 1, "/workspace");
    let before = fixture.record("same");
    let prepared = fixture
        .prepare(NativeResumeTarget::Exact(id("same")))
        .unwrap();
    assert_eq!(prepared.revision(), before.revision);
    assert_eq!(block_on(prepared.adopt()).unwrap().record(), before);
    assert_eq!(fixture.record("same"), before);
}

#[test]
fn unknown_eligible_activity_refuses_latest_but_exact_can_establish_association() {
    let fixture = Fixture::new();
    fixture.save("unknown", 1, "/workspace");
    let mut record = fixture.record("unknown");
    let expected = record.revision;
    let metadata = record
        .metadata
        .get_mut(NATIVE_SESSION_METADATA_KEY)
        .unwrap();
    metadata["created_at_ms"] = json!(null);
    metadata["updated_at_ms"] = json!(null);
    block_on(fixture.store.save(record, Some(expected))).unwrap();
    assert_eq!(
        fixture
            .prepare(NativeResumeTarget::Latest)
            .unwrap_err()
            .kind(),
        Kind::SelectionIncomplete
    );
    let prepared = block_on(prepare_native_session_resume(
        &fixture.lifecycle,
        NativeResumeTarget::Exact(id("unknown")),
        Path::new("/new"),
        100,
    ))
    .unwrap();
    let record = block_on(prepared.adopt()).unwrap().record();
    let metadata = NativeSessionMetadata::from_metadata(&record.metadata).unwrap();
    assert_eq!(metadata.created_at_ms(), None);
    assert_eq!(metadata.workspace(), Some(Path::new("/new")));
}

#[test]
fn construction_drop_and_missing_target_are_inert_without_creating_sessions() {
    let fixture = Fixture::new();
    let future = prepare_native_session_resume(
        &fixture.lifecycle,
        NativeResumeTarget::Exact(id("missing")),
        Path::new("/workspace"),
        100,
    );
    drop(future);
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
    assert_eq!(
        fixture
            .prepare(NativeResumeTarget::Exact(id("missing")))
            .unwrap_err()
            .kind(),
        Kind::NotFound
    );
    assert_eq!(
        fixture
            .prepare(NativeResumeTarget::Latest)
            .unwrap_err()
            .kind(),
        Kind::NotFound
    );
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
    assert!(fixture.provider.requests().is_empty());
}

#[test]
fn invalid_workspace_and_native_metadata_fail_before_publication() {
    let fixture = Fixture::new();
    fixture.save("bad", 1, "/workspace");
    let original = fixture.record("bad");
    assert_eq!(
        block_on(prepare_native_session_resume(
            &fixture.lifecycle,
            NativeResumeTarget::Exact(id("bad")),
            Path::new("relative"),
            100
        ))
        .unwrap_err()
        .kind(),
        Kind::InvalidWorkspace
    );
    assert_eq!(fixture.record("bad"), original);
    for key in [
        crate::NATIVE_CONVERSATION_CHECKPOINT_KEY,
        crate::NATIVE_CONTEXT_PREFERENCES_KEY,
        crate::NATIVE_MODEL_PREFERENCES_KEY,
        crate::NATIVE_WORKSPACE_REBINDINGS_KEY,
        NATIVE_SESSION_METADATA_KEY,
    ] {
        let mut record = fixture.record("bad");
        let expected = record.revision;
        record.metadata = original.metadata.clone();
        record
            .metadata
            .insert(key.to_owned(), json!({"schema_version": 99}));
        block_on(fixture.store.save(record, Some(expected))).unwrap();
        let before = fixture.record("bad");
        assert_eq!(
            fixture
                .prepare(NativeResumeTarget::Exact(id("bad")))
                .unwrap_err()
                .kind(),
            Kind::Corrupt
        );
        assert_eq!(
            fixture
                .prepare(NativeResumeTarget::Latest)
                .unwrap_err()
                .kind(),
            Kind::Corrupt
        );
        assert_eq!(fixture.record("bad"), before);
    }
}

#[test]
fn selected_target_revision_or_incarnation_replacement_is_not_silently_adopted() {
    for reset in [false, true] {
        let fixture = Fixture::new();
        fixture.save("chosen", 1, "/workspace");
        let result = block_on(prepare(
            &fixture.lifecycle,
            NativeResumeTarget::Exact(id("chosen")),
            Path::new("/workspace"),
            100,
            || {
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            if reset {
                                drop(block_on(fixture.lifecycle.reset(id("chosen"))).unwrap());
                            } else {
                                fixture.change("chosen");
                            }
                        })
                        .join()
                        .unwrap();
                });
            },
            || {},
        ));
        assert_eq!(result.unwrap_err().kind(), Kind::Conflict);
    }
}

#[test]
fn guarded_load_rejects_changed_record_without_reconciling_current_runtime() {
    let fixture = Fixture::new();
    fixture.save("current", 1, "/workspace");
    let conversation = block_on(NativeConversation::resume(
        &fixture.lifecycle,
        id("current"),
    ))
    .unwrap();
    let runtime = NativeConversationRuntime::new(
        conversation,
        NativeModelPreferences::new("test", NativeReasoningEffort::default(), false).unwrap(),
        None,
    )
    .unwrap();
    runtime.enqueue("queued unchanged".into()).unwrap();
    let before = runtime.record();
    let status = runtime.status();
    let result = block_on(prepare(
        &fixture.lifecycle,
        NativeResumeTarget::Exact(id("current")),
        Path::new("/workspace"),
        100,
        || {},
        || std::thread::scope(|scope| scope.spawn(|| fixture.change("current")).join().unwrap()),
    ));
    assert_eq!(result.unwrap_err().kind(), Kind::Conflict);
    assert_eq!(runtime.record(), before);
    assert_eq!(runtime.status(), status);
    assert_eq!(runtime.model_preferences().model(), "test");
}

#[test]
fn adoption_rejects_canonical_or_durable_changes_and_is_inert_before_poll() {
    for canonical in [false, true] {
        let fixture = Fixture::new();
        fixture.save("chosen", 1, "/workspace");
        let prepared = fixture.prepare(NativeResumeTarget::Latest).unwrap();
        let old = prepared.revision();
        if canonical {
            let session = block_on(fixture.lifecycle.resume(id("chosen"))).unwrap();
            block_on(crate::rename_native_session(&session, "changed", 100)).unwrap();
        } else {
            fixture.change("chosen");
        }
        assert_eq!(
            block_on(prepared.adopt()).unwrap_err().kind(),
            Kind::Conflict
        );
        assert!(fixture.record("chosen").revision > old);
    }
    let fixture = Fixture::new();
    fixture.save("chosen", 1, "/workspace");
    let prepared = fixture.prepare(NativeResumeTarget::Latest).unwrap();
    let before = fixture.record("chosen");
    drop(prepared.adopt());
    assert_eq!(fixture.record("chosen"), before);
}

#[test]
fn busy_canonical_target_is_rejected_without_cancelling_its_turn() {
    let fixture = Fixture::new();
    fixture.save("busy", 1, "/workspace");
    let prepared = fixture.prepare(NativeResumeTarget::Latest).unwrap();
    let session = block_on(fixture.lifecycle.resume(id("busy"))).unwrap();
    let turn = block_on(session.prompt("active")).unwrap();
    assert_eq!(block_on(prepared.adopt()).unwrap_err().kind(), Kind::Busy);
    assert_eq!(
        fixture
            .prepare(NativeResumeTarget::Exact(id("busy")))
            .unwrap_err()
            .kind(),
        Kind::Busy
    );
    assert!(session.has_active_turn());
    assert!(!turn.handle().is_cancelled());
    drop(turn);
}

#[test]
fn rebind_failure_leaves_current_runtime_and_original_association_intact() {
    let fixture = Fixture::new();
    fixture.save("current", 1, "/workspace");
    fixture.save("future", 200, "/elsewhere");
    let current = block_on(NativeConversation::resume(
        &fixture.lifecycle,
        id("current"),
    ))
    .unwrap();
    let before = current.record();
    let target_before = fixture.record("future");
    assert!(
        fixture
            .prepare(NativeResumeTarget::Exact(id("future")))
            .is_err()
    );
    assert_eq!(current.record(), before);
    assert_eq!(fixture.record("future"), target_before);
    assert!(fixture.provider.requests().is_empty());
}

#[test]
fn missing_historical_workspace_stays_unknown_until_explicit_exact_rebind() {
    let fixture = Fixture::new();
    fixture.save("legacy", 1, "/old");
    let mut record = fixture.record("legacy");
    let expected = record.revision;
    record.metadata.remove(NATIVE_SESSION_METADATA_KEY);
    block_on(fixture.store.save(record, Some(expected))).unwrap();
    assert_eq!(
        fixture
            .prepare(NativeResumeTarget::Latest)
            .unwrap_err()
            .kind(),
        Kind::NotFound
    );
    let prepared = fixture
        .prepare(NativeResumeTarget::Exact(id("legacy")))
        .unwrap();
    let record = block_on(prepared.adopt()).unwrap().record();
    let metadata = NativeSessionMetadata::from_metadata(&record.metadata).unwrap();
    assert_eq!(metadata.workspace(), Some(Path::new("/workspace")));
    assert_eq!(metadata.created_at_ms(), None);
    assert_eq!(metadata.origin(), None);
}

#[test]
fn same_revision_durable_drift_is_not_an_adoption_receipt() {
    let fixture = Fixture::new();
    fixture.save("drift", 1, "/workspace");
    let prepared = fixture.prepare(NativeResumeTarget::Latest).unwrap();
    let current = block_on(fixture.lifecycle.resume(id("drift"))).unwrap();
    let before = current.record();
    let path = fs::read_dir(&fixture.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let mut wire: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    wire["record"]["metadata"]["unrelated"] = json!("changed without a revision");
    fs::write(&path, serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_eq!(
        block_on(prepared.adopt()).unwrap_err().kind(),
        Kind::Conflict
    );
    assert_eq!(current.record(), before);
}
