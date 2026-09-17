use super::*;
use crate::managed::{
    notices::{
        ManagedNotices, NoticeLimits, NoticePrincipal, NoticeRelationship, NoticeTerminal,
        PreparedNotice, WorkNoticeIdentity,
    },
    prompt_context::{ParentNoticeContext, saved_outbox},
    store::{JournalCreate, JournalIntent, JournalLimits, JournalPublication, JournalTranscript},
};
use crate::mcp::runtime::NativeMcpRuntimeClock;
use crate::{NativeConversation, NativeOwnedWorkerScope};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, Engine, ManagedAgentMode, ManagedAgentState, ManagedConfiguration,
    ManagedNotifications, ManagedPermissionMode, ModelEvent, Session, SessionId,
    SessionIncarnationId, StopReason,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use rustix::fs::{Mode, OFlags};
use std::{
    future::Future,
    num::NonZeroU64,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

struct Clock;
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

struct Fixture {
    path: PathBuf,
    workers: NativeOwnedWorkerScope,
    entries: usize,
}
impl Fixture {
    fn new() -> Self {
        Self::with_entries(14)
    }
    fn with_entries(entries: usize) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "mg-archived-delivery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            path,
            workers: NativeOwnedWorkerScope::new(),
            entries,
        }
    }
    fn open(&self) -> Result<ManagedJournal, JournalError> {
        let root = rustix::fs::open(
            &self.path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        block_on(ManagedJournal::open(
            root,
            self.workers.clone(),
            JournalLimits {
                directory_entries: self.entries,
                ..JournalLimits::default()
            },
        ))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.workers.close();
        block_on(self.workers.completion().wait());
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
fn confirmed(publication: JournalPublication) -> JournalSnapshot {
    let JournalPublication::Confirmed(snapshot) = publication else {
        panic!("actual confirmed publication required")
    };
    *snapshot
}
fn publish(
    journal: &ManagedJournal,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
) -> JournalSnapshot {
    confirmed(block_on(journal.mutate(snapshot, mutation)).unwrap())
}
fn principal(id: &str) -> NoticePrincipal {
    NoticePrincipal {
        id: id.into(),
        generation: NonZeroU64::new(1).unwrap(),
    }
}
fn engine(provider: &ScriptedModelProvider) -> Engine {
    Engine::builder()
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .session_store(InMemorySessionStore::default())
        .build()
        .unwrap()
}
fn transcript(session: &Session) -> JournalTranscript {
    JournalTranscript {
        session_id: session.id(),
        incarnation: session.incarnation_id(),
    }
}
struct Delivered {
    session: Session,
    conversation: NativeConversation,
    context: Arc<ParentNoticeContext>,
    notices: Arc<ManagedNotices>,
    provider: ScriptedModelProvider,
    original: ManagedNotice,
    snapshot: JournalSnapshot,
}

#[allow(clippy::too_many_lines)] // Real journal original and actual parent checkpoint share one fixture.
fn source_delivery(
    journal: &ManagedJournal,
    foreign_incarnation: bool,
    archived: bool,
) -> Delivered {
    let provider = ScriptedModelProvider::new(
        "parent",
        [ModelProviderStep::events([ModelEvent::Stop {
            reason: StopReason::Completed,
        }])],
    );
    let engine = engine(&provider);
    let parent = engine
        .create_session(
            SessionId::new("parent").unwrap(),
            SessionIncarnationId::new("original-parent").unwrap(),
        )
        .unwrap();
    let source = engine
        .create_session(
            SessionId::new("source").unwrap(),
            SessionIncarnationId::new("source-life").unwrap(),
        )
        .unwrap();
    let mut snapshot = confirmed(
        block_on(journal.create(JournalCreate {
            id: "source".into(),
            mode: ManagedAgentMode::Persistent,
            configuration: ManagedConfiguration {
                name: "source".into(),
                model: None,
                effort: None,
                permission_mode: ManagedPermissionMode::Ask,
                notifications: ManagedNotifications::default(),
            },
            transcript: transcript(&source),
            controller: transcript(&parent),
            parent_id: Some(parent.id().to_string()),
            parent_owner: Some(transcript(&parent)),
            parent_generation: Some(1),
            initial_work: None,
        }))
        .unwrap(),
    );
    let session = if foreign_incarnation {
        self::engine(&provider)
            .create_session(
                parent.id(),
                SessionIncarnationId::new("foreign-parent").unwrap(),
            )
            .unwrap()
    } else {
        parent
    };
    let notices = Arc::new(ManagedNotices::new(NoticeLimits::default(), Arc::new(Clock)).unwrap());
    let work = notices
        .register_work(
            &WorkNoticeIdentity {
                source: principal("source"),
                work_id: "original-work".into(),
                work_generation: NonZeroU64::new(1).unwrap(),
            },
            ManagedNotifications::default(),
            &NoticeRelationship {
                parent: Some(principal("parent")),
                parent_incarnation: Some(session.incarnation_id()),
                generation: NonZeroU64::new(snapshot.head.revision).unwrap(),
            },
            snapshot.head.notice_cursor,
        )
        .unwrap();
    let PreparedNotice::Staged(stage) = notices
        .prepare_terminal(
            &work,
            NonZeroU64::new(snapshot.head.next_sequence).unwrap(),
            NoticeTerminal::Completed,
            None,
        )
        .unwrap()
    else {
        panic!("original notice stage")
    };
    let original = stage.notice().clone();
    snapshot = publish(
        journal,
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::Notice(original.clone())]),
    );
    notices.confirm_durable(&stage).unwrap();
    let context = Arc::new(ParentNoticeContext::new(
        &session,
        principal("parent"),
        &notices,
    ));
    let conversation = NativeConversation::from_session(session.clone())
        .unwrap()
        .with_notice_context(&context)
        .unwrap();
    let turn = block_on(conversation.prompt("explicit next parent input".into(), 102)).unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let delivery = context.delivery().unwrap();
    assert_eq!(delivery.originals(), std::slice::from_ref(&original));
    assert!(saved_outbox(&session.record()).unwrap().is_some());
    assert!(block_on(conversation.clear_notice_delivery(&delivery)).is_err());
    if archived {
        snapshot = publish(
            journal,
            snapshot,
            JournalMutation::Intent(JournalIntent::Archive),
        );
        // Retain four bounded accepted maintenance pages before the final archive.
        // At the resulting physical entry boundary only the exact ACK can fit.
        for _ in 0..4 {
            let sequence = snapshot.head.next_sequence;
            snapshot = publish(
                journal,
                snapshot,
                JournalMutation::SuppressedNotice(sequence),
            );
        }
        snapshot = publish(journal, snapshot, JournalMutation::Archive);
        assert_eq!(snapshot.head.cleanup_bytes, 0);
        assert_eq!(snapshot.head.cleanup_entries, 0);
    }
    assert_eq!(
        snapshot.head.notice_reservations,
        vec![original.source_sequence.get()]
    );
    Delivered {
        session,
        conversation,
        context,
        notices,
        provider,
        original,
        snapshot,
    }
}

fn checked_step(
    journal: &ManagedJournal,
    delivery: &NoticeDelivery,
    progress: &mut Progress,
    repaired: &mut Vec<JournalSnapshot>,
) -> Result<bool, ManagedRuntimeError> {
    let gate = Arc::new(durability::RetryGate::default());
    let mut future = Box::pin(step(journal, &gate, delivery, progress, repaired));
    block_on(std::future::poll_fn(|cx| {
        let result = future.as_mut().poll(cx);
        if result.is_pending() {
            assert!(
                gate.issue().is_none(),
                "delivery attempted uncredited generic recovery instead of exact ACK"
            );
        }
        result
    }))
}

#[allow(clippy::too_many_lines)] // Exact checkpoint, owner restart, ACK publication and outbox clear lifecycle.
fn reopened_source_uses_reserved_ack(archived: bool) {
    let fixture = Fixture::new();
    let mut journal = fixture.open().unwrap();
    let mut delivered = source_delivery(&journal, false, archived);
    if !archived {
        // Real owner restarts exhaust finite no-op recovery allowance without
        // changing the original quiescent work state or its saved parent outbox.
        for _ in 0..6 {
            drop(journal);
            journal = fixture.open().unwrap();
            let snapshot = block_on(journal.inspect("source".into())).unwrap();
            delivered.snapshot = publish(&journal, snapshot, JournalMutation::Recover);
        }
        assert_eq!(delivered.snapshot.head.cleanup_entries, 0);
    }
    assert!(matches!(fixture.open(), Err(JournalError::Busy)));
    drop(journal);
    let journal = fixture.open().unwrap();
    let snapshot = block_on(journal.inspect("source".into())).unwrap();
    assert!(snapshot.recovery_required());
    assert_eq!(snapshot.head.revision, delivered.snapshot.head.revision);
    assert!(matches!(
        block_on(journal.mutate(snapshot.clone(), JournalMutation::Recover)),
        Err(JournalError::Limit)
    ));
    let receipt = delivered.context.delivery().unwrap();
    let mut progress = Progress::default();
    let mut repaired = Vec::new();
    let mut complete = false;
    for _ in 0..16 {
        if checked_step(&journal, &receipt, &mut progress, &mut repaired).unwrap() {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(
        repaired.len(),
        1,
        "no generic Recover publication may precede the ACK"
    );
    let after = block_on(journal.inspect("source".into())).unwrap();
    assert_eq!(after.head.revision, snapshot.head.revision + 1);
    assert_eq!(
        after.head.status,
        if archived {
            ManagedAgentState::Archived
        } else {
            ManagedAgentState::Idle
        }
    );
    assert!(!after.recovery_required());
    assert!(after.head.notice_reservations.is_empty());
    let records = block_on(journal.history(after, None, 100)).unwrap().records;
    assert!(records.contains(&JournalRecord::NoticeAcknowledged {
        identity: delivered.original.identity(),
        target: delivered.original.target.clone(),
        checkpoint: receipt.checkpoint().clone()
    }));
    assert!(records.contains(&JournalRecord::Notice(delivered.original.clone())));
    assert!(block_on(delivered.conversation.clear_notice_delivery(&receipt)).is_err());
    // This receipt is granted only after the real source publication above.
    delivered
        .notices
        .acknowledge_recovered(receipt.originals())
        .unwrap();
    delivered
        .context
        .confirm_source_acknowledgements(&receipt, &[delivered.original.identity()])
        .unwrap();
    block_on(delivered.conversation.clear_notice_delivery(&receipt)).unwrap();
    assert!(receipt.is_cleared());
    assert!(saved_outbox(&delivered.session.record()).unwrap().is_none());
    assert!(
        delivered
            .context
            .confirm_source_acknowledgements(&receipt, &[delivered.original.identity()])
            .is_err()
    );
    assert_eq!(delivered.provider.requests().len(), 1);
}

#[test]
fn reopened_archived_source_uses_only_its_reserved_exact_ack_then_clears_actual_outbox() {
    reopened_source_uses_reserved_ack(true);
}

#[test]
fn reopened_idle_source_uses_reserved_ack_after_actual_restarts_exhaust_generic_recovery() {
    reopened_source_uses_reserved_ack(false);
}

#[test]
fn reopened_archived_source_rejects_real_foreign_checkpoint_without_spending_credit() {
    let fixture = Fixture::new();
    let journal = fixture.open().unwrap();
    let delivered = source_delivery(&journal, true, true);
    drop(journal);
    let journal = fixture.open().unwrap();
    let before = block_on(journal.inspect("source".into())).unwrap();
    let receipt = delivered.context.delivery().unwrap();
    let mut progress = Progress::default();
    let mut repaired = Vec::new();
    let mut rejected = false;
    for _ in 0..16 {
        match checked_step(&journal, &receipt, &mut progress, &mut repaired) {
            Err(ManagedRuntimeError::Invalid) => {
                rejected = true;
                break;
            }
            Ok(false) => {}
            other => panic!("foreign original recipient must not ACK: {other:?}"),
        }
    }
    assert!(rejected);
    assert!(repaired.is_empty());
    let after = block_on(journal.inspect("source".into())).unwrap();
    assert_eq!(after.head, before.head);
    assert!(after.recovery_required());
    assert_eq!(after.head.notice_reservations.len(), 1);
    assert!(block_on(delivered.conversation.clear_notice_delivery(&receipt)).is_err());
    assert!(saved_outbox(&delivered.session.record()).unwrap().is_some());
    assert_eq!(delivered.provider.requests().len(), 1);
}

#[test]
#[allow(clippy::too_many_lines)] // Real live recovery precedes an exact ACK-only quiescent owner transfer.
fn ack_repair_requires_live_work_recovery_and_preserves_quiescent_accepted_intent() {
    use crate::managed::store::JournalWork;
    use machine_god_core::ManagedQueueStatus;
    let fixture = Fixture::with_entries(64);
    let journal = fixture.open().unwrap();
    let delivered = source_delivery(&journal, false, false);
    let configuration = delivered.snapshot.head.configuration.clone();
    let snapshot = publish(
        &journal,
        delivered.snapshot.clone(),
        JournalMutation::Enqueue(JournalWork {
            id: "accepted-work".into(),
            source_id: delivered.session.id().to_string(),
            source_owner: transcript(&delivered.session),
            content: "standalone accepted work".into(),
            skills: Vec::new(),
            accepted_at_ms: 103,
            configuration,
        }),
    );
    let snapshot = publish(
        &journal,
        snapshot,
        JournalMutation::Intent(JournalIntent::Cancel),
    );
    assert!(snapshot.head.recovery_changes_work());
    // Pure predicate inputs only: these clones never enter publication or
    // become execution/receipt authority. Intent does not affect Recover's work changes.
    let mut evidence = snapshot.head.clone();
    for (status, changes) in [
        (ManagedQueueStatus::Pending, true),
        (ManagedQueueStatus::Running, true),
        (ManagedQueueStatus::AwaitingApproval, true),
        (ManagedQueueStatus::Interrupted, false),
        (ManagedQueueStatus::Failed, false),
        (ManagedQueueStatus::Completed, false),
        (ManagedQueueStatus::Cancelled, false),
    ] {
        evidence.queue[0].status = status;
        assert_eq!(evidence.recovery_changes_work(), changes);
    }
    evidence.status = ManagedAgentState::Archived;
    evidence.queue[0].status = ManagedQueueStatus::Pending;
    assert!(!evidence.recovery_changes_work());
    drop(journal);
    let journal = fixture.open().unwrap();
    let snapshot = block_on(journal.inspect("source".into())).unwrap();
    assert_eq!(snapshot.head.status, ManagedAgentState::Queued);
    assert_eq!(
        crate::managed::manager::projection::observed_status(&snapshot),
        ManagedAgentState::Interrupted
    );
    assert_eq!(snapshot.head.queue[0].status, ManagedQueueStatus::Pending);
    assert_eq!(
        block_on(journal.inspect("source".into())).unwrap().head,
        snapshot.head
    );
    let receipt = delivered.context.delivery().unwrap();
    let ack = JournalMutation::AppendHistory(vec![JournalRecord::NoticeAcknowledged {
        identity: delivered.original.identity(),
        target: delivered.original.target.clone(),
        checkpoint: receipt.checkpoint().clone(),
    }]);
    assert!(matches!(
        block_on(journal.mutate(snapshot.clone(), ack)),
        Err(JournalError::RecoveryRequired)
    ));
    assert_eq!(
        block_on(journal.inspect("source".into())).unwrap().head,
        snapshot.head
    );
    let snapshot = publish(&journal, snapshot, JournalMutation::Recover);
    assert!(!snapshot.head.recovery_changes_work());
    assert_eq!(
        snapshot.head.queue[0].status,
        ManagedQueueStatus::Interrupted
    );
    assert_eq!(snapshot.head.intent, Some(JournalIntent::Cancel));
    drop(journal);
    let journal = fixture.open().unwrap();
    let mut progress = Progress::default();
    let mut repaired = Vec::new();
    for _ in 0..16 {
        if checked_step(&journal, &receipt, &mut progress, &mut repaired).unwrap() {
            break;
        }
    }
    assert_eq!(progress.processed, 1);
    assert_eq!(repaired.len(), 1);
    let after = block_on(journal.inspect("source".into())).unwrap();
    assert_eq!(after.head.revision, snapshot.head.revision + 1);
    assert_eq!(after.head.status, snapshot.head.status);
    assert_eq!(after.head.queue, snapshot.head.queue);
    assert_eq!(after.head.intent, snapshot.head.intent);
    assert!(!after.recovery_required());
    assert_eq!(delivered.provider.requests().len(), 1);
}
