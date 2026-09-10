use super::super::{Gate, deferred};
use super::{
    Control, Fixture, NativeInteractiveOutcome, NativeInteractiveSession, Receipt, close,
    control_outcome, executor, owner, poll_once, support,
};
use crate::{NativeBackgroundCommand, NativeBackgroundControlReceipt, NativeInteractiveControlId};
use futures_util::future::poll_fn;
use machine_god_core::{CancellationToken, StopReason, TurnEvent};
use std::{
    sync::{Arc, atomic::Ordering},
    task::Poll,
    time::Duration,
};

/// Retain a real background result at the delivery boundary. Cancellation may
/// signal its token but must not drop the owned future or replace its receipt.
async fn pending_background(
    session: &mut NativeInteractiveSession,
) -> (NativeInteractiveControlId, CancellationToken, Arc<Gate>) {
    let id = session
        .request_control(
            Control::Background {
                command: NativeBackgroundCommand::List,
            },
            200,
        )
        .unwrap();
    let mut control = session.control.take().unwrap();
    let token = control.cancellation.as_ref().unwrap().clone();
    let result = control.future.await;
    assert!(matches!(
        result,
        Ok(Receipt::Background(NativeBackgroundControlReceipt::Listed(
            _
        )))
    ));
    let gate = Arc::new(Gate::default());
    control.future = deferred(result, &gate);
    session.control = Some(control);
    assert!(poll_once(session).is_pending());
    (id, token, gate)
}

async fn turn_reason(session: &mut NativeInteractiveSession) -> StopReason {
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 300);
            let _ = session.take_presentation();
            session.take_outcome().map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap();
    let NativeInteractiveOutcome::Turn(Ok(event)) = result else {
        panic!("settled turn receipt");
    };
    let TurnEvent::Completed { reason, .. } = event.payload else {
        panic!("completed turn");
    };
    reason
}

async fn listed_receipt(session: &mut NativeInteractiveSession, id: NativeInteractiveControlId) {
    let receipt = control_outcome(session).await;
    assert_eq!(receipt.id, id);
    assert!(matches!(
        receipt.result,
        Ok(Receipt::Background(NativeBackgroundControlReceipt::Listed(
            _
        )))
    ));
}

#[test]
fn background_cancel_also_cancels_active_turn_and_preserves_queued_input() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let original = session.runtime().clone();
        session.enqueue("current turn".into()).unwrap();
        let turn = original.start_next(200).await.unwrap().unwrap();
        let handle = turn.handle().unwrap();
        session.turn = Some(turn);
        session.enqueue("later queued turn".into()).unwrap();
        let (id, token, gate) = pending_background(&mut session).await;

        assert!(session.request_cancel());
        assert!(token.is_cancelled());
        assert!(poll_once(&mut session).is_pending());
        assert!(
            !handle.is_cancelled(),
            "control settlement still owns its lane"
        );
        assert_eq!(gate.drops.load(Ordering::SeqCst), 0);
        assert!(session.control.is_some());
        gate.release();
        listed_receipt(&mut session, id).await;
        assert!(
            handle.is_cancelled(),
            "cancel must also reach the active turn"
        );
        assert_eq!(turn_reason(&mut session).await, StopReason::Cancelled);
        assert_eq!(gate.drops.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&original, session.runtime()));
        assert_eq!(original.status().queued_jobs, 1);
        assert!(fixture.transport.requests().is_empty());

        fixture.transport.push(support::answer());
        assert_ne!(turn_reason(&mut session).await, StopReason::Cancelled);
        assert_eq!(fixture.transport.requests().len(), 1);
        assert_eq!(original.status().queued_jobs, 0);
        drop(original);
        close(session, fixture).await;
    });
}

#[test]
fn background_cancel_survives_pending_admission_settlement() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        session.enqueue("admitting turn".into()).unwrap();
        session.enqueue("later queued turn".into()).unwrap();
        let admission_gate = Arc::new(Gate::default());
        let gate = admission_gate.clone();
        let runtime = session.runtime().clone();
        session.admission = Some(Box::pin(async move {
            let result = runtime.start_next(200).await;
            deferred(result, &gate).await
        }));
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = session.poll_progress(cx, 300);
                if admission_gate.polls.load(Ordering::SeqCst) > 0 {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        let (id, token, control_gate) = pending_background(&mut session).await;

        assert!(session.request_cancel());
        assert!(token.is_cancelled());
        control_gate.release();
        listed_receipt(&mut session, id).await;
        assert!(session.admission.is_some());
        assert_eq!(admission_gate.drops.load(Ordering::SeqCst), 0);
        // A scripted response makes a lost cancellation settle normally, so the
        // regression fails on the actual turn receipt rather than a timeout.
        fixture.transport.push(support::answer());
        admission_gate.release();
        assert_eq!(turn_reason(&mut session).await, StopReason::Cancelled);
        assert_eq!(admission_gate.drops.load(Ordering::SeqCst), 1);
        assert_eq!(session.runtime().status().queued_jobs, 1);
        assert!(fixture.transport.requests().is_empty());
        assert_ne!(turn_reason(&mut session).await, StopReason::Cancelled);
        assert_eq!(fixture.transport.requests().len(), 1);
        close(session, fixture).await;
    });
}

#[test]
fn background_only_cancel_retains_receipt_without_cancelling_queued_turn() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        assert!(!session.request_cancel());
        session.enqueue("still queued".into()).unwrap();
        let (id, token, gate) = pending_background(&mut session).await;
        assert!(session.request_cancel());
        assert!(token.is_cancelled());
        assert!(poll_once(&mut session).is_pending());
        assert_eq!(gate.drops.load(Ordering::SeqCst), 0);
        gate.release();
        listed_receipt(&mut session, id).await;
        assert!(!session.request_cancel());
        assert_eq!(session.runtime().status().queued_jobs, 1);
        fixture.transport.push(support::answer());
        assert_ne!(turn_reason(&mut session).await, StopReason::Cancelled);
        assert_eq!(fixture.transport.requests().len(), 1);
        close(session, fixture).await;
    });
}

#[test]
fn background_cancel_before_first_poll_retains_cancelled_receipt() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        session.enqueue("still queued".into()).unwrap();
        let id = session
            .request_control(
                Control::Background {
                    command: NativeBackgroundCommand::List,
                },
                200,
            )
            .unwrap();
        assert!(session.request_cancel());
        assert!(session.control.is_some());
        let receipt = control_outcome(&mut session).await;
        assert_eq!(receipt.id, id);
        assert!(matches!(
            receipt.result,
            Err(super::ControlError::Background(
                crate::NativeBackgroundControlError::Terminal(
                    crate::NativeTerminalBackgroundError::Cancelled
                )
            ))
        ));
        assert_eq!(session.runtime().status().queued_jobs, 1);
        fixture.transport.push(support::answer());
        assert_ne!(turn_reason(&mut session).await, StopReason::Cancelled);
        assert_eq!(fixture.transport.requests().len(), 1);
        close(session, fixture).await;
    });
}

#[test]
fn background_cancel_rejects_shutdown_and_closed_owners_without_losing_receipt() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        session
            .enqueue("discarded only by shutdown".into())
            .unwrap();
        let (id, token, gate) = pending_background(&mut session).await;
        session.request_shutdown();
        assert!(token.is_cancelled());
        assert!(!session.request_cancel());
        assert!(poll_once(&mut session).is_pending());
        assert_eq!(gate.drops.load(Ordering::SeqCst), 0);
        gate.release();
        assert!(matches!(
            super::outcome(&mut session).await,
            NativeInteractiveOutcome::Shutdown
        ));
        assert!(session.is_closed());
        assert!(!session.request_cancel());
        listed_receipt(&mut session, id).await;
        assert_eq!(gate.drops.load(Ordering::SeqCst), 1);
        assert_eq!(session.runtime().status().queued_jobs, 0);
        assert!(fixture.transport.requests().is_empty());
        drop(session);
        fixture.finish();
    });
}
