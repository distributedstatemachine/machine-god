use super::{Fixture, completed};
use crate::managed::manager::{
    ManagedRuntimeError,
    factory::{
        ManagedPreparation, ManagedRuntimeFactory, ManagedRuntimePreparationKind,
        ManagedRuntimeRequest, PreparedManagedRuntime,
    },
};
use crate::managed::store::JournalTranscript;
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    CancellationToken, ManagedConfiguration, ManagedNotifications, ManagedPermissionMode,
    ManagedSubagentAuthority, SessionId, SessionIncarnationId,
};
use std::{
    sync::{Arc, atomic::Ordering},
    task::{Context, Waker},
};

fn prepare(f: &Fixture, id: &str) -> PreparedManagedRuntime {
    let prepared = block_on(f.factory.prepare(
        ManagedRuntimeRequest {
            kind: ManagedRuntimePreparationKind::Create,
            child_id: id.into(),
            generation: 1,
            transcript: JournalTranscript {
                session_id: SessionId::new(id).unwrap(),
                incarnation: SessionIncarnationId::new("foreground-life").unwrap(),
            },
            journal_owner: f.journal.owner_lease(),
            configuration: ManagedConfiguration {
                name: id.into(),
                model: None,
                effort: None,
                permission_mode: ManagedPermissionMode::Ask,
                notifications: ManagedNotifications::default(),
            },
            origin: None,
            now_ms: 1,
        },
        CancellationToken::new(),
    ))
    .unwrap();
    match prepared {
        ManagedPreparation::Ready(prepared) => prepared,
        ManagedPreparation::Ambiguous(_) => panic!("confirmed fixture preparation"),
    }
}

fn reserve(f: &mut Fixture) -> crate::managed::manager::ManagedForegroundReservation {
    let reservation = f.manager.reserve_foreground().unwrap();
    f.drive(|f| {
        f.manager
            .poll_foreground_reservation(&reservation, &Context::from_waker(Waker::noop()))
            .is_ready()
    });
    reservation
}

fn enroll(
    f: &mut Fixture,
    prepared: PreparedManagedRuntime,
) -> crate::managed::manager::ManagedForegroundSelection {
    let reservation = reserve(f);
    f.manager
        .enroll_foreground(Box::new(prepared), &reservation)
        .unwrap()
}

#[test]
fn staged_same_principal_context_cannot_activate_or_admit_human_work_before_retirement() {
    let mut f = Fixture::new(vec![]);
    let original = f.notified_foreground(f.notice_session());
    let runtime = original.runtime.clone();
    let context = original.notice_context.clone().unwrap();
    let original = enroll(&mut f, original);
    let replacement = f.notified_foreground(f.notice_session());
    let next_context = replacement.notice_context.clone().unwrap();
    let reservation = reserve(&mut f);
    let replacement = f
        .manager
        .stage_foreground(Box::new(replacement), &reservation)
        .unwrap();
    assert_eq!(
        f.manager.activate_foreground(&replacement),
        Err(ManagedRuntimeError::Invalid)
    );
    assert!(!context.is_retired());
    let command = machine_god_core::ManagedSubagentCommand::decode(
        serde_json::json!({"command":{"inspect":{"id":"child-1","sections":["status"]}}}),
    )
    .unwrap();
    assert!(
        f.manager
            .request_human_command(&replacement, command, CancellationToken::new())
            .is_err()
    );
    let mut guard = runtime.begin_quiescence().unwrap();
    block_on(guard.wait_idle()).unwrap();
    guard.try_retire().unwrap();
    assert!(f.manager.retire_foreground(&original));
    assert!(context.is_retired());
    f.manager.activate_foreground(&replacement).unwrap();
    assert!(!next_context.is_retired());
    assert!(
        f.manager.parents.iter().any(|parent| {
            parent.active && parent.context.ptr_eq(&Arc::downgrade(&next_context))
        })
    );
}

#[test]
fn candidate_reservations_are_bounded_inert_and_cannot_be_reused_or_transferred() {
    let mut f = Fixture::new(vec![]);
    let mut foreign = Fixture::new(vec![]);
    f.manager.limits.residents = 1;
    let reservation = f.manager.reserve_foreground().unwrap();
    let cx = Context::from_waker(Waker::noop());
    assert!(
        f.manager
            .poll_foreground_reservation(&reservation, &cx)
            .is_pending()
    );
    assert_eq!(f.factory.prepared.load(Ordering::Acquire), 0);
    assert_eq!(
        reservation.validate_preparation(),
        Err(ManagedRuntimeError::Invalid)
    );
    assert_eq!(
        f.manager.validate_foreground_reservation(&reservation),
        Err(ManagedRuntimeError::Invalid)
    );
    assert!(matches!(
        f.manager.reserve_foreground(),
        Err(ManagedRuntimeError::Capacity)
    ));
    f.drive(|f| f.manager.reserved_foregrounds() == 1);
    assert_eq!(reservation.validate_preparation(), Ok(()));
    assert_eq!(
        f.manager.validate_foreground_reservation(&reservation),
        Ok(())
    );
    assert_eq!(
        foreign
            .manager
            .validate_foreground_reservation(&reservation),
        Err(ManagedRuntimeError::Invalid)
    );
    assert!(!f.manager.has_capacity());
    assert_eq!(
        foreign
            .manager
            .poll_foreground_reservation(&reservation, &cx),
        std::task::Poll::Ready(Err(ManagedRuntimeError::Invalid))
    );
    let prepared = prepare(&f, "reserved-parent");
    let (_, prepared) = foreign
        .manager
        .enroll_foreground(Box::new(prepared), &reservation)
        .unwrap_err();
    let selected = f.manager.enroll_foreground(prepared, &reservation).unwrap();
    assert!(f.manager.foreground_runtime(&selected).is_some());
    assert_eq!(f.manager.reserved_foregrounds(), 0);
    assert_eq!(
        reservation.validate_preparation(),
        Err(ManagedRuntimeError::Invalid)
    );
    assert_eq!(
        f.manager.validate_foreground_reservation(&reservation),
        Err(ManagedRuntimeError::Invalid)
    );
    assert_eq!(
        f.manager.poll_foreground_reservation(&reservation, &cx),
        std::task::Poll::Ready(Err(ManagedRuntimeError::Invalid))
    );
    // A consumed observation no longer pins shutdown or occupies ticket capacity.
    block_on(futures_util::future::poll_fn(|cx| {
        f.manager.poll_shutdown(cx, 2)
    }))
    .unwrap();
}

#[test]
fn pending_candidate_shutdown_waits_for_ticket_release_without_preparing_a_runtime() {
    let mut f = Fixture::new(vec![]);
    f.manager.limits.residents = 1;
    let prepared = prepare(&f, "retained-parent");
    enroll(&mut f, prepared);
    let reservation = f.manager.reserve_foreground().unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    let _ = f.manager.poll_progress(&mut cx, 2);
    assert!(
        f.manager
            .poll_foreground_reservation(&reservation, &cx)
            .is_pending()
    );
    assert!(f.manager.poll_shutdown(&mut cx, 3).is_pending());
    assert_eq!(
        f.manager.validate_foreground_reservation(&reservation),
        Err(ManagedRuntimeError::Unavailable)
    );
    assert_eq!(
        f.manager.poll_foreground_reservation(&reservation, &cx),
        std::task::Poll::Ready(Err(ManagedRuntimeError::Unavailable))
    );
    drop(reservation);
    block_on(futures_util::future::poll_fn(|cx| {
        f.manager.poll_shutdown(cx, 3)
    }))
    .unwrap();
    assert_eq!(f.factory.prepared.load(Ordering::Acquire), 1);
    assert!(f.factory.provider.requests().is_empty());
}

#[test]
fn candidate_pressure_retires_only_one_idle_child_and_waits_for_original_cleanup() {
    let mut f = Fixture::new(vec![]);
    f.manager.limits.residents = 3;
    let prepared = prepare(&f, "retained-parent");
    let foreground = enroll(&mut f, prepared);
    for name in ["first", "second"] {
        assert!(
            f.command(serde_json::json!({"create": {"name": name, "mode": "persistent"}}))
                .ok
        );
    }
    f.drive(|f| f.manager.active.is_none());
    f.manager.limits.work_per_poll = 1;
    let reservation = f.manager.reserve_foreground().unwrap();
    f.drive(|f| f.manager.retiring.len() == 1);
    f.factory.cleanup.store(false, Ordering::Release);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..4 {
        assert!(!matches!(
            f.manager.poll_progress(&mut cx, 101),
            std::task::Poll::Ready(Err(_))
        ));
    }
    assert_eq!(f.manager.children.len(), 1);
    assert_eq!(f.manager.retiring.len(), 1);
    assert!(f.manager.foreground_runtime(&foreground).is_some());
    assert!(
        f.manager
            .poll_foreground_reservation(&reservation, &cx)
            .is_pending()
    );
    f.factory.cleanup.store(true, Ordering::Release);
    f.drive(|f| f.manager.reserved_foregrounds() == 1);
    assert!(f.manager.retiring.is_empty());
    assert_eq!(f.manager.children.len(), 1);
    assert!(!f.manager.has_capacity());
    drop(reservation);
    assert!(f.manager.has_capacity());
    assert!(block_on(f.journal.inspect("child-1".into())).is_ok());
}

#[test]
fn granted_preparation_custody_prevents_empty_manager_shutdown_receipt() {
    let mut f = Fixture::new(vec![]);
    let reservation = reserve(&mut f);
    assert_eq!(f.manager.reserved_foregrounds(), 1);
    assert!(f.manager.foregrounds.is_empty());
    assert_eq!(reservation.validate_preparation(), Ok(()));
    assert!(
        f.manager
            .poll_shutdown(&mut Context::from_waker(Waker::noop()), 2)
            .is_pending()
    );
    assert_eq!(
        reservation.validate_preparation(),
        Err(ManagedRuntimeError::Unavailable)
    );
    assert_eq!(f.manager.reserved_foregrounds(), 1);
    drop(reservation);
    block_on(futures_util::future::poll_fn(|cx| {
        f.manager.poll_shutdown(cx, 2)
    }))
    .unwrap();
    assert!(f.factory.provider.requests().is_empty());
}

#[test]
fn foreground_retirement_is_allocation_bound_and_waits_for_actual_cleanup() {
    let mut f = Fixture::new(vec![]);
    let a = prepare(&f, "foreground-a");
    let a_weak = Arc::downgrade(&a.runtime);
    let a_principal = a.owner.principal().clone();
    let a = enroll(&mut f, a);
    let b = prepare(&f, "foreground-b");
    let b_principal = b.owner.principal().clone();
    let b = enroll(&mut f, b);
    assert!(f.factory.provider.requests().is_empty());
    f.factory.cleanup.store(false, Ordering::Release);
    assert!(f.manager.retire_foreground(&a));
    assert!(!a_principal.is_live());
    assert!(b_principal.is_live());
    assert!(f.manager.foreground_runtime(&a).is_none());
    assert!(f.manager.foreground_runtime(&b).is_some());
    f.manager
        .poll_foregrounds(&mut Context::from_waker(Waker::noop()))
        .unwrap();
    assert!(
        a_weak.upgrade().is_some(),
        "cleanup retains its original runtime"
    );
    f.factory.cleanup.store(true, Ordering::Release);
    f.manager
        .poll_foregrounds(&mut Context::from_waker(Waker::noop()))
        .unwrap();
    assert!(a_weak.upgrade().is_none());
    assert!(!f.manager.retire_foreground(&a));
    assert!(f.manager.foreground_runtime(&b).is_some());
    assert!(b_principal.is_live());
}

#[test]
fn foreground_next_turn_waits_for_original_run_settlement() {
    let mut f = Fixture::new(vec![completed(), completed()]);
    let prepared = prepare(&f, "foreground");
    let selected = enroll(&mut f, prepared);
    let runtime = f.manager.foreground_runtime(&selected).unwrap().clone();
    runtime.enqueue("first".into()).unwrap();
    let turn = block_on(runtime.start_next(2)).unwrap().unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(f.factory.provider.requests().len(), 1);
    f.factory.cleanup.store(false, Ordering::Release);
    f.manager
        .poll_foregrounds(&mut Context::from_waker(Waker::noop()))
        .unwrap();
    runtime.enqueue("second".into()).unwrap();
    assert!(block_on(runtime.start_next(3)).is_err());
    assert_eq!(f.factory.provider.requests().len(), 1);
    assert_eq!(runtime.status().queued_jobs, 1);
    f.factory.cleanup.store(true, Ordering::Release);
    f.manager
        .poll_foregrounds(&mut Context::from_waker(Waker::noop()))
        .unwrap();
    let turn = block_on(runtime.start_next(4)).unwrap().unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    assert_eq!(f.factory.provider.requests().len(), 2);
}

#[test]
fn shutdown_rejects_enrollment_without_discarding_prepared_resource_custody() {
    let mut f = Fixture::new(vec![]);
    let prepared = prepare(&f, "foreground");
    let original = Arc::downgrade(&prepared.runtime);
    let reservation = reserve(&mut f);
    f.manager.request_shutdown();
    let (error, mut retained) = f
        .manager
        .enroll_foreground(Box::new(prepared), &reservation)
        .unwrap_err();
    assert_eq!(error, ManagedRuntimeError::Unavailable);
    assert!(Arc::ptr_eq(&original.upgrade().unwrap(), &retained.runtime));
    retained.resources.begin_close();
    block_on(futures_util::future::poll_fn(|cx| {
        retained.resources.poll_closed(cx)
    }))
    .unwrap();
    drop(retained);
    assert!(original.upgrade().is_none());
}

#[test]
fn shutdown_cannot_complete_while_foreground_cleanup_is_pending() {
    let mut f = Fixture::new(vec![]);
    let prepared = prepare(&f, "foreground");
    enroll(&mut f, prepared);
    f.factory.cleanup.store(false, Ordering::Release);
    assert!(
        f.manager
            .poll_shutdown(&mut Context::from_waker(Waker::noop()), 2)
            .is_pending()
    );
    f.factory.cleanup.store(true, Ordering::Release);
    block_on(futures_util::future::poll_fn(|cx| {
        f.manager.poll_shutdown(cx, 3)
    }))
    .unwrap();
    assert!(f.manager.foregrounds.is_empty());
}

#[test]
fn shared_residency_pressure_evicts_settled_child_not_the_retained_foreground() {
    let mut f = Fixture::new(vec![]);
    f.manager.limits.residents = 2;
    let prepared = prepare(&f, "foreground");
    let foreground = enroll(&mut f, prepared);
    for name in ["first", "second"] {
        assert!(
            f.command(serde_json::json!({"create": {"name": name, "mode": "persistent"}}))
                .ok
        );
    }
    f.drive(|f| f.manager.active.is_none() && f.manager.retiring.is_empty());
    let children = f.manager.children();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id, "child-2");
    assert!(f.manager.foreground_runtime(&foreground).is_some());
    assert!(block_on(f.journal.inspect("child-1".into())).is_ok());
    assert!(f.factory.provider.requests().is_empty());
}

#[test]
fn foreground_retirement_acknowledges_original_notice_before_clearing_and_retiring_context() {
    let mut f = Fixture::new(vec![completed()]);
    let session = f.notice_session();
    let original = super::delivery::original(&mut f, &session);
    let prepared = f.notified_foreground(session);
    let context = prepared.notice_context.as_ref().unwrap().clone();
    let selected = enroll(&mut f, prepared);
    let runtime = f.manager.foreground_runtime(&selected).unwrap().clone();
    runtime.enqueue("consume original notice".into()).unwrap();
    let turn = block_on(runtime.start_next(2)).unwrap().unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let delivery = context.delivery().unwrap();
    assert_eq!(delivery.originals(), std::slice::from_ref(&original));
    f.manager.retire_foreground(&selected);
    assert!(
        !context.is_retired(),
        "source acknowledgement still needs this context"
    );
    f.drive(|f| f.manager.foregrounds.is_empty() && f.manager.active.is_none());
    assert!(context.is_retired());
    let snapshot = block_on(f.journal.inspect("child-1".into())).unwrap();
    let history = block_on(f.journal.history(snapshot, None, 100)).unwrap();
    assert!(
        history
            .records
            .contains(&crate::managed::store::JournalRecord::NoticeAcknowledged {
                identity: original.identity(),
                target: original.target,
                checkpoint: delivery.checkpoint().clone(),
            })
    );
    assert_eq!(f.factory.provider.requests().len(), 1);
}

#[test]
fn residency_pressure_retains_saved_outbox_without_a_live_delivery_receipt() {
    use crate::managed::prompt_context::NOTICE_OUTBOX_KEY;
    let mut f = Fixture::new(vec![]);
    f.manager.limits.residents = 1;
    assert!(
        f.command(serde_json::json!({"create": {
            "name": "original", "mode": "persistent"
        }}))
        .ok
    );
    f.drive(|f| f.manager.active.is_none());
    let session = f.child_session("child-1");
    let original = Arc::downgrade(&f.manager.children[0].prepared.runtime);
    // Even malformed retained evidence requires explicit repair, not eviction.
    let mut record = session.record();
    record
        .metadata
        .insert(NOTICE_OUTBOX_KEY.into(), serde_json::Value::Null);
    block_on(session.update_metadata(record.revision, record.metadata)).unwrap();
    assert!(f.manager.children[0].prepared.notice_context.is_none());
    assert!(
        f.manager.children[0]
            .prepared
            .runtime
            .notice_cleanup_pending()
    );

    let (_admission, invocation) = f.invocation(serde_json::json!({"create": {
        "name": "replacement", "mode": "persistent"
    }}));
    let requester = f.requester.clone();
    let mut response = requester.execute(invocation, CancellationToken::new());
    assert!(
        response
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    f.drive(|f| f.manager.pending_job.is_some() && f.manager.active.is_none());
    assert!(original.upgrade().is_some());
    assert!(f.manager.retiring.is_empty());
    assert_eq!(f.factory.prepared.load(Ordering::Acquire), 1);

    // Model-free confirmed metadata repair makes the same idle slot reusable.
    let mut record = session.record();
    record.metadata.remove(NOTICE_OUTBOX_KEY);
    block_on(session.update_metadata(record.revision, record.metadata)).unwrap();
    f.drive(|f| {
        f.manager
            .children()
            .iter()
            .any(|child| child.id == "child-2")
    });
    assert!(block_on(response).unwrap().ok);
    f.drive(|f| f.manager.active.is_none() && f.manager.retiring.is_empty());
    assert!(original.upgrade().is_none());
    block_on(f.journal.inspect("child-1".into())).unwrap();
    assert!(f.factory.provider.requests().is_empty());
}

#[test]
fn quiesced_foreground_clears_original_notice_without_reopening_prompt_or_save_admission() {
    let mut f = Fixture::new(vec![completed()]);
    let session = f.notice_session();
    let original = super::delivery::original(&mut f, &session);
    let prepared = f.notified_foreground(session);
    let context = prepared.notice_context.as_ref().unwrap().clone();
    let selected = enroll(&mut f, prepared);
    let runtime = f.manager.foreground_runtime(&selected).unwrap().clone();
    runtime.enqueue("consume notice".into()).unwrap();
    let turn = block_on(runtime.start_next(2)).unwrap().unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    let delivery = context.delivery().unwrap();
    let mut guard = f.manager.quiesce_foreground(&selected).unwrap();
    assert!(guard.notice_cleanup_pending());
    assert_eq!(
        guard.try_retire(),
        Err(crate::NativeConversationRuntimeError::Busy)
    );
    assert!(runtime.enqueue("not admitted".into()).is_err());
    assert!(block_on(runtime.rename("not saved", 3)).is_err());
    assert!(block_on(runtime.clear_notice_delivery(&delivery)).is_err());
    f.drive(|f| !context.has_pending_delivery() && f.manager.active.is_none());
    assert!(!guard.notice_cleanup_pending());
    assert!(runtime.enqueue("still not admitted".into()).is_err());
    assert!(block_on(runtime.rename("still not saved", 4)).is_err());
    let snapshot = block_on(f.journal.inspect("child-1".into())).unwrap();
    let history = block_on(f.journal.history(snapshot, None, 100)).unwrap();
    assert!(
        history
            .records
            .contains(&crate::managed::store::JournalRecord::NoticeAcknowledged {
                identity: original.identity(),
                target: original.target,
                checkpoint: delivery.checkpoint().clone(),
            })
    );
    guard.try_retire().unwrap();
    f.manager.retire_foreground(&selected);
    f.drive(|f| f.manager.foregrounds.is_empty());
    assert!(context.is_retired());
    assert_eq!(f.factory.provider.requests().len(), 1);
}

#[test]
fn dropped_quiescence_restores_original_notice_cleanup_without_reviving_stale_selection() {
    let mut f = Fixture::new(vec![completed()]);
    let session = f.notice_session();
    super::delivery::original(&mut f, &session);
    let prepared = f.notified_foreground(session);
    let context = prepared.notice_context.as_ref().unwrap().clone();
    let selected = enroll(&mut f, prepared);
    let runtime = f.manager.foreground_runtime(&selected).unwrap().clone();
    runtime.enqueue("consume notice".into()).unwrap();
    let turn = block_on(runtime.start_next(2)).unwrap().unwrap();
    assert!(block_on(turn.collect::<Vec<_>>()).iter().all(Result::is_ok));
    drop(f.manager.quiesce_foreground(&selected).unwrap());
    f.drive(|f| !context.has_pending_delivery() && f.manager.active.is_none());
    block_on(runtime.rename("reopened admission", 3)).unwrap();
    let mut next = f.manager.quiesce_foreground(&selected).unwrap();
    next.try_retire().unwrap();
    f.manager.retire_foreground(&selected);
    f.drive(|f| f.manager.foregrounds.is_empty());
    assert!(f.manager.quiesce_foreground(&selected).is_err());
    assert_eq!(f.factory.provider.requests().len(), 1);
}
