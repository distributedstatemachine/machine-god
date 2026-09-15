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
    SessionId, SessionIncarnationId,
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

#[test]
fn foreground_retirement_is_allocation_bound_and_waits_for_actual_cleanup() {
    let mut f = Fixture::new(vec![]);
    let a = prepare(&f, "foreground-a");
    let a_weak = Arc::downgrade(&a.runtime);
    let a_principal = a.owner.principal().clone();
    let a = f.manager.enroll_foreground(Box::new(a)).unwrap();
    let b = prepare(&f, "foreground-b");
    let b_principal = b.owner.principal().clone();
    let b = f.manager.enroll_foreground(Box::new(b)).unwrap();
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
    let selected = f.manager.enroll_foreground(Box::new(prepared)).unwrap();
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
    f.manager.request_shutdown();
    let (error, mut retained) = f.manager.enroll_foreground(Box::new(prepared)).unwrap_err();
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
    f.manager.enroll_foreground(Box::new(prepared)).unwrap();
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
    let foreground = f.manager.enroll_foreground(Box::new(prepared)).unwrap();
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
    let selected = f.manager.enroll_foreground(Box::new(prepared)).unwrap();
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
