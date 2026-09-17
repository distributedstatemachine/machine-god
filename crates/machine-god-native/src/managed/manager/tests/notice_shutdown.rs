//! Shutdown retains a child's actual notice recipient until durable delivery settles.
use super::{Fixture, completed};
use crate::managed::{
    manager::{Active, ManagerBlock},
    notices::{
        NoticePrincipal, NoticeRelationship, NoticeTerminal, PreparedNotice, WorkNoticeIdentity,
    },
    prompt_context::{NOTICE_OUTBOX_KEY, ParentNoticeContext},
    store::{JournalMutation, JournalPublication, JournalRecord, JournalTranscript},
};
use futures_executor::block_on;
use futures_util::{StreamExt, task::AtomicWaker};
use machine_god_core::{
    BoxFuture, CancellationToken, ManagedNotifications, ManagedSubagentAuthority, SessionId,
    SessionRecord, SessionRevision, SessionStore, SessionStoreError, SessionStoreErrorKind,
};
use machine_god_testkit::InMemorySessionStore;
use std::{
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

#[derive(Default)]
struct Control {
    pause_clear: AtomicBool,
    error_clear: AtomicBool,
    error_next: AtomicBool,
    clear_started: AtomicUsize,
    wake: AtomicWaker,
}
impl Control {
    fn release(&self) {
        self.pause_clear.store(false, Ordering::Release);
        self.wake.wake();
    }
}
struct ControlledStore {
    inner: InMemorySessionStore,
    control: Arc<Control>,
}
impl SessionStore for ControlledStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.inner.load(id)
    }
    fn save(
        &self,
        record: SessionRecord,
        revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async move {
            let clearing = !record.metadata.contains_key(NOTICE_OUTBOX_KEY)
                && self
                    .inner
                    .record(&record.id)
                    .is_some_and(|old| old.metadata.contains_key(NOTICE_OUTBOX_KEY));
            if clearing {
                self.control.clear_started.fetch_add(1, Ordering::AcqRel);
                std::future::poll_fn(|cx| {
                    self.control.wake.register(cx.waker());
                    if self.control.pause_clear.load(Ordering::Acquire) {
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                })
                .await;
            }
            let revision = self.inner.save(record, revision).await?;
            if self.control.error_next.swap(false, Ordering::AcqRel)
                || (clearing && self.control.error_clear.swap(false, Ordering::AcqRel))
            {
                return Err(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "ambiguous_notice_store",
                    "committed before failed observation",
                    false,
                ));
            }
            Ok(revision)
        })
    }
}

fn fixture() -> (Fixture, InMemorySessionStore, Arc<Control>) {
    let store = InMemorySessionStore::default();
    let control = Arc::new(Control::default());
    let mut f = Fixture::with_store(
        vec![completed()],
        ControlledStore {
            inner: store.clone(),
            control: control.clone(),
        },
    );
    f.enable_child_notices();
    // Observe every real manager transition, rather than racing a worker's timing.
    f.manager.limits.work_per_poll = 1;
    for name in ["source", "recipient"] {
        assert!(
            f.command(serde_json::json!({
                "create": {"name": name, "mode": "persistent"}
            }))
            .ok
        );
    }
    f.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    let parent = f.actual_notice_session("child-2");
    let snapshot = block_on(f.journal.inspect("child-1".into())).unwrap();
    let JournalPublication::Confirmed(snapshot) = block_on(f.journal.mutate(
        snapshot,
        JournalMutation::Relationship {
            parent_id: Some("child-2".into()),
            parent_owner: Some(JournalTranscript {
                session_id: parent.id(),
                incarnation: parent.incarnation_id(),
            }),
            parent_generation: Some(1),
        },
    ))
    .unwrap() else {
        panic!("confirmed relationship");
    };
    let work = f
        .manager
        .notices
        .register_work(
            &WorkNoticeIdentity {
                source: NoticePrincipal {
                    id: "child-1".into(),
                    generation: nz(1),
                },
                work_id: "original-work".into(),
                work_generation: nz(1),
            },
            ManagedNotifications::default(),
            &NoticeRelationship {
                generation: nz(snapshot.head.revision),
                parent: Some(NoticePrincipal {
                    id: "child-2".into(),
                    generation: nz(1),
                }),
                parent_incarnation: Some(parent.incarnation_id()),
            },
            snapshot.head.notice_cursor,
        )
        .unwrap();
    let PreparedNotice::Staged(stage) = f
        .manager
        .notices
        .prepare_terminal(
            &work,
            nz(snapshot.head.next_sequence),
            NoticeTerminal::Completed,
            None,
        )
        .unwrap()
    else {
        panic!("original notice");
    };
    let JournalPublication::Confirmed(snapshot) = block_on(f.journal.mutate(
        *snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::Notice(stage.notice().clone())]),
    ))
    .unwrap() else {
        panic!("confirmed notice");
    };
    f.manager.children[0].snapshot = *snapshot;
    f.manager.notices.confirm_durable(&stage).unwrap();
    f.manager.notices.release_work(&work).unwrap();
    f.manager.replay_reset = true;
    (f, store, control)
}
fn nz(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

#[test]
fn full_notice_inbox_does_not_require_a_parent_prompt_to_shutdown() {
    use crate::managed::notices::{ManagedNotices, NoticeLimits};
    use machine_god_core::ManagedAgentState;
    use std::time::{Duration, Instant};

    let mut f = Fixture::new(vec![completed()]);
    f.manager.notices = Arc::new(
        ManagedNotices::new(
            NoticeLimits {
                records: 1,
                ..NoticeLimits::default()
            },
            f.manager.clock.clone(),
        )
        .unwrap(),
    );
    assert!(
        f.command(serde_json::json!({"create": {
            "name": "worker", "mode": "persistent", "prompt": "finish",
            "notifications": {"started": true}
        }}))
        .ok
    );
    f.drive(|f| {
        f.manager.children[0].snapshot.head.status == ManagedAgentState::Completed
            && f.manager.children[0].work.is_none()
            && f.manager.children[0].actual_settled
            && f.manager.active.is_none()
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut cx = Context::from_waker(Waker::noop());
    let stopped = loop {
        match f.manager.poll_shutdown(&mut cx, 101) {
            Poll::Ready(result) => {
                result.unwrap();
                break true;
            }
            Poll::Pending if Instant::now() >= deadline => break false,
            Poll::Pending => std::thread::yield_now(),
        }
    };
    // On the unfixed candidate, release only the original in-memory fixture
    // projection before asserting, so Fixture::drop cannot hang the test runner.
    if !stopped {
        let batch = f
            .manager
            .notices
            .snapshot(&NoticePrincipal { id: "parent".into(), generation: nz(1) }, 1, 64 * 1024)
            .unwrap();
        let tokens = batch.entries().iter().map(|entry| entry.token()).collect::<Vec<_>>();
        f.manager.notices.acknowledge(&batch, &tokens).unwrap();
        drop(batch);
        shutdown(&mut f);
    }
    assert!(stopped, "full notice inbox prevented actual manager shutdown");
}

fn context(f: &Fixture) -> Arc<ParentNoticeContext> {
    f.manager
        .children
        .iter()
        .find(|child| child.snapshot.head.id == "child-2")
        .unwrap()
        .prepared
        .notice_context
        .as_ref()
        .unwrap()
        .clone()
}
fn has_outbox(store: &InMemorySessionStore) -> bool {
    store
        .record(&SessionId::new("child-2").unwrap())
        .unwrap()
        .metadata
        .contains_key(NOTICE_OUTBOX_KEY)
}
fn send(f: &mut Fixture) {
    assert!(
        f.command(serde_json::json!({
            "message": {"send": {"id": "child-2", "content": "consume original notice"}}
        }))
        .ok
    );
}
fn shutdown(f: &mut Fixture) {
    block_on(std::future::poll_fn(|cx| f.manager.poll_shutdown(cx, 101))).unwrap();
}
fn retained(f: &mut Fixture) -> bool {
    let mut cx = Context::from_waker(Waker::noop());
    assert!(f.manager.poll_shutdown(&mut cx, 101).is_pending());
    f.manager.children.iter().any(|child| {
        child.snapshot.head.id == "child-2"
            && !child.closing
            && child.prepared.runtime.notice_cleanup_pending()
    })
}

#[test]
fn shutdown_ack_pending_clears_child_outbox_before_success() {
    let (mut f, store, _) = fixture();
    send(&mut f);
    f.drive(|f| context(f).delivery().is_some());
    assert!(has_outbox(&store));
    // The checkpoint confirmed, but the source ACK has not yet been polled.
    assert!(matches!(f.manager.active, Some(Active::Delivery(_))));
    shutdown(&mut f);
    assert!(
        !has_outbox(&store),
        "successful shutdown must clear the original outbox"
    );
    assert_eq!(
        f.factory.provider.requests().len(),
        0,
        "shutdown does not start the child provider"
    );
}

#[test]
fn shutdown_pending_clear_retains_child_capacity_and_exact_context() {
    let (mut f, store, control) = fixture();
    control.pause_clear.store(true, Ordering::Release);
    send(&mut f);
    f.drive(|_| control.clear_started.load(Ordering::Acquire) == 1);
    // Keep only a weak context observation: the fixture cannot hide early retirement.
    let weak = Arc::downgrade(&context(&f));
    f.manager.request_shutdown();
    f.drive(|f| f.manager.active.is_none() && f.manager.retiring.is_empty());
    let kept = retained(&mut f);
    let live = weak.strong_count() > 0;
    let charged = f.manager.progress().residents > 0;
    assert!(has_outbox(&store));
    control.release();
    shutdown(&mut f);
    assert!(
        kept && live && charged,
        "original child/context/capacity retired during clear"
    );
    assert!(!has_outbox(&store));
}

#[test]
fn shutdown_clear_committed_error_requires_original_retry() {
    let (mut f, store, control) = fixture();
    control.error_clear.store(true, Ordering::Release);
    send(&mut f);
    f.drive(|f| f.manager.retry.issue() == Some(ManagerBlock::Journal));
    // A durable readback showing absence is not the missing clear confirmation.
    assert!(!has_outbox(&store));
    let delivery = context(&f).delivery().unwrap();
    assert!(!delivery.is_cleared());
    f.child_session("child-2");
    f.manager.request_shutdown();
    let kept = retained(&mut f);
    f.manager.retry_reconciliation();
    shutdown(&mut f);
    assert!(
        kept,
        "uncertain clear must retain its original child until confirmed retry"
    );
    assert!(!has_outbox(&store));
    assert!(delivery.is_cleared());
}

#[test]
fn shutdown_dropped_clear_retains_original_until_explicit_repair() {
    let (mut f, store, control) = fixture();
    control.pause_clear.store(true, Ordering::Release);
    send(&mut f);
    f.drive(|_| control.clear_started.load(Ordering::Acquire) == 1);
    let parent = context(&f);
    let runtime = f
        .manager
        .children
        .iter()
        .find(|c| c.snapshot.head.id == "child-2")
        .unwrap()
        .prepared
        .runtime
        .clone();
    let slot = f
        .manager
        .parents
        .iter_mut()
        .find(|p| p.context.ptr_eq(&Arc::downgrade(&parent)))
        .unwrap();
    // Explicit observer abandonment must not turn a Clearing slot into success.
    drop(slot.clearing.take().unwrap());
    let delivery = parent.delivery().unwrap();
    assert!(!delivery.is_cleared());
    f.manager.request_shutdown();
    let kept = retained(&mut f);
    control.release();
    block_on(runtime.clear_notice_delivery(&delivery)).unwrap();
    assert!(delivery.is_cleared());
    shutdown(&mut f);
    assert!(kept, "dropped clear still owns exact cleanup custody");
    assert!(!has_outbox(&store));
}

fn saved_recipient(
    f: &Fixture,
) -> (
    Arc<ParentNoticeContext>,
    Arc<crate::NativeConversationRuntime>,
) {
    let session = f.actual_notice_session("child-2");
    let bound = context(f);
    // Prepare saved restart evidence through an actual native/core checkpoint.
    // Its temporary publisher is not the reconstructed recipient context.
    let publisher = Arc::new(ParentNoticeContext::new(
        &session,
        bound.principal().clone(),
        &f.manager.notices,
    ));
    let conversation = crate::NativeConversation::from_session(session.clone())
        .unwrap()
        .with_notice_context(&publisher)
        .unwrap();
    let turn = block_on(conversation.prompt("saved parent checkpoint".into(), 100)).unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    drop(conversation);
    drop(publisher);
    assert!(bound.delivery().is_none());
    assert!(
        !bound.has_pending_delivery(),
        "saved custody is independently checked in runtime metadata"
    );
    let runtime = f
        .manager
        .children
        .iter()
        .find(|c| c.snapshot.head.id == "child-2")
        .unwrap()
        .prepared
        .runtime
        .clone();
    (bound, runtime)
}

#[test]
fn shutdown_saved_outbox_and_uncertain_recovery_need_confirmed_repair() {
    let (mut f, store, control) = fixture();
    let (bound, runtime) = saved_recipient(&f);
    assert!(has_outbox(&store));
    f.manager.request_shutdown();
    let saved_kept = retained(&mut f);
    control.error_next.store(true, Ordering::Release);
    assert!(block_on(runtime.recover_notice_delivery()).is_err());
    assert!(bound.delivery().is_none());
    assert!(bound.has_pending_delivery());
    f.child_session("child-2");
    let uncertain_kept = retained(&mut f);
    block_on(runtime.recover_notice_delivery())
        .unwrap()
        .unwrap();
    shutdown(&mut f);
    assert!(
        saved_kept && uncertain_kept,
        "saved and uncertain custody must both retain the child"
    );
    assert!(!has_outbox(&store));
    assert_eq!(
        f.factory.provider.requests().len(),
        1,
        "repair adds no provider turn"
    );
}

#[test]
fn explicit_close_waits_for_unobservable_notice_custody() {
    let (mut f, store, control) = fixture();
    let (bound, runtime) = saved_recipient(&f);
    control.error_next.store(true, Ordering::Release);
    assert!(block_on(runtime.recover_notice_delivery()).is_err());
    f.child_session("child-2");
    assert!(bound.delivery().is_none() && bound.has_pending_delivery());
    let (_admission, invocation) = f.invocation(serde_json::json!({
        "lifecycle": {"id": "child-2", "action": "close"}
    }));
    let requester = f.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    assert!(
        response
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    f.drive(|f| {
        f.manager.active.is_none()
            && f.manager
                .children
                .iter()
                .find(|child| child.snapshot.head.id == "child-2")
                .is_none_or(|child| child.control_requested && !child.busy())
    });
    let kept = f.manager.children.iter().any(|child| {
        child.snapshot.head.id == "child-2"
            && !child.closing
            && child.snapshot.head.intent == Some(crate::managed::store::JournalIntent::Archive)
    });
    // On the old implementation this branch finishes teardown before reporting
    // the regression, avoiding a deliberately held fixture on assertion unwind.
    if !kept {
        shutdown(&mut f);
        panic!("close archived the child while its original delivery was uncertain");
    }
    assert!(
        response
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    block_on(runtime.recover_notice_delivery())
        .unwrap()
        .unwrap();
    let result = block_on(std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = response.as_mut().poll(cx) {
            return Poll::Ready(result.unwrap());
        }
        let progress = f.manager.poll_progress(cx, 100);
        assert!(!matches!(progress, Poll::Ready(Err(_))), "{progress:?}");
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }));
    assert!(result.ok);
    shutdown(&mut f);
    assert!(!has_outbox(&store));
}

#[test]
fn rejected_restored_admission_retains_outbox_until_clear() {
    use crate::managed::store::{JournalLimits, JournalWork};
    use machine_god_core::{ManagedFailureCode, ManagedQueueStatus};

    let (mut f, store, control) = fixture();
    // Seed actual saved metadata, then discard the idle resident to model a
    // nonresident transcript. No turn or worker is outstanding in this fixture.
    drop(saved_recipient(&f));
    let index = f
        .manager
        .children
        .iter()
        .position(|child| child.snapshot.head.id == "child-2")
        .unwrap();
    let child = f.manager.children.remove(index);
    assert!(!child.busy() && child.actual_settled);
    drop(child);
    let mut snapshot = block_on(f.journal.inspect("child-2".into())).unwrap();
    for index in 0..JournalLimits::default().queue_entries {
        let work = JournalWork {
            id: format!("retained-{index}"),
            source_id: snapshot.head.controller.session_id.to_string(),
            source_owner: snapshot.head.controller.clone(),
            content: "already accepted".into(),
            skills: Vec::new(),
            accepted_at_ms: 100,
            configuration: snapshot.head.configuration.clone(),
        };
        let JournalPublication::Confirmed(next) =
            block_on(f.journal.mutate(snapshot, JournalMutation::Enqueue(work))).unwrap()
        else {
            panic!("confirmed original work");
        };
        snapshot = *next;
    }
    let JournalPublication::Confirmed(snapshot) = block_on(f.journal.mutate(
        snapshot,
        JournalMutation::HeadState {
            work_id: "retained-0".into(),
            status: ManagedQueueStatus::Interrupted,
            failure: None,
        },
    ))
    .unwrap() else {
        panic!("confirmed interrupted head");
    };
    let accepted = snapshot.head.queue.clone();
    control.pause_clear.store(true, Ordering::Release);
    let prepared_before = f.factory.prepared.load(Ordering::Acquire);
    let result = f.command(serde_json::json!({
        "message": {"send": {"id": "child-2", "content": "overflow"}}
    }));
    assert_eq!(result.error_code, Some(ManagedFailureCode::ResourceLimit));
    assert_eq!(
        f.factory.prepared.load(Ordering::Acquire),
        prepared_before + 1
    );
    f.drive(|f| control.clear_started.load(Ordering::Acquire) > 0 || f.manager.retiring.is_empty());
    let retained = f.manager.retiring.iter().any(|retired| {
        retired.prepared.runtime.notice_cleanup_pending()
            && retired
                .prepared
                .notice_context
                .as_ref()
                .is_some_and(|context| {
                    context.principal().id == "child-2" && context.has_pending_delivery()
                })
    });
    let charged = f.manager.progress().residents > f.manager.children.len();
    let pending = f
        .manager
        .poll_shutdown(&mut Context::from_waker(Waker::noop()), 101)
        .is_pending();
    control.release();
    shutdown(&mut f);
    assert!(
        retained && charged && pending,
        "rejected restore lost original custody"
    );
    assert!(!has_outbox(&store));
    let snapshot = block_on(f.journal.inspect("child-2".into())).unwrap();
    assert_eq!(snapshot.head.queue, accepted);
    assert_eq!(f.factory.provider.requests().len(), 1);
}
