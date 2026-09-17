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

#[test]
#[allow(clippy::too_many_lines)] // Actual owner release/reopen must not become a recovery publication.
fn owner_reopen_replay_skips_noop_recovery_for_archived_and_quiescent_sources() {
    use crate::NativeOwnedWorkerScope;
    use crate::managed::store::{JournalCreate, JournalIntent, JournalLimits};
    use machine_god_core::{
        ManagedAgentMode, ManagedConfiguration, ManagedNotifications, ManagedPermissionMode,
        SessionId, SessionIncarnationId,
    };
    use std::{
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(1);
    for archived in [false, true] {
        let path = std::env::temp_dir().join(format!(
            "mg-replay-owner-reopen-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let workers = NativeOwnedWorkerScope::new();
        let open = || {
            block_on(ManagedJournal::open(
                std::fs::File::open(&path).unwrap().into(),
                workers.clone(),
                JournalLimits::default(),
            ))
            .unwrap()
        };
        let journal = open();
        let transcript = JournalTranscript {
            session_id: SessionId::new("source").unwrap(),
            incarnation: SessionIncarnationId::new("source-life").unwrap(),
        };
        let JournalPublication::Confirmed(mut snapshot) = block_on(journal.create(JournalCreate {
            id: "source".into(),
            mode: ManagedAgentMode::Persistent,
            configuration: ManagedConfiguration {
                name: "source".into(),
                model: None,
                effort: None,
                permission_mode: ManagedPermissionMode::Ask,
                notifications: ManagedNotifications::default(),
            },
            transcript: transcript.clone(),
            controller: transcript,
            parent_id: None,
            parent_owner: None,
            parent_generation: None,
            initial_work: None,
        }))
        .unwrap() else {
            panic!("confirmed creation")
        };
        if archived {
            for mutation in [
                JournalMutation::Intent(JournalIntent::Archive),
                JournalMutation::Archive,
            ] {
                let JournalPublication::Confirmed(next) =
                    block_on(journal.mutate(*snapshot, mutation)).unwrap()
                else {
                    panic!("confirmed archive lifecycle")
                };
                snapshot = next;
            }
            assert_eq!(snapshot.head.cleanup_entries, 0);
        }
        let before = snapshot.head.clone();
        drop(snapshot);
        drop(journal);
        let journal = open();
        assert!(
            block_on(journal.inspect("source".into()))
                .unwrap()
                .recovery_required()
        );
        let gate = Arc::new(durability::RetryGate::default());
        let mut replay = Replay::default();
        let mut repaired = None;
        for _ in 0..16 {
            block_on(step(&journal, &gate, &mut replay, &[], &mut repaired)).unwrap();
            assert!(
                repaired.is_none(),
                "quiescent replay minted a generic recovery page"
            );
            assert!(replay.pending.is_none());
            assert!(replay.validation.is_none());
            if replay.done {
                break;
            }
        }
        assert!(replay.done);
        let after = block_on(journal.inspect("source".into())).unwrap();
        assert_eq!(after.head, before);
        assert!(
            after.recovery_required(),
            "read-only replay must not transfer owner epoch"
        );
        assert!(gate.issue().is_none());
        drop(after);
        drop(journal);
        workers.close();
        block_on(workers.completion().wait());
        std::fs::remove_dir_all(path).unwrap();
    }
}
