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
    fixture
        .manager
        .children
        .iter_mut()
        .find(|child| child.snapshot.head.id == snapshot.head.id)
        .unwrap()
        .snapshot = *snapshot.clone();
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
        replay.pending.as_ref().map(|pending| &pending.original),
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

// Retain a fully validated original behind an actual full notice inbox.
fn capacity_fixture() -> (
    Fixture,
    Session,
    Arc<ParentNoticeContext>,
    ManagedNotice,
    ManagedNotice,
) {
    use crate::managed::notices::{ManagedNotices, NoticeLimits};
    let (mut fixture, session, _, snapshot, original) = history_fixture();
    // Inject the smaller registry with a matching deadline subscription. The
    // old weak subscription cannot observe this replacement allocation.
    fixture.manager.deadline = None;
    fixture.manager.notices = Arc::new(
        ManagedNotices::new(
            NoticeLimits {
                records: 1,
                ..NoticeLimits::default()
            },
            fixture.manager.clock.clone(),
        )
        .unwrap(),
    );
    let mut blocker = original.clone();
    blocker.source.work_id = "capacity-blocker".into();
    blocker.source_sequence = NonZeroU64::new(snapshot.head.next_sequence).unwrap();
    let snapshot = append(
        &mut fixture,
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::Notice(blocker.clone())]),
    );
    let work = fixture
        .manager
        .notices
        .register_work(
            &blocker.source,
            ManagedNotifications::default(),
            &NoticeRelationship {
                generation: blocker.target.relationship_generation,
                parent: Some(blocker.target.parent.clone()),
                parent_incarnation: Some(blocker.target.parent_incarnation.clone()),
            },
            blocker.source_sequence.get(),
        )
        .unwrap();
    fixture
        .manager
        .notices
        .restore_notice(&work, &blocker)
        .unwrap();
    fixture.manager.notices.stop_work(&work).unwrap();
    fixture.manager.notices.release_work(&work).unwrap();
    let context = Arc::new(ParentNoticeContext::new(
        &session,
        original.target.parent.clone(),
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&context).unwrap();
    let mut replay = Replay {
        catalog_done: true,
        validation: Some(Validation {
            snapshot,
            original: original.clone(),
            cursor: None,
            original_seen: false,
            parent_checked: false,
            parent: None,
        }),
        ..Replay::default()
    };
    for _ in 0..10 {
        block_on(step(
            &fixture.journal,
            &fixture.manager.retry,
            &mut replay,
            &[Arc::downgrade(&context)],
            &mut None,
        ))
        .unwrap();
        if replay.pending.is_some() {
            break;
        }
    }
    assert!(replay.pending.is_some());
    fixture.manager.replay = replay;
    fixture.manager.replay_reset = false;
    assert!(
        !fixture
            .manager
            .begin_replay(&mut Context::from_waker(std::task::Waker::noop()))
    );
    assert!(fixture.manager.active.is_none());
    assert!(fixture.manager.replay.pending.as_ref().unwrap().fresh);
    (fixture, session, context, original, blocker)
}

#[test]
fn capacity_delayed_original_revalidates_after_ack_or_archive() {
    use crate::managed::{prompt_context::NoticeCheckpoint, store::JournalIntent};
    for archive in [false, true] {
        let (mut fixture, session, _context, original, blocker) = capacity_fixture();
        let mut snapshot = block_on(fixture.journal.inspect("child-1".into())).unwrap();
        if archive {
            for mutation in [
                JournalMutation::Intent(JournalIntent::Archive),
                JournalMutation::Archive,
            ] {
                snapshot = append(&mut fixture, snapshot, mutation);
            }
        } else {
            let record = session.record();
            append(
                &mut fixture,
                snapshot,
                JournalMutation::AppendHistory(vec![JournalRecord::NoticeAcknowledged {
                    identity: original.identity(),
                    target: original.target.clone(),
                    checkpoint: NoticeCheckpoint {
                        session_id: record.id.clone(),
                        incarnation_id: record.incarnation_id.clone(),
                        expected_revision: record.revision,
                        turn_sequence: 1,
                        first_user_message: 0,
                    },
                }]),
            );
        }
        fixture.manager.replay_reset = true;
        fixture
            .manager
            .notices
            .acknowledge_recovered(&[blocker])
            .unwrap();
        assert!(
            fixture
                .manager
                .begin_replay(&mut Context::from_waker(std::task::Waker::noop()))
        );
        let Some(Active::Replay(future)) = fixture.manager.active.take() else {
            panic!("intervening write requires serialized exact-head validation");
        };
        fixture.manager.finish_replay(block_on(future));
        assert!(fixture.manager.replay.pending.is_none());
        assert!(fixture.manager.replay.retry.is_none());
        fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
        assert!(
            !fixture
                .manager
                .notices
                .snapshot(&original.target.parent, 64, 64 * 1024)
                .unwrap()
                .entries()
                .iter()
                .any(|entry| entry.notice() == &original)
        );
    }
}

#[test]
fn unchanged_capacity_retry_checks_parent_retirement_without_restarting_history() {
    for retired in [false, true] {
        let (mut fixture, _session, context, original, blocker) = capacity_fixture();
        if retired {
            context.retire();
        }
        fixture
            .manager
            .notices
            .acknowledge_recovered(&[blocker])
            .unwrap();
        assert!(
            fixture
                .manager
                .begin_replay(&mut Context::from_waker(std::task::Waker::noop()))
        );
        assert!(
            fixture.manager.active.is_none(),
            "unchanged capacity retry reread history"
        );
        assert!(fixture.manager.replay.pending.is_none());
        let batch = fixture
            .manager
            .notices
            .snapshot(&original.target.parent, 64, 64 * 1024)
            .unwrap();
        assert_eq!(
            batch
                .entries()
                .iter()
                .any(|entry| entry.notice() == &original),
            !retired
        );
    }
}

#[test]
fn stale_source_preserves_catalog_frontier_and_requests_one_later_sweep() {
    let (mut fixture, _session, context, _, original) = history_fixture();
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "sibling", "mode": "persistent"
            }}))
            .ok
    );
    fixture.drive(|f| f.manager.active.is_none());
    let catalog = block_on(fixture.journal.catalog(None, 1)).unwrap();
    assert!(catalog.next.is_some());
    let snapshot = block_on(fixture.journal.inspect(catalog.entries[0].id.clone())).unwrap();
    let replay = Replay {
        catalog: catalog.next,
        source: Some((snapshot.clone(), None)),
        ..Replay::default()
    };
    let JournalPublication::Confirmed(_) = block_on(fixture.journal.mutate(
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::History(ManagedHistoryItem {
            kind: ManagedHistoryKind::Conversation,
            work_id: None,
            user: None,
            assistant: Some("intervening source write".into()),
            user_truncated: false,
            assistant_truncated: false,
        })]),
    ))
    .unwrap() else {
        panic!("confirmed source write");
    };
    let targets = vec![Arc::downgrade(&context)];
    let outcome = block_on(advance(
        fixture.journal.clone(),
        fixture.manager.retry.clone(),
        replay,
        targets.clone(),
    ));
    assert!(!outcome.error);
    assert!(outcome.replay.source.is_none());
    assert!(
        outcome.replay.catalog.is_some(),
        "stale source erased sibling frontier"
    );
    assert!(outcome.replay.rescan);
    let mut replay = outcome.replay;
    block_on(step(
        &fixture.journal,
        &fixture.manager.retry,
        &mut replay,
        &targets,
        &mut None,
    ))
    .unwrap();
    assert_ne!(
        replay.source.as_ref().unwrap().0.head.id,
        catalog.entries[0].id
    );
    // The skipped source remains discoverable without a new external mutation.
    fixture.manager.register_parent_context(&context).unwrap();
    fixture.manager.replay = replay;
    fixture.manager.replay_reset = false;
    fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&original.target.parent, 64, 64 * 1024)
            .unwrap()
            .entries()
            .iter()
            .any(|entry| entry.notice() == &original)
    );
}

fn sibling_original(
    fixture: &mut Fixture,
    session: &Session,
    original: &ManagedNotice,
) -> ManagedNotice {
    assert!(
        fixture
            .command(serde_json::json!({"create": {
                "name": "second-source", "mode": "persistent"
            }}))
            .ok
    );
    fixture.drive(|f| f.manager.active.is_none());
    let snapshot = block_on(fixture.journal.inspect("child-2".into())).unwrap();
    let snapshot = append(
        fixture,
        snapshot,
        JournalMutation::Relationship {
            parent_id: Some(original.target.parent.id.clone()),
            parent_generation: Some(1),
            parent_owner: Some(JournalTranscript {
                session_id: session.id(),
                incarnation: session.incarnation_id(),
            }),
        },
    );
    let mut second = original.clone();
    second.source.source.id = "child-2".into();
    second.source_sequence = NonZeroU64::new(snapshot.head.next_sequence).unwrap();
    second.target.relationship_generation = NonZeroU64::new(snapshot.head.revision).unwrap();
    append(
        fixture,
        snapshot,
        JournalMutation::AppendHistory(vec![JournalRecord::Notice(second.clone())]),
    );
    second
}

#[test]
fn initial_parent_registration_needs_only_one_catalog_sweep() {
    let mut fixture = Fixture::new(vec![]);
    let session = fixture.notice_session();
    let context = Arc::new(ParentNoticeContext::new(
        &session,
        NoticePrincipal {
            id: session.id().to_string(),
            generation: NonZeroU64::new(1).unwrap(),
        },
        &fixture.manager.notices,
    ));
    fixture.manager.register_parent_context(&context).unwrap();
    let mut cx = Context::from_waker(std::task::Waker::noop());
    // One empty catalog read followed by its completion, not a second sweep
    // caused by registration before there was any frontier to invalidate.
    for _ in 0..2 {
        assert!(fixture.manager.begin_replay(&mut cx));
        let Some(Active::Replay(future)) = fixture.manager.active.take() else {
            panic!("actual replay admission");
        };
        fixture.manager.finish_replay(block_on(future));
    }
    assert!(fixture.manager.replay.done);
    assert!(!fixture.manager.begin_replay(&mut cx));
    assert!(fixture.factory.provider.requests().is_empty());
}

#[test]
fn queued_resweep_discovers_a_source_created_after_the_old_catalog_ended() {
    let (mut fixture, session, context, snapshot, original) = history_fixture();
    let catalog = block_on(fixture.journal.catalog(None, 1)).unwrap();
    assert!(catalog.next.is_none());
    assert_eq!(catalog.entries[0].id, snapshot.head.id);
    // Retain the exact exhausted catalog frontier while its source history is
    // still in progress. The subsequently created source is absent from it.
    let frontier = Replay {
        catalog_done: true,
        source: Some((snapshot.clone(), None)),
        ..Replay::default()
    };
    let second = sibling_original(&mut fixture, &session, &original);
    // No stale-source Conflict can rescue a lost registration/write invalidation.
    block_on(fixture.journal.history(snapshot, None, 1)).unwrap();
    fixture.manager.replay = frontier;
    fixture.manager.register_parent_context(&context).unwrap();
    assert!(fixture.manager.replay_reset);
    fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    assert!(
        fixture
            .manager
            .notices
            .snapshot(&second.target.parent, 64, 64 * 1024)
            .unwrap()
            .entries()
            .iter()
            .any(|entry| entry.notice() == &second)
    );
    assert!(fixture.factory.provider.requests().is_empty());
}

#[test]
fn partial_parent_delivery_invalidates_replay_before_remaining_sources_finish() {
    use crate::NativeConversation;
    use futures_util::StreamExt;
    let (mut fixture, session, context, _, original) = history_fixture();
    sibling_original(&mut fixture, &session, &original);
    fixture.manager.register_parent_context(&context).unwrap();
    fixture.drive(|f| f.manager.active.is_none() && f.manager.replay.done);
    let conversation = NativeConversation::from_session(session.clone())
        .unwrap()
        .with_notice_context(&context)
        .unwrap();
    // Publication precedes the deliberately unscripted provider failure. The
    // receipt is from the real core checkpoint, not fabricated saved metadata.
    let turn = block_on(conversation.prompt("explicit parent input".into(), 102)).unwrap();
    let _events = block_on(turn.collect::<Vec<_>>());
    let delivered = context.delivery().unwrap();
    assert_eq!(delivered.originals().len(), 2);
    let mut partial = false;
    for _ in 0..10 {
        fixture.manager.replay_reset = false;
        assert!(
            fixture
                .manager
                .begin_delivery(&mut Context::from_waker(std::task::Waker::noop()))
        );
        let Some(Active::Delivery(future)) = fixture.manager.active.take() else {
            panic!("actual delivery admission");
        };
        fixture.manager.finish_delivery(block_on(future)).unwrap();
        if fixture.manager.replay_reset {
            partial = fixture.manager.parents[0].pending.is_some();
            break;
        }
    }
    // A failed assertion must not strand the actual parent's outbox at drop.
    fixture.drive(|f| f.manager.active.is_none() && f.manager.parents[0].completed.is_some());
    block_on(conversation.clear_notice_delivery(&delivered)).unwrap();
    assert!(
        partial,
        "source ACK invalidation waited for every source in the receipt"
    );
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
