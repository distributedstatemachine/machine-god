use super::*;
use futures_util::task::noop_waker_ref;
use machine_god_core::{BoxFuture, ManagedStopCondition};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

struct Clock {
    now: Mutex<Instant>,
    next: AtomicU64,
    timers: Mutex<BTreeMap<u64, (Instant, Option<Waker>)>>,
    created: AtomicUsize,
}
impl Clock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Instant::now()),
            next: AtomicU64::new(1),
            timers: Mutex::new(BTreeMap::new()),
            created: AtomicUsize::new(0),
        })
    }
    fn set(&self, now: Instant) {
        *self.now.lock().unwrap() = now;
        let wakers: Vec<_> = self
            .timers
            .lock()
            .unwrap()
            .values_mut()
            .filter(|(deadline, _)| now >= *deadline)
            .filter_map(|(_, waker)| waker.take())
            .collect();
        for waker in wakers {
            waker.wake();
        }
    }
    fn advance(&self, ms: u64) {
        self.set(self.now().checked_add(Duration::from_millis(ms)).unwrap());
    }
    fn active(&self) -> usize {
        self.timers.lock().unwrap().len()
    }
}
struct Sleep<'a> {
    clock: &'a Clock,
    deadline: Instant,
    id: Option<u64>,
}
impl Future for Sleep<'_> {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.clock.now() >= self.deadline {
            return Poll::Ready(());
        }
        let id = if let Some(id) = self.id {
            id
        } else {
            let id = self.clock.next.fetch_add(1, Ordering::Relaxed);
            self.id = Some(id);
            id
        };
        let waker = cx.waker().clone();
        let old = self
            .clock
            .timers
            .lock()
            .unwrap()
            .insert(id, (self.deadline, Some(waker)));
        drop(old);
        Poll::Pending
    }
}
impl Drop for Sleep<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            let old = self.clock.timers.lock().unwrap().remove(&id);
            drop(old);
        }
    }
}
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        self.created.fetch_add(1, Ordering::Relaxed);
        Box::pin(Sleep {
            clock: self,
            deadline,
            id: None,
        })
    }
}
fn nz(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}
fn principal(id: &str, generation: u64) -> NoticePrincipal {
    NoticePrincipal {
        id: id.to_owned(),
        generation: nz(generation),
    }
}
fn identity(id: &str) -> WorkNoticeIdentity {
    WorkNoticeIdentity {
        source: principal(id, 1),
        work_id: format!("work-{id}"),
        work_generation: nz(1),
    }
}
fn relationship(parent: &str) -> NoticeRelationship {
    NoticeRelationship {
        generation: nz(1),
        parent: Some(principal(parent, 1)),
    }
}
fn manager(clock: &Arc<Clock>) -> ManagedNotices {
    ManagedNotices::new(NoticeLimits::default(), clock.clone()).unwrap()
}
fn work(manager: &ManagedNotices, id: &str, policy: ManagedNotifications) -> WorkNoticeRef {
    manager
        .register_work(&identity(id), policy, &relationship("parent"), 0)
        .unwrap()
}
fn started_policy() -> ManagedNotifications {
    ManagedNotifications {
        started: true,
        ..ManagedNotifications::default()
    }
}
fn interval_policy(interval: u64, duration: Option<u64>) -> ManagedNotifications {
    ManagedNotifications {
        report_interval_ms: Some(interval),
        report_duration_ms: duration,
        ..ManagedNotifications::default()
    }
}
fn observe(work: &WorkNoticeRef, sequence: u64, state: ManagedAgentState) -> NoticeObservation {
    NoticeObservation {
        work: work.clone(),
        state,
        source_sequence: nz(sequence),
        history: Some(NoticeHistoryRef {
            record_id: "history".to_owned(),
            source_sequence: nz(sequence),
        }),
    }
}
fn poll<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    Pin::new(future).poll(&mut Context::from_waker(noop_waker_ref()))
}
fn snapshot(manager: &ManagedNotices) -> NoticeBatch {
    manager
        .snapshot(&principal("parent", 1), 64, 64 * 1024)
        .unwrap()
}
fn ack_all(manager: &ManagedNotices, batch: &NoticeBatch) {
    let tokens: Vec<_> = batch
        .entries()
        .iter()
        .map(NoticeBatchEntry::token)
        .collect();
    assert_eq!(manager.acknowledge(batch, &tokens), Ok(tokens.len()));
}

#[test]
fn terminal_defaults_are_exactly_once_and_tracker_retention_is_not_a_lifetime_cap() {
    let clock = Clock::new();
    let manager = ManagedNotices::new(
        NoticeLimits {
            trackers: 1,
            ..NoticeLimits::default()
        },
        clock,
    )
    .unwrap();
    for (index, outcome) in [
        NoticeTerminal::Completed,
        NoticeTerminal::Failed,
        NoticeTerminal::Cancelled,
    ]
    .into_iter()
    .cycle()
    .take(90)
    .enumerate()
    {
        let work = work(
            &manager,
            &format!("child-{index}"),
            ManagedNotifications::default(),
        );
        assert_eq!(
            manager.start_work(&work, nz(1), None),
            Ok(NoticeEmission::Suppressed)
        );
        assert_eq!(
            manager.terminal(&work, nz(2), outcome, None),
            Ok(NoticeEmission::Queued)
        );
        assert_eq!(
            manager.terminal(&work, nz(2), outcome, None),
            Ok(NoticeEmission::AlreadyRecorded)
        );
        assert_eq!(manager.release_work(&work), Err(NoticeError::Busy));
        let batch = snapshot(&manager);
        assert_eq!(batch.entries().len(), 1);
        assert_eq!(
            batch.entries()[0].notice().event,
            NoticeEvent::Terminal { outcome }
        );
        ack_all(&manager, &batch);
        manager.release_work(&work).unwrap();
        assert_eq!(manager.usage().trackers, 0);
        drop(batch);
        assert_eq!(manager.usage().retained_records, 0);
    }
}

#[test]
fn policy_is_frozen_and_milestones_are_declared_and_deduplicated() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let mut policy = started_policy();
    policy.milestones.push("halfway".to_owned());
    let work = work(&manager, "child", policy.clone());
    policy.started = false;
    policy.milestones.clear();
    policy.terminal.completed = false;
    assert_eq!(
        manager.start_work(&work, nz(1), None),
        Ok(NoticeEmission::Queued)
    );
    assert_eq!(
        manager.milestone(&work, nz(2), "other", None),
        Err(NoticeError::UndeclaredMilestone)
    );
    assert_eq!(
        manager.milestone(&work, nz(2), "halfway", None),
        Ok(NoticeEmission::Queued)
    );
    assert_eq!(
        manager.milestone(&work, nz(3), "halfway", None),
        Ok(NoticeEmission::AlreadyRecorded)
    );
    assert_eq!(
        manager.terminal(&work, nz(3), NoticeTerminal::Completed, None),
        Ok(NoticeEmission::Queued)
    );
    assert_eq!(snapshot(&manager).entries().len(), 3);
}

#[test]
fn late_intervals_coalesce_one_observed_state_with_explicit_gap() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let work = work(&manager, "child", interval_policy(10, None));
    manager.start_work(&work, nz(1), None).unwrap();
    clock.advance(35);
    let observation = observe(&work, 5, ManagedAgentState::AwaitingApproval);
    assert_eq!(
        manager.observe_due(&observation),
        Ok(NoticeEmission::Queued)
    );
    assert_eq!(
        manager.observe_due(&observation),
        Ok(NoticeEmission::Suppressed)
    );
    let batch = snapshot(&manager);
    assert_eq!(batch.entries().len(), 1);
    let notice = batch.entries()[0].notice();
    assert_eq!(notice.source_sequence, nz(5));
    assert_eq!(notice.history.as_ref().unwrap().source_sequence, nz(5));
    assert_eq!(
        notice.event,
        NoticeEvent::Interval {
            state: ManagedAgentState::AwaitingApproval,
            first_tick: nz(1),
            last_tick: nz(3),
            coalesced_intervals: nz(3),
            gap: true
        }
    );
    ack_all(&manager, &batch);
    clock.advance(5);
    manager
        .observe_due(&observe(&work, 5, ManagedAgentState::Running))
        .unwrap();
    assert_eq!(
        snapshot(&manager).entries()[0].notice().event,
        NoticeEvent::Interval {
            state: ManagedAgentState::Running,
            first_tick: nz(4),
            last_tick: nz(4),
            coalesced_intervals: nz(1),
            gap: false
        }
    );
}

#[test]
fn duration_precedes_interval_and_does_not_invent_missed_notifications() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let work = work(&manager, "child", interval_policy(100, Some(35)));
    manager.start_work(&work, nz(1), None).unwrap();
    let mut deadline = manager.wait_deadline(CancellationToken::new());
    assert_eq!(poll(&mut deadline), Poll::Pending);
    clock.advance(35);
    assert_eq!(poll(&mut deadline), Poll::Ready(Ok(())));
    assert_eq!(
        manager.observe_due(&observe(&work, 2, ManagedAgentState::Running)),
        Ok(NoticeEmission::Suppressed)
    );
    clock.advance(1000);
    assert_eq!(
        manager.observe_due(&observe(&work, 2, ManagedAgentState::Running)),
        Ok(NoticeEmission::Suppressed)
    );
    assert_eq!(manager.usage().pending, 0);
    // Duration stops periodic reports, not the separately enabled final notice.
    manager
        .terminal(&work, nz(3), NoticeTerminal::Completed, None)
        .unwrap();
    assert_eq!(manager.usage().pending, 1);
}

#[test]
fn close_invalidates_snapshots_and_suppresses_future_emission_without_eating_siblings() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let a = work(&manager, "a", started_policy());
    let b = work(&manager, "b", started_policy());
    manager.start_work(&a, nz(1), None).unwrap();
    manager.start_work(&b, nz(1), None).unwrap();
    let old = snapshot(&manager);
    manager.close_work(&a).unwrap();
    assert_eq!(manager.validate_batch(&old), Err(NoticeError::InvalidBatch));
    assert_eq!(
        manager.acknowledge(&old, &[old.entries()[0].token()]),
        Err(NoticeError::InvalidBatch)
    );
    assert_eq!(
        manager.terminal(&a, nz(2), NoticeTerminal::Cancelled, None),
        Err(NoticeError::Closed)
    );
    assert_eq!(
        snapshot(&manager).entries()[0].notice().source.source.id,
        "b"
    );
    manager.release_work(&a).unwrap();
    assert_eq!(manager.start_work(&a, nz(3), None), Err(NoticeError::Stale));
}

#[test]
fn reparent_changes_future_targets_but_never_existing_notice_targets() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let mut policy = started_policy();
    policy.milestones.push("progress".into());
    let work = work(&manager, "child", policy);
    manager.start_work(&work, nz(1), None).unwrap();
    let old = snapshot(&manager);
    manager
        .set_relationship(
            &work,
            &NoticeRelationship {
                generation: nz(2),
                parent: Some(principal("new", 2)),
            },
        )
        .unwrap();
    manager.milestone(&work, nz(2), "progress", None).unwrap();
    manager.validate_batch(&old).unwrap();
    assert_eq!(
        old.entries()[0].notice().target.relationship_generation,
        nz(1)
    );
    let next = manager
        .snapshot(&principal("new", 2), 64, 64 * 1024)
        .unwrap();
    assert_eq!(next.entries().len(), 1);
    assert_eq!(
        next.entries()[0].notice().target.relationship_generation,
        nz(2)
    );
    manager.retire_target(&principal("parent", 1)).unwrap();
    assert_eq!(manager.validate_batch(&old), Err(NoticeError::InvalidBatch));
    manager.validate_batch(&next).unwrap();
}

#[test]
fn snapshot_ack_is_an_exact_non_consuming_subset_with_atomic_stale_rejection() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let a = work(&manager, "a", started_policy());
    let b = work(&manager, "b", started_policy());
    manager.start_work(&a, nz(1), None).unwrap();
    let original = snapshot(&manager);
    manager.start_work(&b, nz(1), None).unwrap();
    manager.validate_batch(&original).unwrap();
    assert_eq!(manager.usage().pending, 2);
    let token = original.entries()[0].token();
    assert_eq!(
        manager.acknowledge(&original, std::slice::from_ref(&token)),
        Ok(1)
    );
    assert_eq!(
        manager.acknowledge(&original, std::slice::from_ref(&token)),
        Err(NoticeError::InvalidBatch)
    );
    assert_eq!(manager.usage().pending, 1);
    let next = snapshot(&manager);
    assert_eq!(
        manager.acknowledge(&next, &[next.entries()[0].token(), token]),
        Err(NoticeError::InvalidBatch)
    );
    assert_eq!(manager.usage().pending, 1);
    let other = super::ManagedNotices::new(NoticeLimits::default(), clock).unwrap();
    assert_eq!(
        other.acknowledge(&next, &[next.entries()[0].token()]),
        Err(NoticeError::InvalidBatch)
    );
    ack_all(&manager, &next);
    assert_eq!(manager.usage().pending, 0);
}

#[test]
fn capacity_is_reserved_before_advancing_source_and_old_batches_keep_their_charge() {
    let clock = Clock::new();
    let manager = ManagedNotices::new(
        NoticeLimits {
            records: 1,
            ..NoticeLimits::default()
        },
        clock,
    )
    .unwrap();
    let work = work(&manager, "child", started_policy());
    manager.start_work(&work, nz(1), None).unwrap();
    let batch = snapshot(&manager);
    assert_eq!(
        manager.terminal(&work, nz(2), NoticeTerminal::Completed, None),
        Err(NoticeError::Capacity)
    );
    ack_all(&manager, &batch);
    assert_eq!(manager.usage().pending, 0);
    assert_eq!(manager.usage().retained_records, 1);
    assert_eq!(
        manager.terminal(&work, nz(2), NoticeTerminal::Completed, None),
        Err(NoticeError::Capacity)
    );
    drop(batch);
    assert_eq!(manager.usage().retained_bytes, 0);
    assert_eq!(
        manager.terminal(&work, nz(2), NoticeTerminal::Completed, None),
        Ok(NoticeEmission::Queued)
    );
}

#[test]
fn bounded_batches_round_robin_sources_and_keep_principals_separate() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let mut policy = started_policy();
    policy.milestones = vec!["one".into(), "two".into()];
    let a = work(&manager, "a", policy);
    let b = work(&manager, "b", started_policy());
    manager.start_work(&a, nz(1), None).unwrap();
    manager.milestone(&a, nz(2), "one", None).unwrap();
    manager.milestone(&a, nz(3), "two", None).unwrap();
    manager.start_work(&b, nz(1), None).unwrap();
    let batch = manager
        .snapshot(&principal("parent", 1), 2, 64 * 1024)
        .unwrap();
    assert_eq!(
        batch
            .entries()
            .iter()
            .map(|entry| entry.notice().source.source.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert!(batch.has_more());
    assert!(batch.encoded_bytes() <= 64 * 1024);
    assert!(
        manager
            .snapshot(&principal("parent", 2), 2, 64 * 1024)
            .unwrap()
            .entries()
            .is_empty()
    );
    assert!(
        manager
            .snapshot(&principal("parent", 1), 2, 0)
            .unwrap()
            .entries()
            .is_empty()
    );
    assert!(
        manager
            .snapshot(&principal("parent", 1), 2, 0)
            .unwrap()
            .has_more()
    );
    ack_all(&manager, &batch);
    assert_eq!(snapshot(&manager).entries().len(), 2);
}

#[test]
fn one_shared_timer_retargets_earlier_later_and_close_without_child_timers() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let a = work(&manager, "a", interval_policy(100, None));
    manager.start_work(&a, nz(1), None).unwrap();
    let mut deadline = manager.wait_deadline(CancellationToken::new());
    assert_eq!(clock.active(), 0);
    assert_eq!(poll(&mut deadline), Poll::Pending);
    assert_eq!(clock.active(), 1);
    let b = work(&manager, "b", interval_policy(10, None));
    manager.start_work(&b, nz(1), None).unwrap();
    assert_eq!(poll(&mut deadline), Poll::Pending);
    assert_eq!(clock.active(), 1);
    manager.close_work(&b).unwrap();
    assert_eq!(poll(&mut deadline), Poll::Pending);
    assert_eq!(clock.active(), 1);
    clock.advance(100);
    assert_eq!(poll(&mut deadline), Poll::Ready(Ok(())));
    assert_eq!(clock.active(), 0);
    manager.close_work(&a).unwrap();
    let mut empty = manager.wait_deadline(CancellationToken::new());
    assert_eq!(poll(&mut empty), Poll::Pending);
    assert_eq!(clock.active(), 0);
}

#[test]
fn cancellation_drop_and_competing_timer_refund_only_the_exact_observer() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let work = work(&manager, "child", interval_policy(10, None));
    manager.start_work(&work, nz(1), None).unwrap();
    let cancel = CancellationToken::new();
    let mut deadline = manager.wait_deadline(cancel.clone());
    assert_eq!(poll(&mut deadline), Poll::Pending);
    let mut other = manager.wait_deadline(CancellationToken::new());
    assert_eq!(poll(&mut other), Poll::Ready(Err(NoticeError::Busy)));
    assert_eq!(clock.active(), 1);
    clock.advance(10);
    cancel.cancel();
    assert_eq!(
        poll(&mut deadline),
        Poll::Ready(Err(NoticeError::Cancelled))
    );
    assert_eq!(clock.active(), 0);
    manager
        .observe_due(&observe(&work, 2, ManagedAgentState::Running))
        .unwrap();
    let mut next = manager.wait_deadline(CancellationToken::new());
    assert_eq!(poll(&mut next), Poll::Pending);
    drop(next);
    assert_eq!(clock.active(), 0);
    let mut replacement = manager.wait_deadline(CancellationToken::new());
    assert_eq!(poll(&mut replacement), Poll::Pending);
    drop(manager);
    assert_eq!(poll(&mut replacement), Poll::Ready(Err(NoticeError::Stale)));
    assert_eq!(clock.active(), 0);
}

fn maximal_instant(base: Instant) -> Instant {
    let mut low = 0u64;
    let mut high = u64::MAX;
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if base.checked_add(Duration::from_secs(middle)).is_some() {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let mut nanos_low = 0u32;
    let mut nanos_high = 999_999_999u32;
    while nanos_low < nanos_high {
        let middle = nanos_low + (nanos_high - nanos_low).div_ceil(2);
        if base.checked_add(Duration::new(low, middle)).is_some() {
            nanos_low = middle;
        } else {
            nanos_high = middle - 1;
        }
    }
    base.checked_add(Duration::new(low, nanos_low)).unwrap()
}

#[test]
fn checked_deadline_addition_and_clock_regression_do_not_advance_source() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let original = clock.now();
    let work = work(&manager, "child", interval_policy(1, None));
    clock.set(maximal_instant(original));
    assert_eq!(
        manager.start_work(&work, nz(1), None),
        Err(NoticeError::DeadlineOverflow)
    );
    // Failed start did not consume its source sequence or queue any start.
    assert_eq!(
        manager.terminal(&work, nz(1), NoticeTerminal::Cancelled, None),
        Ok(NoticeEmission::Queued)
    );
    assert!(matches!(
        manager.register_work(
            &identity("overflow"),
            interval_policy(1, None),
            &relationship("parent"),
            0
        ),
        Err(NoticeError::DeadlineOverflow)
    ));
    clock.set(original);
    let mut deadline = manager.wait_deadline(CancellationToken::new());
    assert_eq!(
        poll(&mut deadline),
        Poll::Ready(Err(NoticeError::ClockRegressed))
    );
}

#[test]
fn input_limits_history_bounds_and_disabled_terminal_stops_are_enforced() {
    let clock = Clock::new();
    for limits in [
        NoticeLimits {
            trackers: 0,
            ..NoticeLimits::default()
        },
        NoticeLimits {
            records: 4097,
            ..NoticeLimits::default()
        },
        NoticeLimits {
            retained_bytes: 1,
            ..NoticeLimits::default()
        },
        NoticeLimits {
            notice_bytes: 16 * 1024 + 1,
            ..NoticeLimits::default()
        },
    ] {
        assert!(matches!(
            ManagedNotices::new(limits, clock.clone()),
            Err(NoticeError::InvalidLimits)
        ));
    }
    let manager = manager(&clock);
    let invalid = ManagedNotifications {
        report_interval_ms: Some(0),
        ..ManagedNotifications::default()
    };
    assert!(matches!(
        manager.register_work(&identity("invalid"), invalid, &relationship("parent"), 0),
        Err(NoticeError::InvalidInput)
    ));
    let mut policy = interval_policy(10, None);
    policy.terminal.completed = false;
    policy.stop_conditions = vec![ManagedStopCondition::Terminal];
    let work = work(&manager, "child", policy);
    let history = NoticeHistoryRef {
        record_id: "x".repeat(256),
        source_sequence: nz(1),
    };
    assert_eq!(
        manager.start_work(&work, nz(1), Some(&history)),
        Err(NoticeError::InvalidInput)
    );
    manager.start_work(&work, nz(1), None).unwrap();
    clock.advance(20);
    assert_eq!(
        manager.observe_due(&observe(&work, 2, ManagedAgentState::Completed)),
        Ok(NoticeEmission::Suppressed)
    );
    // Snapshot stop does not consume the actual terminal event at this sequence.
    assert_eq!(
        manager.terminal(&work, nz(2), NoticeTerminal::Completed, None),
        Ok(NoticeEmission::Suppressed)
    );
    assert_eq!(manager.usage().pending, 0);
    manager.release_work(&work).unwrap();
}

#[test]
fn explicit_restore_preserves_original_identity_target_and_payload_without_timers() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let work = manager
        .register_work(
            &identity("child"),
            interval_policy(10, None),
            &relationship("new-parent"),
            10,
        )
        .unwrap();
    let notice = ManagedNotice {
        source: identity("child"),
        source_sequence: nz(5),
        target: NoticeTarget {
            parent: principal("parent", 1),
            relationship_generation: nz(3),
        },
        event: NoticeEvent::Interval {
            state: ManagedAgentState::Running,
            first_tick: nz(1),
            last_tick: nz(3),
            coalesced_intervals: nz(3),
            gap: true,
        },
        history: Some(NoticeHistoryRef {
            record_id: "original-event".into(),
            source_sequence: nz(5),
        }),
    };
    let encoded = serde_json::to_vec(&notice).unwrap();
    let original: ManagedNotice = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        manager.restore_notice(&work, &original),
        Ok(NoticeEmission::Queued)
    );
    assert_eq!(
        manager.restore_notice(&work, &original),
        Ok(NoticeEmission::AlreadyRecorded)
    );
    let mut changed = original.clone();
    changed.target.relationship_generation = nz(4);
    assert_eq!(
        manager.restore_notice(&work, &changed),
        Err(NoticeError::InvalidInput)
    );
    let batch = snapshot(&manager);
    assert_eq!(batch.entries()[0].notice(), &original);
    assert_eq!(batch.entries()[0].notice().identity(), original.identity());
    let mut deadline = manager.wait_deadline(CancellationToken::new());
    assert_eq!(poll(&mut deadline), Poll::Pending);
    assert_eq!(clock.created.load(Ordering::Acquire), 0);
    manager.stop_work(&work).unwrap();
    assert_eq!(manager.release_work(&work), Err(NoticeError::Busy));
    ack_all(&manager, &batch);
    manager.release_work(&work).unwrap();
}

#[test]
fn reentrant_waker_operations_never_run_under_the_notice_registry_lock() {
    use machine_god_reentrant_waker_test::{Callback, new};
    for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
        let clock = Clock::new();
        let manager = manager(&clock);
        let work = work(&manager, "child", interval_policy(10, None));
        manager.start_work(&work, nz(1), None).unwrap();
        let weak = Arc::downgrade(&manager.inner);
        let (waker, handle) = new(callback, move || {
            if let Some(inner) = weak.upgrade() {
                let _ = inner.usage();
            }
        });
        let mut deadline = manager.wait_deadline(CancellationToken::new());
        assert_eq!(
            Pin::new(&mut deadline).poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        );
        assert_eq!(poll(&mut deadline), Poll::Pending);
        assert_eq!(
            Pin::new(&mut deadline).poll(&mut Context::from_waker(&waker)),
            Poll::Pending
        );
        manager.close_work(&work).unwrap();
        assert_eq!(poll(&mut deadline), Poll::Pending);
        drop(deadline);
        drop(waker);
        assert!(handle.calls() > 0);
        assert_eq!(clock.active(), 0);
    }
}
