use super::super::notices::*;
use super::*;
#[path = "outbox_tests.rs"]
mod outbox_tests;
use crate::mcp::runtime::NativeMcpRuntimeClock;
use futures_executor::block_on;
use futures_util::task::noop_waker_ref;
use machine_god_core::{
    BoxFuture, Engine, ManagedNotifications, SessionId, SessionIncarnationId,
    SessionTurnPreparation,
};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use std::{
    num::NonZeroU64,
    task::{Context, Poll},
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
fn principal(id: &str) -> NoticePrincipal {
    NoticePrincipal {
        id: id.into(),
        generation: NonZeroU64::new(1).unwrap(),
    }
}
struct Fixture {
    engine: Engine,
    session: Session,
    notices: Arc<ManagedNotices>,
    parent: ParentNoticeContext,
    work: WorkNoticeRef,
}
impl Fixture {
    fn new() -> Self {
        Self::with_store(InMemorySessionStore::default())
    }
    fn with_store(store: impl machine_god_core::SessionStore) -> Self {
        let engine = Engine::builder()
            .provider(ScriptedModelProvider::new("test", []))
            .permission_handler(ScriptedPermissionHandler::new([]))
            .session_store(store)
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("parent").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            )
            .unwrap();
        let notices =
            Arc::new(ManagedNotices::new(NoticeLimits::default(), Arc::new(Clock)).unwrap());
        let parent = ParentNoticeContext::new(&session, principal("parent"), &notices);
        let work = notices
            .register_work(
                &WorkNoticeIdentity {
                    source: principal("child"),
                    work_id: "work".into(),
                    work_generation: NonZeroU64::new(1).unwrap(),
                },
                ManagedNotifications::default(),
                &NoticeRelationship {
                    generation: NonZeroU64::new(1).unwrap(),
                    parent: Some(principal("parent")),
                },
                0,
            )
            .unwrap();
        let PreparedNotice::Staged(stage) = notices
            .prepare_terminal(
                &work,
                NonZeroU64::new(1).unwrap(),
                NoticeTerminal::Completed,
                None,
            )
            .unwrap()
        else {
            panic!("terminal fixture is staged");
        };
        notices.confirm_durable(&stage).unwrap();
        Self {
            engine,
            session,
            notices,
            parent,
            work,
        }
    }
    fn record(&self) -> SessionRecord {
        self.session.record()
    }
    fn checkpoint(&self) -> NoticeCheckpoint {
        let record = self.record();
        checkpoint(&record)
    }
    fn prepare(&self) -> PreparedNoticeContext {
        self.parent
            .prepare(
                &self.session,
                &self.record(),
                self.checkpoint(),
                Some("skill"),
                Some("resource"),
            )
            .unwrap()
            .unwrap()
    }
    fn pending(&self) -> usize {
        self.notices
            .snapshot(&principal("parent"), 64, 65536)
            .unwrap()
            .entries()
            .len()
    }
}
fn checkpoint(record: &SessionRecord) -> NoticeCheckpoint {
    NoticeCheckpoint {
        session_id: record.id.clone(),
        incarnation_id: record.incarnation_id.clone(),
        expected_revision: record.revision,
        turn_sequence: record.next_turn_sequence,
        first_user_message: 0,
    }
}
fn preparation(prepared: &PreparedNoticeContext, record: &SessionRecord) -> SessionTurnPreparation {
    let mut metadata = record.metadata.clone();
    metadata.insert(
        NOTICE_CONTEXT_KEY.into(),
        prepared.checkpoint_value().unwrap(),
    );
    metadata.insert(NOTICE_OUTBOX_KEY.into(), prepared.outbox_value().unwrap());
    SessionTurnPreparation {
        expected_revision: record.revision,
        metadata: Some(metadata),
        context: None,
        user_context: Some(prepared.user_context()),
    }
}
#[test]
fn confirmed_actual_core_publication_consumes_original_before_caller_observes_turn() {
    let f = Fixture::new();
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    let future = prepared.publish_prompt(&f.session, "hello".into(), prep);
    assert_eq!(f.pending(), 1);
    let turn = block_on(future).unwrap();
    assert_eq!(f.pending(), 0);
    let saved = saved_context(&f.record(), Some((1, 0))).unwrap().unwrap();
    assert_eq!(saved.originals().len(), 1);
    assert_eq!(saved.identities(), &[saved.originals()[0].identity()]);
    drop(turn); // Any later native registration failure cannot undo the confirmed ACK.
}
#[test]
fn unpolled_future_and_guard_drop_leave_originals_and_release_slot() {
    let f = Fixture::new();
    drop(f.prepare());
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    drop(prepared.publish_prompt(&f.session, "hello".into(), prep));
    assert_eq!(f.pending(), 1);
    drop(f.prepare());
}
#[test]
fn foreign_actual_session_and_changed_preparation_fail_before_publication() {
    let f = Fixture::new();
    let foreign = Fixture::new();
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    assert!(matches!(
        block_on(prepared.publish_prompt(&foreign.session, "hello".into(), prep)),
        Err(NoticePublicationError::Context(
            NoticeContextError::ForeignSession
        ))
    ));
    let prepared = f.prepare();
    let mut prep = preparation(&prepared, &f.record());
    prep.user_context.as_mut().unwrap().text.push('x');
    assert!(matches!(
        block_on(prepared.publish_prompt(&f.session, "hello".into(), prep)),
        Err(NoticePublicationError::Context(
            NoticeContextError::InvalidCheckpoint
        ))
    ));
    assert_eq!(f.pending(), 1);
}
#[test]
fn close_or_parent_retirement_invalidates_prepared_batch() {
    let f = Fixture::new();
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    f.notices.close_work(&f.work).unwrap();
    assert!(matches!(
        block_on(prepared.publish_prompt(&f.session, "hello".into(), prep)),
        Err(NoticePublicationError::Context(NoticeContextError::Stale))
    ));
    let g = Fixture::new();
    let prepared = g.prepare();
    let prep = preparation(&prepared, &g.record());
    g.parent.retire();
    assert!(block_on(prepared.publish_prompt(&g.session, "hello".into(), prep)).is_err());
}
#[test]
fn combined_context_counts_every_separator_and_preserves_empty_semantics() {
    let text = "a".repeat(65530);
    assert_eq!(
        compose_user_context(Some(&text), Some("b"), Some("c"), 0)
            .unwrap()
            .unwrap()
            .text
            .len(),
        65536
    );
    assert!(compose_user_context(Some(&text), Some("bb"), Some("c"), 0).is_err());
    assert!(
        compose_user_context(Some(""), None, Some(""), 0)
            .unwrap()
            .is_none()
    );
    let f = Fixture::new();
    assert!(
        f.parent
            .prepare(
                &f.session,
                &f.record(),
                f.checkpoint(),
                Some(&"x".repeat(65536)),
                None
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(f.pending(), 1);
}
#[test]
fn saved_context_is_exact_inert_and_rejects_mismatched_originals() {
    let f = Fixture::new();
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    let turn = block_on(prepared.publish_prompt(&f.session, "hello".into(), prep)).unwrap();
    let mut record = f.record();
    assert!(saved_context(&record, Some((2, 0))).is_err());
    let saved = saved_context(&record, Some((1, 0))).unwrap().unwrap();
    assert!(!saved.text().is_empty());
    assert_eq!(f.pending(), 0);
    record.metadata.get_mut(NOTICE_CONTEXT_KEY).unwrap()["text"] = "forged".into();
    assert!(saved_context(&record, Some((1, 0))).is_err());
    drop(turn);
}

struct AmbiguousStore {
    store: InMemorySessionStore,
    first: std::sync::atomic::AtomicBool,
    error: bool,
    resume: Option<Arc<std::sync::atomic::AtomicBool>>,
}
impl machine_god_core::SessionStore for AmbiguousStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, machine_god_core::SessionStoreError>> {
        machine_god_core::SessionStore::load(&self.store, id)
    }
    fn save(
        &self,
        record: SessionRecord,
        revision: Option<machine_god_core::SessionRevision>,
    ) -> BoxFuture<'_, Result<machine_god_core::SessionRevision, machine_god_core::SessionStoreError>>
    {
        Box::pin(async move {
            let revision =
                machine_god_core::SessionStore::save(&self.store, record, revision).await?;
            if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                if self.error {
                    return Err(machine_god_core::SessionStoreError::new(
                        machine_god_core::SessionStoreErrorKind::Unavailable,
                        "ambiguous",
                        "committed before error",
                        false,
                    ));
                }
                futures_util::future::poll_fn(|_| {
                    if self
                        .resume
                        .as_ref()
                        .is_some_and(|ready| ready.load(std::sync::atomic::Ordering::SeqCst))
                    {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
            Ok(revision)
        })
    }
}
#[test]
fn committed_pending_drop_keeps_fence_until_exact_new_confirmed_continuation() {
    ambiguous_recovery(false);
}
#[test]
fn committed_error_readback_does_not_acknowledge() {
    ambiguous_recovery(true);
}
fn ambiguous_recovery(error: bool) {
    let f = Fixture::with_store(AmbiguousStore {
        store: InMemorySessionStore::default(),
        first: std::sync::atomic::AtomicBool::new(true),
        error,
        resume: None,
    });
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    let mut future = prepared.publish_prompt(&f.session, "hello".into(), prep);
    let result = future
        .as_mut()
        .poll(&mut Context::from_waker(noop_waker_ref()));
    if error {
        assert!(matches!(
            result,
            Poll::Ready(Err(NoticePublicationError::Core(_)))
        ));
    } else {
        assert!(result.is_pending());
    }
    if !error {
        assert!(matches!(
            f.parent
                .prepare(&f.session, &f.record(), f.checkpoint(), None, None),
            Err(NoticeContextError::Busy)
        ));
    }
    drop(future);
    assert_eq!(f.pending(), 1);
    assert!(matches!(
        f.parent
            .prepare(&f.session, &f.record(), f.checkpoint(), None, None),
        Err(NoticeContextError::Uncertain)
    ));
    let loaded = block_on(f.engine.load_session(f.session.id().clone()))
        .unwrap()
        .unwrap();
    assert!(loaded.witness().same_session(&f.session.witness()));
    let record = f.record();
    let saved = saved_context(&record, Some((1, 0))).unwrap().unwrap();
    assert_eq!(f.pending(), 1);
    assert!(matches!(
        block_on(f.parent.recover_delivery(&f.session)),
        Err(NoticePublicationError::Context(
            NoticeContextError::Uncertain
        ))
    ));
    assert!(f.parent.delivery().is_none());
    let recovery = f
        .parent
        .prepare_continuation(
            &f.session,
            &record,
            &saved,
            f.checkpoint(),
            Some("skill"),
            Some("resource"),
        )
        .unwrap();
    drop(recovery); // Unpolled continuation preserves the original uncertainty.
    assert!(matches!(
        f.parent
            .prepare(&f.session, &f.record(), f.checkpoint(), None, None),
        Err(NoticeContextError::Uncertain)
    ));
    let recovery = f
        .parent
        .prepare_continuation(
            &f.session,
            &record,
            &saved,
            f.checkpoint(),
            Some("skill"),
            Some("resource"),
        )
        .unwrap();
    let prep = preparation(&recovery, &record);
    let turn = block_on(recovery.publish_continuation(
        &f.session,
        machine_god_core::InferenceOptions::default(),
        prep,
    ))
    .unwrap();
    assert_eq!(f.pending(), 0);
    let saved = saved_context(&f.record(), Some((2, 0))).unwrap().unwrap();
    assert_eq!(saved.originals().len(), 1);
    drop(turn);
}
#[test]
fn new_arrivals_between_snapshot_and_confirmation_are_not_consumed() {
    let f = Fixture::new();
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    let work = f
        .notices
        .register_work(
            &WorkNoticeIdentity {
                source: principal("sibling"),
                work_id: "later".into(),
                work_generation: NonZeroU64::new(1).unwrap(),
            },
            ManagedNotifications::default(),
            &NoticeRelationship {
                generation: NonZeroU64::new(1).unwrap(),
                parent: Some(principal("parent")),
            },
            0,
        )
        .unwrap();
    let PreparedNotice::Staged(stage) = f
        .notices
        .prepare_terminal(
            &work,
            NonZeroU64::new(1).unwrap(),
            NoticeTerminal::Failed,
            None,
        )
        .unwrap()
    else {
        panic!("terminal fixture is staged");
    };
    f.notices.confirm_durable(&stage).unwrap();
    let turn = block_on(prepared.publish_prompt(&f.session, "hello".into(), prep)).unwrap();
    assert_eq!(f.pending(), 1);
    assert_eq!(
        f.notices
            .snapshot(&principal("parent"), 64, 65536)
            .unwrap()
            .entries()[0]
            .notice()
            .source
            .source
            .id,
        "sibling"
    );
    drop(turn);
}
#[test]
fn oversized_and_deep_saved_metadata_are_rejected_before_serde_traversal() {
    let f = Fixture::new();
    let mut record = f.record();
    record.metadata.insert(
        NOTICE_CONTEXT_KEY.into(),
        serde_json::Value::String("x".repeat(192 * 1024 + 1)),
    );
    assert!(matches!(
        saved_context(&record, Some((1, 0))),
        Err(NoticeContextError::ResourceLimit)
    ));
    let mut value = serde_json::Value::Null;
    for _ in 0..32 {
        value = serde_json::Value::Array(vec![value]);
    }
    record.metadata.insert(NOTICE_CONTEXT_KEY.into(), value);
    assert!(matches!(
        saved_context(&record, Some((1, 0))),
        Err(NoticeContextError::ResourceLimit)
    ));
}
#[test]
fn retirement_during_pending_save_cancels_turn_after_confirmed_receipt() {
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = Fixture::with_store(AmbiguousStore {
        store: InMemorySessionStore::default(),
        first: std::sync::atomic::AtomicBool::new(true),
        error: false,
        resume: Some(Arc::clone(&ready)),
    });
    let prepared = f.prepare();
    let prep = preparation(&prepared, &f.record());
    let mut future = prepared.publish_prompt(&f.session, "hello".into(), prep);
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(noop_waker_ref()))
            .is_pending()
    );
    f.parent.retire();
    ready.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(noop_waker_ref())),
        Poll::Ready(Err(NoticePublicationError::Context(
            NoticeContextError::Retired
        )))
    ));
    assert_eq!(f.pending(), 0);
    assert!(saved_context(&f.record(), Some((1, 0))).unwrap().is_some());
}
#[test]
fn owner_and_observer_do_not_keep_session_or_notices_runtime_alive() {
    let f = Fixture::new();
    let prepared = f.prepare();
    let witness = f.session.witness();
    let weak_notices = Arc::downgrade(&f.notices);
    drop(f);
    assert!(!witness.is_live());
    assert!(weak_notices.upgrade().is_none());
    drop(prepared);
}
