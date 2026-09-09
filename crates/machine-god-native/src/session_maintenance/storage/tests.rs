use super::super::{ContentBlock, Message, Role, ToolCall, ToolCallId, ToolName, write_all};
use super::*;
use std::os::unix::fs::PermissionsExt;

#[path = "tests/faults.rs"]
mod faults;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum PublicationStage {
    BeforeCommit,
    AfterCommit,
    AfterCleanup,
}
type PublicationHook = Box<dyn FnMut(PublicationStage) -> Result<(), Error>>;
thread_local! {
    static PUBLICATION_HOOK: std::cell::RefCell<Option<PublicationHook>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn publication_checkpoint(stage: PublicationStage) -> Result<(), Error> {
    PUBLICATION_HOOK.with(|hook| {
        hook.borrow_mut()
            .as_mut()
            .map_or(Ok(()), |hook| hook(stage))
    })
}
pub(super) struct HookGuard;
impl HookGuard {
    pub(super) fn install(
        hook: impl FnMut(PublicationStage) -> Result<(), Error> + 'static,
    ) -> Self {
        PUBLICATION_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        Self
    }
}
impl Drop for HookGuard {
    fn drop(&mut self) {
        PUBLICATION_HOOK.with(|slot| *slot.borrow_mut() = None);
    }
}
use crate::session_maintenance::{
    NativeSessionCleanupMode as CleanupMode, NativeSessionCleanupStatus as Status,
    NativeSessionMaintenance,
};
use futures_executor::block_on;
use machine_god_core::CancellationToken;
use std::{fs, path::PathBuf, sync::Arc};

struct Fixture {
    root: PathBuf,
    store: Arc<FileSessionStore>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "mg-session-maintenance-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let store = Arc::new(FileSessionStore::open(&root).unwrap());
        Self { root, store }
    }
    fn write(&self, record: &SessionRecord) -> Vec<u8> {
        let bytes = serialize_record(record).unwrap();
        let path = self.root.join(SessionNames::for_id(&record.id).data);
        fs::write(&path, &bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        bytes
    }
    fn bytes(&self, id: &SessionId) -> Vec<u8> {
        fs::read(self.root.join(SessionNames::for_id(id).data)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn control() -> FileSessionScanControl {
    FileSessionScanControl {
        cancel: CancellationToken::new(),
        abandoned: CancellationToken::new(),
        after_read: None,
    }
}
fn record() -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("source").unwrap(),
        SessionIncarnationId::new("inc-source").unwrap(),
    );
    record.revision = SessionRevision(3);
    record.next_turn_sequence = 7;
    record.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        serde_json::json!({"schema_version":1,"title":"Before"}),
    );
    record.messages.push(Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "hello".to_owned(),
        }],
    });
    record
}
fn recover(f: &Fixture, id: &SessionId) -> Result<NativeSessionRecovery, Error> {
    f.store.maintenance_recover(
        id,
        &SessionId::new("copy").unwrap(),
        &SessionIncarnationId::new("inc-copy").unwrap(),
        &control(),
    )
}

#[test]
fn migrates_only_metadata_and_revision_then_is_byte_stable() {
    let f = Fixture::new();
    let original = record();
    f.write(&original);
    let NativeSessionMigration::Migrated(updated) = f
        .store
        .maintenance_migrate(&original.id, &control())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(updated.revision, SessionRevision(4));
    assert_eq!(updated.messages, original.messages);
    assert_eq!(updated.next_turn_sequence, 7);
    assert_eq!(updated.incarnation_id, original.incarnation_id);
    let metadata = NativeSessionMetadata::from_metadata(&updated.metadata).unwrap();
    assert!(metadata.workspace().is_none());
    assert!(metadata.created_at_ms().is_none());
    let bytes = f.bytes(&original.id);
    assert!(matches!(
        f.store
            .maintenance_migrate(&original.id, &control())
            .unwrap(),
        NativeSessionMigration::AlreadyCurrent(_)
    ));
    assert_eq!(f.bytes(&original.id), bytes);
}

#[test]
fn missing_metadata_remains_unknown_and_unsupported_version_unchanged() {
    let f = Fixture::new();
    let mut original = record();
    original.metadata.clear();
    f.write(&original);
    f.store
        .maintenance_migrate(&original.id, &control())
        .unwrap();
    let current = decode(&f.bytes(&original.id), &original.id).unwrap();
    assert_eq!(
        NativeSessionMetadata::from_metadata(&current.metadata).unwrap(),
        NativeSessionMetadata::default()
    );
    original.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        serde_json::json!({"schema_version":99}),
    );
    let bytes = f.write(&original);
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control()),
        Err(Error::UnsupportedVersion)
    ));
    assert_eq!(f.bytes(&original.id), bytes);
}

#[test]
fn corrupt_metadata_recovery_creates_copy_and_clears_control_authority() {
    let f = Fixture::new();
    let mut original = record();
    original.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        serde_json::json!("broken"),
    );
    original.metadata.insert(
        "machine_god.conversation_checkpoint".to_owned(),
        serde_json::json!("broken"),
    );
    let bytes = f.write(&original);
    let recovered = recover(&f, &original.id).unwrap();
    assert_eq!(f.bytes(&original.id), bytes);
    assert_eq!(recovered.record.messages, original.messages);
    assert_eq!(recovered.record.metadata.len(), 1);
    assert_eq!(recovered.record.revision, SessionRevision(1));
    assert_eq!(recovered.record.next_turn_sequence, 7);
    assert!(!recovered.truncated_source);
    assert!(matches!(
        recover(&f, &original.id),
        Err(Error::DestinationExists)
    ));
}

#[test]
fn torn_message_keeps_only_complete_prefix_and_marks_unknown_tool_results() {
    let f = Fixture::new();
    let mut original = record();
    original.messages.push(Message {
        role: Role::Assistant,
        content: vec![ContentBlock::ToolCall {
            call: ToolCall {
                id: ToolCallId::new("call-1").unwrap(),
                name: ToolName::new("exec").unwrap(),
                arguments: serde_json::json!({"command":"echo no replay"}),
            },
        }],
    });
    original.messages.push(Message {
        role: Role::Assistant,
        content: vec![ContentBlock::Text {
            text: "torn-value".to_owned(),
        }],
    });
    let bytes = f.write(&original);
    let cut = bytes
        .windows(10)
        .position(|window| window == b"torn-value")
        .unwrap()
        + 4;
    fs::write(
        f.root.join(SessionNames::for_id(&original.id).data),
        &bytes[..cut],
    )
    .unwrap();
    let recovered = recover(&f, &original.id).unwrap();
    assert!(recovered.truncated_source);
    assert_eq!(recovered.unknown_tool_results, 1);
    assert_eq!(recovered.record.messages.len(), 3);
    assert_eq!(recovered.record.messages[2].role, Role::Tool);
    assert_eq!(f.bytes(&original.id), bytes[..cut]);
}

#[test]
fn truncation_does_not_repair_headers_duplicates_or_unsupported_envelopes() {
    let original = record();
    let bytes = serialize_record(&original).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for invalid in [
        text.replace(
            "\"schema_version\":1,\"record\"",
            "\"schema_version\":99,\"record\"",
        ),
        text.replace("\"id\":\"source\"", "\"id\":\"source\",\"id\":\"source\""),
        text.replace(
            "\"text\":\"hello\"",
            "\"text\":\"hello\",\"text\":\"duplicate\"",
        ),
    ] {
        assert!(
            recovery::decode_source(&invalid.as_bytes()[..invalid.len() - 4], &original.id)
                .is_err()
        );
    }
    for cut in 0..text.find("messages").unwrap() {
        assert!(recovery::decode_source(&text.as_bytes()[..cut], &original.id).is_err());
    }
}

#[test]
fn oversize_cancel_and_active_writer_preserve_source() {
    let f = Fixture::new();
    let original = record();
    let bytes = f.write(&original);
    let cancelled = control();
    cancelled.cancel.cancel();
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &cancelled),
        Err(Error::Cancelled)
    ));
    let names = SessionNames::for_id(&original.id);
    let lock = open_lock(f.store.root.as_fd(), &names.lock).unwrap();
    let held = control();
    let guard = held.lock(&lock).unwrap_or_else(|_| panic!());
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control()),
        Err(Error::Busy)
    ));
    drop(guard);
    assert_eq!(f.bytes(&original.id), bytes);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(f.root.join(&names.data))
        .unwrap();
    file.set_len((MAX_FILE_SESSION_BYTES + 1) as u64).unwrap();
    assert!(matches!(
        f.store.maintenance_migrate(&original.id, &control()),
        Err(Error::Oversized)
    ));
}

#[test]
fn cleanup_removes_only_proven_reproducible_staging_after_explicit_apply() {
    let f = Fixture::new();
    let original = record();
    let bytes = f.write(&original);
    let names = SessionNames::for_id(&original.id);
    let _lock = open_lock(f.store.root.as_fd(), &names.lock).unwrap();
    let mut migrated = original.clone();
    assert!(migrate_metadata(&mut migrated).unwrap());
    let staging = create_new_temp(f.store.root.as_fd(), &names.temp).unwrap();
    write_all(&staging, &serialize_record(&migrated).unwrap()).unwrap();
    let report = f
        .store
        .maintenance_cleanup(CleanupMode::ReportOnly, &control())
        .unwrap();
    assert_eq!(report.outcomes, vec![Status::ReportOnly]);
    assert!(f.root.join(&names.temp).exists());
    let report = f
        .store
        .maintenance_cleanup(CleanupMode::Apply, &control())
        .unwrap();
    assert_eq!(report.outcomes, vec![Status::Completed]);
    assert!(!f.root.join(&names.temp).exists());
    assert_eq!(f.bytes(&original.id), bytes);
}

#[test]
fn cleanup_retains_partial_untrusted_and_active_staging() {
    let f = Fixture::new();
    let original = record();
    f.write(&original);
    let names = SessionNames::for_id(&original.id);
    let lock = open_lock(f.store.root.as_fd(), &names.lock).unwrap();
    fs::write(f.root.join(&names.temp), b"partial").unwrap();
    let report = f
        .store
        .maintenance_cleanup(CleanupMode::Apply, &control())
        .unwrap();
    assert_eq!(report.outcomes, vec![Status::Untrusted]);
    let held = control();
    let _guard = held.lock(&lock).unwrap_or_else(|_| panic!());
    let report = f
        .store
        .maintenance_cleanup(CleanupMode::Apply, &control())
        .unwrap();
    assert_eq!(report.outcomes, vec![Status::ActiveWriter]);
    assert_eq!(fs::read(f.root.join(&names.temp)).unwrap(), b"partial");
}

#[test]
fn owned_facade_is_inert_and_actual_workers_join() {
    let f = Fixture::new();
    let original = record();
    let before = f.write(&original);
    let scope = crate::NativeOwnedWorkerScope::new();
    let service = NativeSessionMaintenance::new(f.store.clone(), scope.clone());
    drop(service.migrate(original.id.clone(), CancellationToken::new()));
    assert_eq!(f.bytes(&original.id), before);
    assert!(matches!(
        block_on(service.migrate(original.id, CancellationToken::new())).unwrap(),
        NativeSessionMigration::Migrated(_)
    ));
    scope.close();
    scope.completion().wait_on_worker().unwrap();
    assert!(scope.completion().is_complete());
}
