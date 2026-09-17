use super::*;
use crate::managed::{
    manager::tests::Fixture,
    notices::{NoticeEvent, NoticePrincipal, NoticeTarget, NoticeTerminal, WorkNoticeIdentity},
    store::JournalPublication,
};
use futures_executor::block_on;
use machine_god_core::{ManagedHistoryItem, ManagedHistoryKind, Session};
use std::{future::Future, num::NonZeroU64};

fn append(
    fixture: &mut Fixture,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
) -> JournalSnapshot {
    let JournalPublication::Confirmed(snapshot) =
        block_on(fixture.journal.mutate(snapshot, mutation)).unwrap()
    else {
        panic!("confirmed test mutation");
    };
    fixture.manager.children[0].snapshot = *snapshot.clone();
    *snapshot
}

fn history_fixture() -> (
    Fixture,
    Session,
    Arc<ParentNoticeContext>,
    JournalSnapshot,
    ManagedNotice,
) {
    let nz = |n| NonZeroU64::new(n).unwrap();
    let mut fixture = Fixture::new(vec![]);
    assert!(
        fixture
            .command(serde_json::json!({
                "create":{"name":"source","mode":"persistent"}
            }))
            .ok
    );
    fixture.drive(|f| f.manager.active.is_none());
    let session = fixture.notice_session();
    let parent = NoticePrincipal {
        id: "notice-parent".into(),
        generation: nz(1),
    };
    let snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
    let mut snapshot = append(
        &mut fixture,
        snapshot,
        JournalMutation::Relationship {
            parent_id: Some(parent.id.clone()),
            parent_generation: Some(1),
            parent_owner: Some(JournalTranscript {
                session_id: session.id(),
                incarnation: session.incarnation_id(),
            }),
        },
    );
    let relationship = snapshot.head.revision;
    // The original relationship is older than three bounded validation pages.
    for _ in 0..4 {
        snapshot = append(
            &mut fixture,
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
        );
    }
    let original = ManagedNotice {
        source: WorkNoticeIdentity {
            source: NoticePrincipal {
                id: "child-1".into(),
                generation: nz(1),
            },
            work_id: "original-work".into(),
            work_generation: nz(1),
        },
        source_sequence: nz(snapshot.head.next_sequence),
        target: NoticeTarget {
            parent: parent.clone(),
            parent_incarnation: session.incarnation_id(),
            relationship_generation: nz(relationship),
        },
        event: NoticeEvent::Terminal {
            outcome: NoticeTerminal::Completed,
        },
        history: None,
    };
    snapshot = append(
        &mut fixture,
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::Notice(original.clone())]),
    );
    let context = Arc::new(ParentNoticeContext::new(
        &session,
        parent,
        &fixture.manager.notices,
    ));
    // The context intentionally holds only a weak witness. Its actual session
    // owner must remain alive while the caller exercises replay validation.
    (fixture, session, context, snapshot, original)
}

#[test]
fn historical_notice_validation_yields_before_scanning_an_entire_history() {
    let (fixture, session, context, snapshot, original) = history_fixture();
    let transcript = JournalTranscript {
        session_id: session.id(),
        incarnation: session.incarnation_id(),
    };
    assert!(context.matches_transcript(&transcript));
    let targets = vec![Arc::downgrade(&context)];
    let mut replay = Replay {
        source: Some((snapshot, None)),
        ..Replay::default()
    };
    let mut repaired = None;
    block_on(step(
        &fixture.journal,
        &fixture.manager.retry,
        &mut replay,
        &targets,
        &mut repaired,
    ))
    .unwrap();
    assert!(
        replay.pending.is_none(),
        "a single replay admission scanned the entire historical relationship"
    );
    let mut admissions = 1;
    while replay.pending.is_none() && admissions < 10 {
        block_on(step(
            &fixture.journal,
            &fixture.manager.retry,
            &mut replay,
            &targets,
            &mut repaired,
        ))
        .unwrap();
        admissions += 1;
    }
    assert!(
        admissions >= 5,
        "validation did not yield between bounded pages"
    );
    assert_eq!(
        replay.pending.as_ref().map(|(notice, _)| notice),
        Some(&original)
    );
    assert!(repaired.is_none());
    assert!(fixture.factory.provider.requests().is_empty());
    drop(session);
    assert!(!context.matches_transcript(&transcript));
}

#[test]
fn validation_rejects_a_changed_source_snapshot_between_admissions() {
    let (mut fixture, _session, context, snapshot, _) = history_fixture();
    let targets = vec![Arc::downgrade(&context)];
    let mut replay = Replay {
        source: Some((snapshot.clone(), None)),
        ..Replay::default()
    };
    let mut repaired = None;
    block_on(step(
        &fixture.journal,
        &fixture.manager.retry,
        &mut replay,
        &targets,
        &mut repaired,
    ))
    .unwrap();
    assert!(replay.validation.is_some());
    append(
        &mut fixture,
        snapshot,
        JournalMutation::Relationship {
            parent_id: None,
            parent_owner: None,
            parent_generation: None,
        },
    );
    assert!(
        block_on(step(
            &fixture.journal,
            &fixture.manager.retry,
            &mut replay,
            &targets,
            &mut repaired
        ))
        .is_err()
    );
    assert!(replay.pending.is_none());
}

#[test]
fn failed_replay_read_does_not_hold_the_command_or_shutdown_lane() {
    let mut fixture = Fixture::new(vec![]);
    fixture.manager.replay_reset = false;
    fixture.manager.finish_replay(Outcome {
        replay: Replay::default(),
        snapshot: None,
        error: true,
    });
    assert!(fixture.manager.active.is_none());
    let mut cx = Context::from_waker(std::task::Waker::noop());
    assert!(!fixture.manager.begin_replay(&mut cx));
    assert!(fixture.manager.replay.retry.is_some());
    assert!(
        fixture
            .command(serde_json::json!({
                "create":{"name":"unblocked","mode":"persistent"}
            }))
            .ok
    );
    assert!(fixture.factory.provider.requests().is_empty());
    // Fixture drop performs actual manager/resource shutdown without an
    // explicit retry or a parent prompt to unblock read-only validation.
}

#[test]
fn dropping_a_superseded_read_retry_clears_only_its_own_blocked_status() {
    let gate = durability::RetryGate::default();
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let mut first = Box::pin(gate.blocked(ManagerBlock::Journal));
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert_eq!(gate.issue(), Some(ManagerBlock::Journal));
    drop(first);
    assert!(gate.issue().is_none());
    let mut first = Box::pin(gate.blocked(ManagerBlock::Journal));
    assert!(first.as_mut().poll(&mut cx).is_pending());
    let mut second = Box::pin(gate.blocked(ManagerBlock::Journal));
    assert!(second.as_mut().poll(&mut cx).is_pending());
    drop(first);
    assert_eq!(gate.issue(), Some(ManagerBlock::Journal));
    drop(second);
    assert!(gate.issue().is_none());
}
