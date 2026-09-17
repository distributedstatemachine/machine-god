use super::{Fixture, completed};
use crate::NativeConversation;
use crate::managed::{
    manager::{ManagedRuntimeError, delivery, durability},
    notices::{
        ManagedNotice, NoticePrincipal, NoticeRelationship, NoticeTerminal, PreparedNotice,
        WorkNoticeIdentity,
    },
    prompt_context::ParentNoticeContext,
    store::{
        JournalMutation, JournalPublication, JournalRecord, JournalSnapshot, JournalTranscript,
    },
};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{ManagedNotifications, Session, SessionIncarnationId};
use std::{num::NonZeroU64, sync::Arc};

fn confirmed(publication: JournalPublication) -> JournalSnapshot {
    let JournalPublication::Confirmed(snapshot) = publication else {
        panic!("fixture publication must confirm");
    };
    *snapshot
}

pub(super) fn original(fixture: &mut Fixture, parent: &Session) -> ManagedNotice {
    original_for_target(fixture, parent, 1, parent.incarnation_id())
}

// Deliberately accepts a mismatched envelope target for historical validation tests.
fn original_for_target(
    fixture: &mut Fixture,
    parent: &Session,
    generation: u64,
    incarnation: SessionIncarnationId,
) -> ManagedNotice {
    assert!(
        fixture
            .command(serde_json::json!({
                "create": {"name": "source", "mode": "persistent"}
            }))
            .ok
    );
    fixture.drive(|f| f.manager.active.is_none());
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let snapshot = confirmed(
        block_on(fixture.journal.mutate(
            snapshot,
            JournalMutation::Relationship {
                parent_id: Some(parent.id().to_string()),
                parent_owner: Some(JournalTranscript {
                    session_id: parent.id(),
                    incarnation: parent.incarnation_id(),
                }),
                parent_generation: Some(1),
            },
        ))
        .unwrap(),
    );
    let work = fixture
        .manager
        .notices
        .register_work(
            &WorkNoticeIdentity {
                source: NoticePrincipal {
                    id: "child-1".into(),
                    generation: NonZeroU64::new(1).unwrap(),
                },
                work_id: "original-work".into(),
                work_generation: NonZeroU64::new(1).unwrap(),
            },
            ManagedNotifications::default(),
            &NoticeRelationship {
                parent_incarnation: Some(incarnation),
                generation: NonZeroU64::new(snapshot.head.revision).unwrap(),
                parent: Some(NoticePrincipal {
                    id: parent.id().to_string(),
                    generation: NonZeroU64::new(generation).unwrap(),
                }),
            },
            snapshot.head.notice_cursor,
        )
        .unwrap();
    let PreparedNotice::Staged(stage) = fixture
        .manager
        .notices
        .prepare_terminal(
            &work,
            NonZeroU64::new(snapshot.head.next_sequence).unwrap(),
            NoticeTerminal::Completed,
            None,
        )
        .unwrap()
    else {
        panic!("original terminal notice");
    };
    let original = stage.notice().clone();
    let snapshot = confirmed(
        block_on(fixture.journal.mutate(
            snapshot,
            JournalMutation::AppendHistory(vec![JournalRecord::Notice(original.clone())]),
        ))
        .unwrap(),
    );
    fixture.manager.children[0].snapshot = snapshot;
    fixture.manager.notices.confirm_durable(&stage).unwrap();
    original
}

pub(super) fn deliver(
    fixture: &Fixture,
    session: &Session,
) -> (Arc<ParentNoticeContext>, NativeConversation) {
    deliver_generation(fixture, session, 1)
}

fn deliver_generation(
    fixture: &Fixture,
    session: &Session,
    generation: u64,
) -> (Arc<ParentNoticeContext>, NativeConversation) {
    let context = Arc::new(ParentNoticeContext::new(
        session,
        NoticePrincipal {
            id: session.id().to_string(),
            generation: NonZeroU64::new(generation).unwrap(),
        },
        &fixture.manager.notices,
    ));
    let conversation = NativeConversation::from_session(session.clone())
        .unwrap()
        .with_notice_context(&context)
        .unwrap();
    let turn = block_on(conversation.prompt("explicit next parent input".into(), 102)).unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    assert!(context.delivery().is_some());
    (context, conversation)
}

#[test]
fn actual_parent_checkpoint_is_source_acknowledged_before_outbox_clear() {
    let mut fixture = Fixture::new(vec![completed()]);
    let parent = fixture.notice_session();
    let original = original(&mut fixture, &parent);
    let (context, conversation) = deliver(&fixture, &parent);
    let delivered = context.delivery().unwrap();
    assert_eq!(delivered.originals(), std::slice::from_ref(&original));
    assert!(block_on(conversation.clear_notice_delivery(&delivered)).is_err());
    fixture.manager.register_parent_context(&context).unwrap();
    fixture.drive(|f| {
        f.manager.active.is_none()
            && f.manager
                .parents
                .iter()
                .any(|parent| parent.completed.is_some())
    });
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let history = block_on(fixture.journal.history(snapshot, None, 100)).unwrap();
    assert!(
        history
            .records
            .contains(&JournalRecord::NoticeAcknowledged {
                identity: original.identity(),
                target: original.target,
                checkpoint: delivered.checkpoint().clone(),
            })
    );
    block_on(conversation.clear_notice_delivery(&delivered)).unwrap();
    assert!(context.delivery().is_none());
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

fn retain_long_history(fixture: &mut Fixture) {
    use machine_god_core::{ManagedHistoryItem, ManagedHistoryKind};
    let mut snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    for _ in 0..4 {
        snapshot = confirmed(
            block_on(
                fixture.journal.mutate(
                    snapshot,
                    JournalMutation::AppendHistory(
                        (0..90)
                            .map(|_| {
                                JournalRecord::History(ManagedHistoryItem {
                                    kind: ManagedHistoryKind::Conversation,
                                    work_id: None,
                                    user: None,
                                    assistant: Some("retained history".into()),
                                    user_truncated: false,
                                    assistant_truncated: false,
                                })
                            })
                            .collect(),
                    ),
                ),
            )
            .unwrap(),
        );
    }
    fixture.manager.children[0].snapshot = snapshot;
}

fn advance_delivery(fixture: &mut Fixture) {
    use crate::managed::manager::Active;
    assert!(
        fixture
            .manager
            .begin_delivery(&mut std::task::Context::from_waker(std::task::Waker::noop()))
    );
    let Some(Active::Delivery(future)) = fixture.manager.active.take() else {
        panic!("delivery admission");
    };
    fixture.manager.finish_delivery(block_on(future)).unwrap();
}

#[test]
fn delivery_validation_releases_the_journal_between_history_pages() {
    let mut fixture = Fixture::new(vec![completed()]);
    let parent = fixture.notice_session();
    original(&mut fixture, &parent);
    retain_long_history(&mut fixture);
    let (context, conversation) = deliver(&fixture, &parent);
    let delivered = context.delivery().unwrap();
    fixture.manager.register_parent_context(&context).unwrap();
    advance_delivery(&mut fixture);
    let yielded = fixture.manager.parents[0].completed.is_none();
    // New commands can acquire the journal between validation admissions.
    let sibling = fixture.command(serde_json::json!({
        "create": {"name": "sibling", "mode": "persistent"}
    }));
    fixture.drive(|f| f.manager.active.is_none() && f.manager.parents[0].completed.is_some());
    block_on(conversation.clear_notice_delivery(&delivered)).unwrap();
    assert!(sibling.ok);
    assert!(
        yielded,
        "one delivery admission scanned the whole retained history"
    );
    assert_eq!(fixture.factory.provider.requests().len(), 1);
}

#[test]
fn delivery_restarts_changed_source_without_acknowledging_a_stale_frontier() {
    let mut fixture = Fixture::new(vec![completed()]);
    let parent = fixture.notice_session();
    let original = original(&mut fixture, &parent);
    retain_long_history(&mut fixture);
    let (context, conversation) = deliver(&fixture, &parent);
    let delivered = context.delivery().unwrap();
    fixture.manager.register_parent_context(&context).unwrap();
    advance_delivery(&mut fixture);
    assert!(fixture.manager.parents[0].pending.is_some());
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let changed = confirmed(
        block_on(fixture.journal.mutate(
            snapshot,
            JournalMutation::Relationship {
                parent_id: None,
                parent_owner: None,
                parent_generation: None,
            },
        ))
        .unwrap(),
    );
    fixture.manager.children[0].snapshot = changed.clone();
    advance_delivery(&mut fixture);
    let after = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(
        after.head, changed.head,
        "stale validation must not publish"
    );
    assert!(fixture.manager.parents[0].pending.is_some());
    assert!(fixture.manager.parents[0].completed.is_none());
    fixture.drive(|f| f.manager.active.is_none() && f.manager.parents[0].completed.is_some());
    block_on(conversation.clear_notice_delivery(&delivered)).unwrap();
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let history = block_on(fixture.journal.history(snapshot, None, 100)).unwrap();
    assert!(
        history
            .records
            .contains(&JournalRecord::NoticeAcknowledged {
                identity: original.identity(),
                target: original.target,
                checkpoint: delivered.checkpoint().clone(),
            })
    );
}

#[test]
fn registering_a_parent_cannot_prune_an_in_flight_delivery_frontier() {
    use crate::managed::manager::Active;
    let mut fixture = Fixture::new(vec![completed()]);
    let parent = fixture.notice_session();
    original(&mut fixture, &parent);
    retain_long_history(&mut fixture);
    assert!(
        fixture
            .command(serde_json::json!({
                "create": {"name": "sibling", "mode": "persistent"}
            }))
            .ok
    );
    fixture.drive(|f| f.manager.active.is_none());
    let (context, conversation) = deliver(&fixture, &parent);
    let delivered = context.delivery().unwrap();
    let weak = Arc::downgrade(&context);
    fixture.manager.register_parent_context(&context).unwrap();
    assert!(
        fixture
            .manager
            .begin_delivery(&mut std::task::Context::from_waker(std::task::Waker::noop()))
    );
    drop(context);
    assert_eq!(weak.strong_count(), 0);
    let sibling = fixture.child_session("child-2");
    let other = Arc::new(ParentNoticeContext::new(
        &sibling,
        NoticePrincipal {
            id: "child-2".into(),
            generation: NonZeroU64::new(1).unwrap(),
        },
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&other).unwrap();
    assert!(
        fixture
            .manager
            .parents
            .iter()
            .any(|entry| entry.context.ptr_eq(&weak))
    );
    let Some(Active::Delivery(future)) = fixture.manager.active.take() else {
        panic!("delivery admission");
    };
    fixture.manager.finish_delivery(block_on(future)).unwrap();
    assert!(
        fixture
            .manager
            .parents
            .iter()
            .any(|entry| entry.context.ptr_eq(&weak) && entry.pending.is_some())
    );
    fixture.drive(|f| {
        f.manager.active.is_none()
            && f.manager
                .parents
                .iter()
                .all(|entry| entry.pending.is_none())
    });
    // Source repair does not fabricate a clear receipt for a retired observer.
    assert!(!delivered.is_cleared());
    assert!(block_on(conversation.clear_notice_delivery(&delivered)).is_err());
}

#[test]
fn external_clear_wakes_shutdown_and_survives_context_drop_before_manager_poll() {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        task::{Context, Wake, Waker},
    };
    struct WakeCount(AtomicUsize);
    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    let mut fixture = Fixture::new(vec![completed()]);
    let session = fixture.notice_session();
    original(&mut fixture, &session);
    let (context, conversation) = deliver(&fixture, &session);
    let delivered = context.delivery().unwrap();
    fixture.manager.register_parent_context(&context).unwrap();
    fixture.drive(|f| f.manager.active.is_none() && f.manager.parents[0].clear.is_some());
    assert!(!delivered.is_cleared());
    let notifications = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(notifications.clone());
    assert!(
        fixture
            .manager
            .poll_shutdown(&mut Context::from_waker(&waker), 103)
            .is_pending()
    );
    let before = notifications.0.load(Ordering::Acquire);
    block_on(conversation.clear_notice_delivery(&delivered)).unwrap();
    assert!(delivered.is_cleared());
    assert!(notifications.0.load(Ordering::Acquire) > before);
    assert!(
        !session
            .record()
            .metadata
            .contains_key(crate::managed::prompt_context::NOTICE_OUTBOX_KEY)
    );
    let weak = Arc::downgrade(&context);
    drop(context);
    assert_eq!(weak.strong_count(), 0);
    // No manager poll occurred between the actual save and owner retirement.
    assert!(fixture.manager.parents[0].clear.is_some());
    assert!(
        fixture
            .manager
            .poll_delivery_clears(&mut Context::from_waker(&waker))
    );
    assert!(fixture.manager.parents[0].clear.is_none());
    block_on(std::future::poll_fn(|cx| {
        fixture.manager.poll_shutdown(cx, 103)
    }))
    .unwrap();
}

#[test]
fn dropped_external_context_does_not_confirm_clear_receipt() {
    let mut fixture = Fixture::new(vec![completed()]);
    let session = fixture.notice_session();
    original(&mut fixture, &session);
    let (context, conversation) = deliver(&fixture, &session);
    let delivered = context.delivery().unwrap();
    // No source ACK exists; an attempted clear and subsequent context drop are
    // neither a successful save nor permission to settle this original receipt.
    assert!(block_on(conversation.clear_notice_delivery(&delivered)).is_err());
    drop(context);
    assert!(!delivered.is_cleared());
    assert!(
        session
            .record()
            .metadata
            .contains_key(crate::managed::prompt_context::NOTICE_OUTBOX_KEY)
    );
}

#[test]
fn actual_checkpoint_from_another_incarnation_cannot_acknowledge_the_original() {
    let mut fixture = Fixture::new(vec![completed()]);
    let parent = fixture.notice_session();
    let original = original_for_target(
        &mut fixture,
        &parent,
        1,
        SessionIncarnationId::new("foreign-life").unwrap(),
    );
    // An actual foreign checkpoint is still not the original recipient's checkpoint.
    let engine = machine_god_core::Engine::builder()
        .provider(machine_god_testkit::ScriptedModelProvider::new(
            "foreign",
            [completed()],
        ))
        .permission_handler(machine_god_testkit::ScriptedPermissionHandler::new([]))
        .session_store(machine_god_testkit::InMemorySessionStore::default())
        .build()
        .unwrap();
    let foreign = engine
        .create_session(
            parent.id(),
            SessionIncarnationId::new("foreign-life").unwrap(),
        )
        .unwrap();
    let (context, conversation) = deliver(&fixture, &foreign);
    let delivered = context.delivery().unwrap();
    assert_eq!(delivered.originals(), &[original]);
    let before = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let mut snapshots = Vec::new();
    assert!(matches!(
        block_on(delivery::reconcile_sources(
            &fixture.journal,
            &Arc::new(durability::RetryGate::default()),
            &delivered,
            &mut snapshots,
        )),
        Err(ManagedRuntimeError::Invalid)
    ));
    assert!(snapshots.is_empty());
    let after = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    assert_eq!(before.head, after.head);
    assert!(block_on(conversation.clear_notice_delivery(&delivered)).is_err());
    assert!(fixture.factory.provider.requests().is_empty());
}

#[test]
fn wrong_historical_parent_generation_cannot_replay_or_acknowledge() {
    let mut fixture = Fixture::new(vec![completed()]);
    let parent = fixture.notice_session();
    let original = original_for_target(&mut fixture, &parent, 2, parent.incarnation_id());
    let (context, _conversation) = deliver_generation(&fixture, &parent, 2);
    let delivered = context.delivery().unwrap();
    let mut snapshots = Vec::new();
    assert!(matches!(
        block_on(delivery::reconcile_sources(
            &fixture.journal,
            &Arc::new(durability::RetryGate::default()),
            &delivered,
            &mut snapshots,
        )),
        Err(ManagedRuntimeError::Invalid)
    ));
    assert!(snapshots.is_empty());
    fixture.restart_manager();
    let context = Arc::new(ParentNoticeContext::new(
        &parent,
        original.target.parent.clone(),
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&context).unwrap();
    fixture.drive(|f| f.manager.replay.done && f.manager.active.is_none());
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&original.target.parent, 64, 64 * 1024)
            .unwrap()
            .entries()
            .is_empty()
    );
}

#[test]
fn direct_inbox_excludes_reused_parent_pair_with_foreign_incarnation() {
    let mut fixture = Fixture::new(vec![]);
    let parent = fixture.notice_session();
    let original = original(&mut fixture, &parent);
    let engine = machine_god_core::Engine::builder()
        .provider(machine_god_testkit::ScriptedModelProvider::new(
            "foreign",
            [completed()],
        ))
        .permission_handler(machine_god_testkit::ScriptedPermissionHandler::new([]))
        .session_store(machine_god_testkit::InMemorySessionStore::default())
        .build()
        .unwrap();
    let foreign = engine
        .create_session(
            parent.id(),
            SessionIncarnationId::new("foreign-life").unwrap(),
        )
        .unwrap();
    let context = Arc::new(ParentNoticeContext::new(
        &foreign,
        original.target.parent.clone(),
        &fixture.manager.notices,
    ));
    let conversation = NativeConversation::from_session(foreign)
        .unwrap()
        .with_notice_context(&context)
        .unwrap();
    let turn = block_on(conversation.prompt("explicit foreign input".into(), 102)).unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    assert!(context.delivery().is_none());
    let batch = fixture
        .manager
        .notices
        .snapshot_for_parent(
            &original.target.parent,
            &parent.incarnation_id(),
            64,
            64 * 1024,
        )
        .unwrap();
    assert_eq!(batch.entries()[0].notice(), &original);
}
