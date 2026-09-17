use super::*;

fn staged(prepared: PreparedNotice) -> StagedNotice {
    let PreparedNotice::Staged(stage) = prepared else {
        panic!("expected staged original");
    };
    stage
}

#[test]
fn notice_size_count_matches_json_without_an_encoded_buffer() {
    let manager = ManagedNotices::new(NoticeLimits::default(), Clock::new()).unwrap();
    let work = work(&manager, "child", started_policy());
    let stage = staged(manager.prepare_durable_start(&work, nz(1), None).unwrap());
    let mut notice = stage.notice().clone();
    notice.source.source.id = "snow 雪\n\"quote\"".into();
    notice.history = Some(NoticeHistoryRef {
        record_id: "\\escaped".into(),
        source_sequence: nz(1),
    });
    let encoded = serde_json::to_vec(&notice).unwrap();
    assert_eq!(
        super::super::state::encoded_notice_bytes(&notice, encoded.len()),
        Ok(encoded.len())
    );
    for limit in [0, 1, encoded.len() - 1] {
        assert_eq!(
            super::super::state::encoded_notice_bytes(&notice, limit),
            Err(NoticeError::Capacity)
        );
    }
}

#[test]
fn tiny_durable_notice_limits_refund_publication_without_staging() {
    for notice_bytes in [1, 16, 128] {
        let manager = ManagedNotices::new(
            NoticeLimits {
                notice_bytes,
                ..NoticeLimits::default()
            },
            Clock::new(),
        )
        .unwrap();
        let work = work(&manager, "child", started_policy());
        assert!(matches!(
            manager.prepare_durable_start(&work, nz(1), None),
            Err(NoticeError::Capacity)
        ));
        assert_eq!(manager.usage().publication_records, 0);
        assert_eq!(manager.usage().publication_bytes, 0);
        assert_eq!(manager.usage().retained_records, 0);
        assert_eq!(manager.usage().retained_bytes, 0);
        assert!(manager.pending_notice(&work).unwrap().is_none());
    }
}

#[test]
fn durable_terminal_bypasses_full_inbox_without_refunding_held_snapshot() {
    let clock = Clock::new();
    let manager = ManagedNotices::new(
        NoticeLimits {
            records: 1,
            ..NoticeLimits::default()
        },
        clock.clone(),
    )
    .unwrap();
    let work = work(&manager, "child", started_policy());
    manager.start_work(&work, nz(1), None).unwrap();
    let held = snapshot(&manager);
    let before = manager.usage();
    ack_all(&manager, &held);
    assert_eq!(manager.usage().retained_records, 1);
    assert!(matches!(
        manager.prepare_terminal(&work, nz(2), NoticeTerminal::Completed, None),
        Err(NoticeError::Capacity)
    ));
    let stage = staged(
        manager
            .prepare_durable_terminal(&work, nz(2), NoticeTerminal::Completed, None)
            .unwrap(),
    );
    let original = stage.notice().clone();
    assert_eq!(manager.usage().publication_records, 1);
    assert!(manager.usage().publication_bytes <= 3 * NoticeLimits::default().notice_bytes + 2048);
    assert_eq!(manager.usage().retained_bytes, before.retained_bytes);
    assert!(snapshot(&manager).entries().is_empty());
    assert_eq!(
        manager.confirm_durable(&stage),
        Ok(NoticeEmission::Deferred)
    );
    assert!(snapshot(&manager).entries().is_empty());
    assert_eq!(
        manager.restore_durable_original(&original),
        Err(NoticeError::Capacity)
    );
    drop(stage);
    assert_eq!(manager.usage().publication_records, 0);
    assert_eq!(manager.usage().publication_bytes, 0);
    drop(held);
    assert_eq!(
        manager.restore_durable_original(&original),
        Ok(NoticeEmission::Queued)
    );
    assert_eq!(
        manager.restore_durable_original(&original),
        Ok(NoticeEmission::AlreadyRecorded)
    );
    assert_eq!(snapshot(&manager).entries()[0].notice(), &original);
    assert_eq!(clock.created.load(Ordering::Acquire), 0);
}

#[test]
fn deferred_publication_is_one_exact_slot_until_confirmed_or_discarded_observers_drop() {
    let clock = Clock::new();
    let manager = ManagedNotices::new(
        NoticeLimits {
            records: 1,
            ..NoticeLimits::default()
        },
        clock,
    )
    .unwrap();
    let first = work(&manager, "first", started_policy());
    let second = work(&manager, "second", started_policy());
    manager.start_work(&first, nz(1), None).unwrap();
    let original = staged(
        manager
            .prepare_durable_terminal(&first, nz(2), NoticeTerminal::Completed, None)
            .unwrap(),
    );
    let envelope = original.notice().clone();
    assert!(matches!(
        manager.prepare_durable_start(&second, nz(1), None),
        Err(NoticeError::Capacity)
    ));
    drop(original);
    assert_eq!(manager.usage().publication_records, 1);
    assert_eq!(manager.release_work(&first), Err(NoticeError::Busy));
    let recovered = manager.pending_notice(&first).unwrap().unwrap();
    assert_eq!(recovered.notice(), &envelope);
    manager.discard_not_applied(&recovered).unwrap();
    assert_eq!(manager.usage().publication_records, 1);
    assert!(matches!(
        manager.prepare_durable_start(&second, nz(1), None),
        Err(NoticeError::Capacity)
    ));
    drop(recovered);
    let retried = staged(
        manager
            .prepare_durable_terminal(&first, nz(2), NoticeTerminal::Completed, None)
            .unwrap(),
    );
    assert_eq!(retried.notice(), &envelope);
    assert_eq!(
        manager.confirm_durable(&retried),
        Ok(NoticeEmission::Deferred)
    );
    assert_eq!(manager.usage().publication_records, 1);
    drop(retried);
    let next = staged(manager.prepare_durable_start(&second, nz(1), None).unwrap());
    assert_eq!(manager.usage().publication_records, 1);
    assert_eq!(manager.confirm_durable(&next), Ok(NoticeEmission::Deferred));
}

#[test]
fn deferred_terminal_releases_tracker_without_discarding_original_inbox() {
    let clock = Clock::new();
    let manager = ManagedNotices::new(
        NoticeLimits {
            trackers: 1,
            records: 1,
            ..NoticeLimits::default()
        },
        clock,
    )
    .unwrap();
    let first = work(&manager, "first", started_policy());
    manager.start_work(&first, nz(1), None).unwrap();
    let stage = staged(
        manager
            .prepare_durable_terminal(&first, nz(2), NoticeTerminal::Completed, None)
            .unwrap(),
    );
    let original = stage.notice().clone();
    assert_eq!(
        manager.confirm_durable(&stage),
        Ok(NoticeEmission::Deferred)
    );
    drop(stage);
    manager.release_work(&first).unwrap();
    assert_eq!(manager.usage().trackers, 0);
    assert_eq!(snapshot(&manager).entries().len(), 1);
    let replay = manager
        .register_work(
            &original.source,
            ManagedNotifications::default(),
            &relationship("parent"),
            2,
        )
        .unwrap();
    let held = snapshot(&manager);
    ack_all(&manager, &held);
    drop(held);
    assert_eq!(
        manager.restore_notice(&replay, &original),
        Ok(NoticeEmission::Queued)
    );
    manager.release_work(&replay).unwrap();
    assert_eq!(snapshot(&manager).entries()[0].notice(), &original);
}

#[test]
fn last_snapshot_and_queue_refunds_wake_capacity_outside_the_notice_lock() {
    struct Wake {
        inner: std::sync::Weak<super::super::state::Inner>,
        calls: AtomicUsize,
    }
    impl std::task::Wake for Wake {
        fn wake(self: Arc<Self>) {
            if let Some(inner) = self.inner.upgrade() {
                assert!(
                    inner.state.try_lock().is_ok(),
                    "capacity callback under notice lock"
                );
                self.calls.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
    let clock = Clock::new();
    let manager = manager(&clock);
    let work = work(&manager, "child", started_policy());
    manager.start_work(&work, nz(1), None).unwrap();
    let held = snapshot(&manager);
    ack_all(&manager, &held);
    let wake = Arc::new(Wake {
        inner: Arc::downgrade(&manager.inner),
        calls: AtomicUsize::new(0),
    });
    manager.register_capacity_waker(&Waker::from(wake.clone()));
    drop(held);
    assert_eq!(wake.calls.load(Ordering::Acquire), 1);
    manager
        .terminal(&work, nz(2), NoticeTerminal::Completed, None)
        .unwrap();
    manager.register_capacity_waker(&Waker::from(wake.clone()));
    manager.close_work(&work).unwrap();
    assert_eq!(wake.calls.load(Ordering::Acquire), 2);
}
#[test]
fn staging_is_invisible_and_drop_retains_exact_charged_candidate() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let work = work(&manager, "child", started_policy());
    let stage = staged(manager.prepare_start(&work, nz(1), None).unwrap());
    let original = stage.notice().clone();
    assert!(snapshot(&manager).entries().is_empty());
    assert_eq!(manager.usage().staged, 1);
    assert_eq!(manager.usage().retained_records, 1);
    assert!(matches!(
        manager.prepare_terminal(&work, nz(2), NoticeTerminal::Completed, None),
        Err(NoticeError::Busy)
    ));
    drop(stage);
    assert_eq!(manager.usage().retained_records, 1);
    let recovered = manager.pending_notice(&work).unwrap().unwrap();
    assert_eq!(recovered.notice(), &original);
    assert_eq!(
        manager.confirm_durable(&recovered),
        Ok(NoticeEmission::Queued)
    );
    assert_eq!(manager.usage().staged, 0);
    assert_eq!(snapshot(&manager).entries()[0].notice(), &original);
    assert_eq!(manager.confirm_durable(&recovered), Err(NoticeError::Stale));
    assert!(manager.pending_notice(&work).unwrap().is_none());
}
#[test]
fn original_incarnation_survives_staging_and_filters_before_bounded_selection() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let old_work = work(&manager, "old-child", started_policy());
    let stage = staged(manager.prepare_start(&old_work, nz(1), None).unwrap());
    let original = stage.notice().clone();
    let replacement = machine_god_core::SessionIncarnationId::new("replacement").unwrap();
    let mut changed = relationship("parent");
    changed.parent_incarnation = Some(replacement.clone());
    assert_eq!(
        manager.set_relationship(&old_work, &changed),
        Err(NoticeError::StaleSource)
    );
    changed.generation = nz(2);
    manager.set_relationship(&old_work, &changed).unwrap();
    drop(stage);
    let recovered = manager.pending_notice(&old_work).unwrap().unwrap();
    assert_eq!(recovered.notice(), &original);
    manager.confirm_durable(&recovered).unwrap();
    let new_work = manager
        .register_work(&identity("new-child"), started_policy(), &changed, 0)
        .unwrap();
    let new_stage = staged(manager.prepare_start(&new_work, nz(1), None).unwrap());
    manager.confirm_durable(&new_stage).unwrap();
    // The older foreign record cannot consume the one-record selection budget.
    let new_batch = manager
        .snapshot_for_parent(&principal("parent", 1), &replacement, 1, 64 * 1024)
        .unwrap();
    assert_eq!(new_batch.entries()[0].notice(), new_stage.notice());
    assert!(!new_batch.has_more());
    manager.validate_batch(&new_batch).unwrap();
    let old_batch = manager
        .snapshot_for_parent(
            &principal("parent", 1),
            &original.target.parent_incarnation,
            1,
            64 * 1024,
        )
        .unwrap();
    assert_eq!(old_batch.entries()[0].notice(), &original);
    assert_eq!(manager.usage().retained_records, 2);
}

#[test]
fn explicit_not_applied_preserves_source_cursor_and_refunds_only_after_custody_drop() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let work = work(&manager, "child", started_policy());
    let old = staged(manager.prepare_start(&work, nz(1), None).unwrap());
    let identity = old.notice().identity();
    manager.discard_not_applied(&old).unwrap();
    assert_eq!(manager.usage().staged, 0);
    assert_eq!(manager.usage().retained_records, 1);
    let replacement = staged(manager.prepare_start(&work, nz(1), None).unwrap());
    assert_eq!(replacement.notice().identity(), identity);
    assert_eq!(manager.confirm_durable(&old), Err(NoticeError::Stale));
    assert_eq!(manager.discard_not_applied(&old), Err(NoticeError::Stale));
    assert_eq!(manager.usage().retained_records, 2);
    drop(old);
    assert_eq!(manager.usage().retained_records, 1);
    assert_eq!(
        manager.confirm_durable(&replacement),
        Ok(NoticeEmission::Queued)
    );
}
#[test]
fn ambiguous_interval_retains_original_ticks_target_history_and_schedule() {
    let clock = Clock::new();
    let manager = manager(&clock);
    let work = work(&manager, "child", interval_policy(10, None));
    manager.start_work(&work, nz(1), None).unwrap();
    clock.advance(35);
    let stage = staged(
        manager
            .prepare_due(&observe(&work, 2, ManagedAgentState::Running))
            .unwrap(),
    );
    let original = stage.notice().clone();
    assert!(
        matches!(original.event, NoticeEvent::Interval { first_tick, last_tick, coalesced_intervals, gap: true, .. } if first_tick == nz(1) && last_tick == nz(3) && coalesced_intervals == nz(3))
    );
    let mut deadline = manager.wait_deadline(CancellationToken::new());
    assert!(poll(&mut deadline).is_pending());
    assert_eq!(clock.active(), 0);
    clock.advance(65);
    manager
        .set_relationship(
            &work,
            &NoticeRelationship {
                parent_incarnation: Some(
                    machine_god_core::SessionIncarnationId::new("incarnation").unwrap(),
                ),
                generation: nz(2),
                parent: Some(principal("new-parent", 1)),
            },
        )
        .unwrap();
    assert!(matches!(
        manager.prepare_due(&observe(&work, 3, ManagedAgentState::Failed)),
        Err(NoticeError::Busy)
    ));
    drop(stage);
    let stage = manager.pending_notice(&work).unwrap().unwrap();
    assert_eq!(stage.notice(), &original);
    manager.confirm_durable(&stage).unwrap();
    assert_eq!(snapshot(&manager).entries()[0].notice(), &original);
    assert!(poll(&mut deadline).is_ready());
    let next = staged(
        manager
            .prepare_due(&observe(&work, 3, ManagedAgentState::Running))
            .unwrap(),
    );
    assert!(
        matches!(next.notice().event, NoticeEvent::Interval { first_tick, last_tick, coalesced_intervals, gap: true, .. } if first_tick == nz(4) && last_tick == nz(10) && coalesced_intervals == nz(7))
    );
    assert_eq!(next.notice().target.parent.id, "new-parent");
}
#[test]
fn close_stop_and_retirement_keep_uncertain_custody_without_later_visibility() {
    for mode in 0..3 {
        let clock = Clock::new();
        let manager = manager(&clock);
        let work = work(&manager, "child", started_policy());
        let stage = staged(manager.prepare_start(&work, nz(1), None).unwrap());
        match mode {
            0 => manager.close_work(&work).unwrap(),
            1 => manager.stop_work(&work).unwrap(),
            _ => manager.retire_target(&principal("parent", 1)).unwrap(),
        }
        assert_eq!(manager.usage().staged, 1);
        assert_eq!(manager.release_work(&work), Err(NoticeError::Busy));
        assert_eq!(
            manager.confirm_durable(&stage),
            Ok(NoticeEmission::Suppressed)
        );
        assert!(snapshot(&manager).entries().is_empty());
        assert_eq!(manager.usage().retained_records, 1);
        drop(stage);
        assert_eq!(manager.usage().retained_records, 0);
        if mode == 2 {
            manager.stop_work(&work).unwrap();
        }
        manager.release_work(&work).unwrap();
    }
}
#[test]
fn foreign_and_stale_stage_cannot_publish_or_discard_another_candidate() {
    let clock = Clock::new();
    let a = manager(&clock);
    let b = manager(&clock);
    let wa = work(&a, "child", started_policy());
    let wb = work(&b, "child", started_policy());
    let sa = staged(a.prepare_start(&wa, nz(1), None).unwrap());
    let sb = staged(b.prepare_start(&wb, nz(1), None).unwrap());
    assert_eq!(b.confirm_durable(&sa), Err(NoticeError::Stale));
    assert_eq!(b.discard_not_applied(&sa), Err(NoticeError::Stale));
    a.confirm_durable(&sa).unwrap();
    assert_eq!(b.usage().staged, 1);
    assert!(snapshot(&b).entries().is_empty());
    b.confirm_durable(&sb).unwrap();
}
#[test]
fn staging_capacity_covers_sibling_records_and_never_advances_rejected_transition() {
    let clock = Clock::new();
    let manager = ManagedNotices::new(
        NoticeLimits {
            records: 1,
            ..NoticeLimits::default()
        },
        clock,
    )
    .unwrap();
    let a = work(&manager, "a", started_policy());
    let b = work(&manager, "b", started_policy());
    let stage = staged(manager.prepare_start(&a, nz(1), None).unwrap());
    assert!(matches!(
        manager.prepare_start(&b, nz(1), None),
        Err(NoticeError::Capacity)
    ));
    assert!(manager.pending_notice(&b).unwrap().is_none());
    manager.discard_not_applied(&stage).unwrap();
    assert!(matches!(
        manager.prepare_start(&b, nz(1), None),
        Err(NoticeError::Capacity)
    ));
    drop(stage);
    let b = staged(manager.prepare_start(&b, nz(1), None).unwrap());
    assert_eq!(manager.confirm_durable(&b), Ok(NoticeEmission::Queued));
}
#[test]
fn staged_observer_does_not_keep_manager_or_clock_alive() {
    let clock = Clock::new();
    let weak = Arc::downgrade(&clock);
    let manager = manager(&clock);
    let work = work(&manager, "child", started_policy());
    let stage = staged(manager.prepare_start(&work, nz(1), None).unwrap());
    drop(manager);
    drop(clock);
    assert!(weak.upgrade().is_none());
    assert_eq!(stage.notice().source.source.id, "child");
}
