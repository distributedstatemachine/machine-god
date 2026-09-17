use super::*;
use futures_executor::block_on;
use machine_god_core::{
    ManagedAgentMode, ManagedAgentState, ManagedConfiguration, ManagedNotifications,
    ManagedPermissionMode, ManagedQueueStatus, SessionId, SessionIncarnationId,
};
use rustix::fs::{Mode, OFlags};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::AtomicU64;

mod directory_capacity;
mod events;
mod skills;
mod workspace;

fn detach() -> JournalMutation {
    JournalMutation::Relationship {
        parent_id: None,
        parent_owner: None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FailurePoint {
    BeforePageRename,
    AfterPageRename,
    BeforeHeadRename,
    AfterHeadRename,
    ReconcileSync,
}
pub(super) fn checkpoint(shared: &Shared, point: FailurePoint) -> Result<(), JournalError> {
    let mut fault = shared.failure.lock().unwrap();
    if *fault == Some(point) {
        *fault = None;
        Err(JournalError::Persistence)
    } else {
        Ok(())
    }
}

struct Fixture {
    path: std::path::PathBuf,
    workers: NativeOwnedWorkerScope,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "mg-managed-journal-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            path,
            workers: NativeOwnedWorkerScope::new(),
        }
    }
    fn root(&self) -> OwnedFd {
        rustix::fs::open(
            &self.path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap()
    }
    fn open(&self) -> ManagedJournal {
        block_on(ManagedJournal::open(
            self.root(),
            self.workers.clone(),
            JournalLimits::default(),
        ))
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.workers.close();
        block_on(self.workers.completion().wait());
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
fn config() -> ManagedConfiguration {
    ManagedConfiguration {
        name: "worker".into(),
        model: Some("model".into()),
        effort: None,
        permission_mode: ManagedPermissionMode::Ask,
        notifications: ManagedNotifications::default(),
    }
}
fn transcript(name: &str) -> JournalTranscript {
    JournalTranscript {
        session_id: SessionId::new(name).unwrap(),
        incarnation: SessionIncarnationId::new("incarnation").unwrap(),
    }
}
fn work(id: &str) -> JournalWork {
    JournalWork {
        id: id.into(),
        source_id: "parent".into(),
        source_owner: transcript("parent"),
        content: "standalone task".into(),
        skills: Vec::new(),
        accepted_at_ms: 1,
        configuration: config(),
    }
}
fn create(id: &str) -> JournalCreate {
    JournalCreate {
        id: id.into(),
        mode: ManagedAgentMode::Persistent,
        configuration: config(),
        transcript: transcript(id),
        controller: transcript("parent"),
        parent_id: Some("parent".into()),
        parent_owner: Some(transcript("parent")),
        initial_work: Some(work("work-1")),
    }
}
fn confirmed(publication: JournalPublication) -> JournalSnapshot {
    let JournalPublication::Confirmed(snapshot) = publication else {
        panic!("confirmed publication required: {publication:?}");
    };
    *snapshot
}

#[test]
fn lineage_survives_empty_creation_and_exact_reparent_history() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut record = create("lineage");
    record.initial_work = None;
    let initial = confirmed(block_on(journal.create(record)).unwrap());
    assert_eq!(initial.head.controller, transcript("parent"));
    let reparented = confirmed(
        block_on(journal.mutate(
            initial,
            JournalMutation::Relationship {
                parent_id: Some("other".into()),
                parent_owner: Some(transcript("other")),
            },
        ))
        .unwrap(),
    );
    assert_eq!(reparented.head.controller, transcript("parent"));
    assert_eq!(reparented.head.parent_owner, Some(transcript("other")));
    let history = block_on(journal.history(reparented.clone(), None, 100)).unwrap();
    assert!(history.records.iter().any(|record| matches!(record, JournalRecord::Control(control) if control.parent_owner == Some(transcript("other")) && control.controller == transcript("parent"))));
    assert!(matches!(
        block_on(journal.mutate(
            reparented,
            JournalMutation::Relationship {
                parent_id: None,
                parent_owner: Some(transcript("other"))
            }
        )),
        Err(JournalError::Invalid)
    ));
}

#[test]
fn idle_cancel_requires_durable_intent_and_clears_it() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut record = create("idle-cancel");
    record.initial_work = None;
    let initial = confirmed(block_on(journal.create(record)).unwrap());
    assert!(matches!(
        block_on(journal.mutate(initial.clone(), JournalMutation::CancelIdle)),
        Err(JournalError::Conflict)
    ));
    let intent = confirmed(
        block_on(journal.mutate(initial, JournalMutation::Intent(JournalIntent::Cancel))).unwrap(),
    );
    let settled = confirmed(block_on(journal.mutate(intent, JournalMutation::CancelIdle)).unwrap());
    assert_eq!(settled.head.status, ManagedAgentState::Idle);
    assert_eq!(settled.head.intent, None);
    assert!(settled.head.queue.is_empty());
}

#[test]
fn exact_notice_envelope_is_pageable_and_validated() {
    use crate::managed::notices::*;
    use std::num::NonZeroU64;
    let fixture = Fixture::new();
    let journal = fixture.open();
    let initial = confirmed(block_on(journal.create(create("notice"))).unwrap());
    let notice = ManagedNotice {
        source: WorkNoticeIdentity {
            source: NoticePrincipal {
                id: "notice".into(),
                generation: NonZeroU64::new(1).unwrap(),
            },
            work_id: "work-1".into(),
            work_generation: NonZeroU64::new(1).unwrap(),
        },
        source_sequence: NonZeroU64::new(initial.head.next_sequence).unwrap(),
        target: NoticeTarget {
            parent: NoticePrincipal {
                id: "parent".into(),
                generation: NonZeroU64::new(9).unwrap(),
            },
            relationship_generation: NonZeroU64::new(2).unwrap(),
        },
        event: NoticeEvent::Started,
        history: None,
    };
    let snapshot = confirmed(
        block_on(journal.mutate(
            initial,
            JournalMutation::AppendHistory(vec![JournalRecord::Notice(notice.clone())]),
        ))
        .unwrap(),
    );
    let history = block_on(journal.history(snapshot.clone(), None, 100)).unwrap();
    assert!(
        history
            .records
            .contains(&JournalRecord::Notice(notice.clone()))
    );
    let mut invalid = notice;
    invalid.event = NoticeEvent::Interval {
        state: ManagedAgentState::Running,
        first_tick: NonZeroU64::new(1).unwrap(),
        last_tick: NonZeroU64::new(3).unwrap(),
        coalesced_intervals: NonZeroU64::new(1).unwrap(),
        gap: true,
    };
    assert!(matches!(
        block_on(journal.mutate(
            snapshot,
            JournalMutation::AppendHistory(vec![JournalRecord::Notice(invalid)])
        )),
        Err(JournalError::Invalid)
    ));
}

#[test]
fn suppressed_notice_checkpoints_only_the_exact_next_source_sequence() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let initial = confirmed(block_on(journal.create(create("suppressed"))).unwrap());
    let sequence = initial.head.next_sequence;
    for invalid in [sequence - 1, sequence + 1] {
        assert!(matches!(
            block_on(journal.mutate(initial.clone(), JournalMutation::SuppressedNotice(invalid))),
            Err(JournalError::Conflict)
        ));
    }
    let snapshot = mutate(
        &journal,
        initial,
        JournalMutation::SuppressedNotice(sequence),
    );
    assert_eq!(snapshot.head.next_sequence, sequence + 1);
    assert_eq!(snapshot.head.notice_cursor, sequence);
    let page = block_on(journal.history(snapshot.clone(), None, 1)).unwrap();
    assert!(matches!(
        page.records.as_slice(),
        [JournalRecord::Control(control)] if control.notice_cursor == sequence
    ));
    assert!(matches!(
        block_on(journal.mutate(snapshot, JournalMutation::SuppressedNotice(sequence))),
        Err(JournalError::Conflict)
    ));
}

#[test]
fn archived_source_accepts_exact_delivery_ack_without_reopening() {
    use crate::managed::{notices::*, prompt_context::NoticeCheckpoint};
    use machine_god_core::SessionRevision;
    use std::num::NonZeroU64;
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut record = create("ack-source");
    record.initial_work = None;
    let snapshot = confirmed(block_on(journal.create(record)).unwrap());
    let snapshot = confirmed(
        block_on(journal.mutate(snapshot, JournalMutation::Intent(JournalIntent::Archive)))
            .unwrap(),
    );
    let snapshot = confirmed(block_on(journal.mutate(snapshot, JournalMutation::Archive)).unwrap());
    let acknowledgement = JournalRecord::NoticeAcknowledged {
        identity: NoticeIdentity {
            source: WorkNoticeIdentity {
                source: NoticePrincipal {
                    id: "ack-source".into(),
                    generation: NonZeroU64::new(1).unwrap(),
                },
                work_id: "work-1".into(),
                work_generation: NonZeroU64::new(1).unwrap(),
            },
            source_sequence: NonZeroU64::new(1).unwrap(),
            kind: NoticeKind::Started,
        },
        target: NoticeTarget {
            parent: NoticePrincipal {
                id: "parent".into(),
                generation: NonZeroU64::new(1).unwrap(),
            },
            relationship_generation: NonZeroU64::new(1).unwrap(),
        },
        checkpoint: NoticeCheckpoint {
            session_id: transcript("parent").session_id,
            incarnation_id: transcript("parent").incarnation,
            expected_revision: SessionRevision(2),
            turn_sequence: 1,
            first_user_message: 0,
        },
    };
    let snapshot = confirmed(
        block_on(journal.mutate(
            snapshot,
            JournalMutation::AppendHistory(vec![acknowledgement.clone()]),
        ))
        .unwrap(),
    );
    assert_eq!(snapshot.head.status, ManagedAgentState::Archived);
    assert_eq!(snapshot.head.generation, 1);
    assert!(
        block_on(journal.history(snapshot, None, 100))
            .unwrap()
            .records
            .contains(&acknowledgement)
    );
}
fn mutate(
    journal: &ManagedJournal,
    snapshot: JournalSnapshot,
    mutation: JournalMutation,
) -> JournalSnapshot {
    confirmed(block_on(journal.mutate(snapshot, mutation)).unwrap())
}

#[test]
fn construction_is_inert_and_owner_lease_excludes_another_actual_manager() {
    let fixture = Fixture::new();
    drop(ManagedJournal::open(
        fixture.root(),
        fixture.workers.clone(),
        JournalLimits::default(),
    ));
    assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 0);
    let journal = fixture.open();
    let lease = journal.owner_lease();
    assert!(lease.belongs_to(&journal));
    drop(journal);
    assert_eq!(
        block_on(ManagedJournal::open(
            fixture.root(),
            fixture.workers.clone(),
            JournalLimits::default()
        ))
        .unwrap_err(),
        JournalError::Busy
    );
    drop(lease);
    drop(fixture.open());
}

#[test]
fn full_64k_prompt_and_32_milestones_are_paged_before_head_confirmation() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut input = create("child");
    input.configuration.notifications.milestones =
        (0..32).map(|n| format!("milestone-{n}")).collect();
    let accepted = input.initial_work.as_mut().unwrap();
    accepted.content = "\u{1}".repeat(65_536);
    accepted.configuration = input.configuration.clone();
    let snapshot = confirmed(block_on(journal.create(input.clone())).unwrap());
    let stored = block_on(journal.read_work(snapshot.head.queue[0].page.clone())).unwrap();
    assert_eq!(stored, input.initial_work.unwrap());
    assert!(
        std::fs::metadata(fixture.path.join(filesystem::head_name("child")))
            .unwrap()
            .len()
            < 128 * 1024
    );
    assert!(snapshot.head.queue[0].page.length > 65_536);
}

#[test]
fn stale_foreign_and_mutated_snapshots_cannot_publish() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let newer = mutate(
        &journal,
        snapshot.clone(),
        JournalMutation::Relationship {
            parent_id: None,
            parent_owner: None,
        },
    );
    assert_eq!(
        block_on(journal.mutate(snapshot, detach())).unwrap_err(),
        JournalError::Conflict
    );
    let mut forged = newer.clone();
    forged.head.configuration.name = "forged".into();
    assert_eq!(
        block_on(journal.mutate(forged, detach())).unwrap_err(),
        JournalError::Conflict
    );
    let other = Fixture::new();
    let foreign = other.open();
    assert_eq!(
        block_on(foreign.mutate(newer, detach())).unwrap_err(),
        JournalError::Conflict
    );
}

#[test]
fn fifo_head_cannot_be_bypassed_and_restart_requires_explicit_resolution() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let snapshot = mutate(&journal, snapshot, JournalMutation::Enqueue(work("work-2")));
    assert_eq!(
        block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::HeadState {
                work_id: "work-2".into(),
                status: ManagedQueueStatus::Running,
                failure: None
            }
        ))
        .unwrap_err(),
        JournalError::Conflict
    );
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::HeadState {
            work_id: "work-1".into(),
            status: ManagedQueueStatus::Running,
            failure: None,
        },
    );
    drop(snapshot);
    drop(journal);
    let journal = fixture.open();
    let observed = block_on(journal.inspect("child".into())).unwrap();
    let recovered = confirmed(block_on(journal.recover(observed)).unwrap());
    assert_eq!(recovered.head.status, ManagedAgentState::Interrupted);
    assert!(
        recovered
            .head
            .queue
            .iter()
            .all(|work| work.status == ManagedQueueStatus::Interrupted)
    );
    assert_eq!(
        block_on(journal.mutate(
            recovered.clone(),
            JournalMutation::HeadState {
                work_id: "work-1".into(),
                status: ManagedQueueStatus::Running,
                failure: None
            }
        ))
        .unwrap_err(),
        JournalError::Conflict
    );
    let resumed = mutate(
        &journal,
        recovered,
        JournalMutation::ResolveHead {
            work_id: "work-1".into(),
            retry: true,
        },
    );
    assert_eq!(resumed.head.queue[0].status, ManagedQueueStatus::Pending);
    assert_eq!(
        resumed.head.queue[1].status,
        ManagedQueueStatus::Interrupted
    );
}

#[test]
fn cancellation_intent_precedes_settlement_and_archive_reopen_never_retries() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    assert_eq!(
        block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::HeadState {
                work_id: "work-1".into(),
                status: ManagedQueueStatus::Cancelled,
                failure: None
            }
        ))
        .unwrap_err(),
        JournalError::Conflict
    );
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::Intent(JournalIntent::Cancel),
    );
    assert_eq!(snapshot.head.intent, Some(JournalIntent::Cancel));
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::HeadState {
            work_id: "work-1".into(),
            status: ManagedQueueStatus::Cancelled,
            failure: None,
        },
    );
    assert_eq!(snapshot.head.status, ManagedAgentState::Idle);
    let snapshot = mutate(&journal, snapshot, JournalMutation::Enqueue(work("work-2")));
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::Intent(JournalIntent::Archive),
    );
    let archived = mutate(&journal, snapshot, JournalMutation::Archive);
    let reopened = mutate(
        &journal,
        archived.clone(),
        JournalMutation::Reopen(transcript("new-session")),
    );
    assert_eq!(reopened.head.generation, archived.head.generation + 1);
    assert_eq!(reopened.head.status, ManagedAgentState::Interrupted);
    assert_eq!(
        reopened.head.queue[0].status,
        ManagedQueueStatus::Interrupted
    );
    assert!(
        block_on(journal.history(reopened, None, 100))
            .unwrap()
            .records
            .len()
            > 4
    );
}

#[test]
fn milestone_operation_labels_preserve_the_core_contract() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    for operation_id in ["host:call/7", "invalid operation", "invalid\toperation"] {
        let event = JournalRecord::Event(machine_god_core::ManagedEvent {
            sequence: snapshot.head.next_sequence,
            revision: snapshot.head.revision + 1,
            id: "event-1".into(),
            timestamp_ms: 0,
            kind: machine_god_core::ManagedEventKind::MilestoneRecorded {
                operation_id: operation_id.into(),
                source_child_id: "child".into(),
                target_parent_id: Some("parent".into()),
                notice_emitted: true,
                work_item_id: "work-1".into(),
                name: "ready".into(),
            },
        });
        let result = block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::AppendHistory(vec![event]),
        ));
        if operation_id == "host:call/7" {
            snapshot = confirmed(result.unwrap());
        } else {
            assert_eq!(result.unwrap_err(), JournalError::Invalid);
        }
    }
}

#[test]
fn every_publication_phase_retains_ambiguity_until_exact_durability_repair() {
    for phase in [
        FailurePoint::BeforePageRename,
        FailurePoint::AfterPageRename,
        FailurePoint::BeforeHeadRename,
        FailurePoint::AfterHeadRename,
    ] {
        let fixture = Fixture::new();
        let journal = fixture.open();
        *journal.shared.failure.lock().unwrap() = Some(phase);
        let JournalPublication::Ambiguous(receipt) =
            block_on(journal.create(create("child"))).unwrap()
        else {
            panic!("ambiguous");
        };
        assert!(journal.shared.state.lock().unwrap().reserved > 0);
        assert_eq!(
            block_on(journal.create(create("other"))).unwrap_err(),
            JournalError::Ambiguous
        );
        *journal.shared.failure.lock().unwrap() = Some(FailurePoint::ReconcileSync);
        assert_eq!(
            block_on(journal.reconcile(receipt.clone())).unwrap_err(),
            JournalError::Persistence
        );
        assert!(journal.pending_receipt().is_some());
        let repaired = block_on(journal.reconcile(receipt)).unwrap();
        assert_eq!(
            matches!(repaired, JournalPublication::Confirmed(_)),
            phase == FailurePoint::AfterHeadRename
        );
        assert_eq!(
            matches!(repaired, JournalPublication::NotApplied),
            phase != FailurePoint::AfterHeadRename
        );
        assert!(journal.pending_receipt().is_none());
        assert_eq!(journal.shared.state.lock().unwrap().reserved, 0);
    }
}

#[test]
fn dropped_ambiguous_response_keeps_receipt_and_all_physical_charges() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    *journal.shared.failure.lock().unwrap() = Some(FailurePoint::AfterPageRename);
    drop(block_on(journal.create(create("child"))).unwrap());
    let receipt = journal.pending_receipt().unwrap();
    assert!(matches!(
        block_on(journal.reconcile(receipt)).unwrap(),
        JournalPublication::NotApplied
    ));
    let before = journal.shared.state.lock().unwrap().used;
    assert!(before > filesystem::FILE_OVERHEAD);
    drop(journal);
    let reopened = fixture.open();
    assert_eq!(reopened.shared.state.lock().unwrap().used, before);
    assert_eq!(
        block_on(reopened.inspect("child".into())).unwrap_err(),
        JournalError::Missing
    );
}

#[test]
fn paging_is_bounded_continues_whole_history_and_rejects_stale_cursor() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let mut snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    for _ in 0..8 {
        snapshot = mutate(
            &journal,
            snapshot,
            JournalMutation::Relationship {
                parent_id: None,
                parent_owner: None,
            },
        );
    }
    let first = block_on(journal.history(snapshot.clone(), None, 2)).unwrap();
    assert_eq!(first.records.len(), 2);
    let stale = first.next.clone().unwrap();
    let mut count = first.records.len();
    let mut next = first.next;
    while let Some(cursor) = next {
        let page = block_on(journal.history(snapshot.clone(), Some(cursor), 2)).unwrap();
        assert!(page.records.len() <= 2);
        count += page.records.len();
        next = page.next;
    }
    assert_eq!(count, 19);
    let newer = mutate(&journal, snapshot, detach());
    assert_eq!(
        block_on(journal.history(newer, Some(stale), 2)).unwrap_err(),
        JournalError::Conflict
    );
}

#[test]
fn referenced_page_corruption_is_not_a_valid_head_or_work_receipt() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let reference = snapshot.head.queue[0].page.clone();
    std::fs::write(fixture.path.join(filesystem::page_name(&reference)), b"{}").unwrap();
    assert_eq!(
        block_on(journal.read_work(reference)).unwrap_err(),
        JournalError::Invalid
    );
    assert_eq!(
        block_on(journal.inspect("child".into())).unwrap_err(),
        JournalError::Invalid
    );
}

#[test]
fn equal_byte_inode_replacement_rejects_old_cas_observation() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let path = fixture.path.join(filesystem::head_name("child"));
    let replacement = fixture.path.join("replacement");
    std::fs::write(&replacement, std::fs::read(&path).unwrap()).unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::rename(replacement, path).unwrap();
    assert_eq!(
        block_on(journal.mutate(snapshot, detach())).unwrap_err(),
        JournalError::Conflict
    );
}

#[test]
fn operation_slot_and_aggregate_reservations_fail_before_acceptance() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let slot = OperationSlot::acquire(journal.shared.clone()).unwrap();
    assert_eq!(
        block_on(journal.create(create("child"))).unwrap_err(),
        JournalError::Busy
    );
    drop(slot);
    let previous = journal.shared.state.lock().unwrap().used;
    journal.shared.state.lock().unwrap().used = journal.shared.limits.aggregate_bytes;
    assert_eq!(
        block_on(journal.create(create("child"))).unwrap_err(),
        JournalError::Limit
    );
    journal.shared.state.lock().unwrap().used = previous;
    assert!(!fixture.path.join(filesystem::head_name("child")).exists());
    let _snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
}

#[test]
fn configuration_is_frozen_per_work_and_history_has_an_aggregate_page_budget() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let first = snapshot.head.queue[0].page.clone();
    let mut changed = config();
    changed.name = "new".into();
    changed.notifications.started = true;
    let snapshot = mutate(
        &journal,
        snapshot,
        JournalMutation::Configure(changed.clone()),
    );
    assert_eq!(
        block_on(journal.read_work(first)).unwrap().configuration,
        config()
    );
    let mut second = work("work-2");
    second.configuration = changed;
    let snapshot = mutate(&journal, snapshot, JournalMutation::Enqueue(second));
    let records = (0..100)
        .map(|_| {
            JournalRecord::History(machine_god_core::ManagedHistoryItem {
                kind: machine_god_core::ManagedHistoryKind::Conversation,
                work_id: None,
                user: Some("x".repeat(4096)),
                assistant: None,
                user_truncated: false,
                assistant_truncated: false,
            })
        })
        .collect();
    let snapshot = mutate(&journal, snapshot, JournalMutation::AppendHistory(records));
    let first = block_on(journal.history(snapshot.clone(), None, 100)).unwrap();
    assert_eq!(first.records.len(), 100);
    let next = block_on(journal.history(snapshot, first.next, 100)).unwrap();
    assert!(!next.records.is_empty());
}

#[test]
fn catalog_pages_nonresident_children_and_new_owner_requires_recovery() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    for index in 0..5 {
        let _snapshot =
            confirmed(block_on(journal.create(create(&format!("child-{index}")))).unwrap());
    }
    drop(journal);
    let journal = fixture.open();
    let mut next = None;
    let mut ids = Vec::new();
    loop {
        let page = block_on(journal.catalog(next, 2)).unwrap();
        assert!(page.entries.len() <= 2);
        for entry in page.entries {
            assert_eq!(entry.generation, 1);
            assert_eq!(entry.revision, 1);
            assert_eq!(entry.name, "worker");
            assert_eq!(entry.parent_id.as_deref(), Some("parent"));
            assert_eq!(entry.status, ManagedAgentState::Queued);
            assert!(entry.recovery_required);
            ids.push(entry.id);
        }
        next = page.next;
        if next.is_none() {
            break;
        }
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 5);
    let snapshot = block_on(journal.inspect(ids[0].clone())).unwrap();
    assert!(snapshot.recovery_required());
    assert_eq!(
        block_on(journal.mutate(
            snapshot.clone(),
            JournalMutation::HeadState {
                work_id: "work-1".into(),
                status: ManagedQueueStatus::Running,
                failure: None
            }
        ))
        .unwrap_err(),
        JournalError::RecoveryRequired
    );
    let recovered = confirmed(block_on(journal.recover(snapshot)).unwrap());
    assert!(!recovered.recovery_required());
    assert_eq!(
        block_on(journal.recover(recovered)).unwrap_err(),
        JournalError::Conflict
    );
}

#[test]
fn owner_lease_can_be_held_by_owned_work_across_a_dropped_store_handle() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let owner = journal.owner_lease();
    let (entered, observed) = std::sync::mpsc::sync_channel(1);
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    fixture
        .workers
        .spawn(move || {
            let _owner = owner;
            entered.send(()).unwrap();
            wait.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        })
        .unwrap();
    observed
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    drop(journal);
    let contender_workers = NativeOwnedWorkerScope::new();
    let contender = block_on(ManagedJournal::open(
        fixture.root(),
        contender_workers.clone(),
        JournalLimits::default(),
    ));
    contender_workers.close();
    block_on(contender_workers.completion().wait());
    assert_eq!(contender.unwrap_err(), JournalError::Busy);
    release.send(()).unwrap();
    fixture.workers.close();
    block_on(fixture.workers.completion().wait());
    let new_workers = NativeOwnedWorkerScope::new();
    let reopened = block_on(ManagedJournal::open(
        fixture.root(),
        new_workers.clone(),
        JournalLimits::default(),
    ))
    .unwrap();
    drop(reopened);
    new_workers.close();
    block_on(new_workers.completion().wait());
}

#[test]
fn immutable_work_references_bind_owner_generation_sequence_length_and_digest() {
    let fixture = Fixture::new();
    let journal = fixture.open();
    let snapshot = confirmed(block_on(journal.create(create("child"))).unwrap());
    let reference = snapshot.head.queue[0].page.clone();
    for field in 0..6 {
        let mut changed = reference.clone();
        match field {
            0 => changed.owner = transcript("different-owner"),
            1 => changed.generation += 1,
            2 => changed.sequence += 1,
            3 => changed.length -= 1,
            4 => changed.digest[0] ^= 1,
            _ => changed.child_id = "different-child".into(),
        }
        assert_eq!(
            block_on(journal.read_work(changed)).unwrap_err(),
            JournalError::Invalid
        );
    }
    let accepted = block_on(journal.read_work(reference)).unwrap();
    assert_eq!(accepted.source_owner, transcript("parent"));
}
