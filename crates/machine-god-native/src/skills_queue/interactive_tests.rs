//! Fresh-helper composed owner scenarios; run with the integrated release host.
use super::{Fixture, close, executor, outcome, owner};
use crate::{
    NativeInteractiveControl, NativeInteractiveError, NativeInteractiveOutcome, NativeSkillSnapshot,
};
use futures_util::future::poll_fn;
use machine_god_core::CancellationToken;
use std::{
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

async fn snapshot(
    session: &mut crate::NativeInteractiveSession,
    fixture: &Fixture,
) -> NativeSkillSnapshot {
    session
        .request_control(
            NativeInteractiveControl::Skills {
                command: "create chosen".parse().unwrap(),
            },
            110,
        )
        .unwrap();
    let receipt = poll_fn(|cx| {
        let _ = session.poll_progress(cx, 110);
        session
            .take_control_outcome()
            .map_or(Poll::Pending, Poll::Ready)
    })
    .await;
    assert!(!receipt.failed());
    let catalog = Arc::clone(fixture.host.skills().unwrap().catalog());
    fixture
        .host
        .control_workers()
        .unwrap()
        .run(move || catalog.discover(&CancellationToken::new()).unwrap())
        .await
        .unwrap()
}

#[test]
fn cancel_requested_before_first_admission_poll_signals_new_materializer_immediately() {
    executor().block_on(async {
        let fixture = Fixture::new_with_skills();
        let mut session = owner(&fixture).await;
        let snapshot = snapshot(&mut session, &fixture).await;
        let queued = session
            .enqueue_with_skills("$chosen".into(), &snapshot, &[])
            .unwrap();
        let (entered, observe) = std::sync::mpsc::sync_channel(1);
        let (release, wait) = std::sync::mpsc::sync_channel(1);
        session
            .runtime()
            .set_skill_queue_test_hook(queued.queued_id, move |token| {
                entered.send(token.clone()).unwrap();
                wait.recv_timeout(Duration::from_secs(10)).unwrap();
            });
        let runtime = session.runtime().clone();
        session.admission = Some(Box::pin(async move { runtime.start_next(120).await }));
        assert!(session.request_cancel());
        let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 120);
        let token = observe.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            token.is_cancelled(),
            "do not require a worker wake to deliver cancellation"
        );
        release.send(()).unwrap();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Turn(Err(NativeInteractiveError::Runtime(
                crate::NativeConversationRuntimeError::Skills(_)
            )))
        ));
        assert!(fixture.transport.requests().is_empty());
        close(session, fixture).await;
    });
}

#[test]
fn enqueue_facade_rejects_missing_service_without_retaining_input() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let catalog = crate::NativeSkillCatalog::new(vec![]).unwrap();
        let snapshot = catalog.discover(&CancellationToken::new()).unwrap();
        assert!(matches!(
            session.enqueue_with_skills("plain".into(), &snapshot, &[]),
            Err(NativeInteractiveError::Configuration)
        ));
        assert_eq!(session.runtime().status().queued_jobs, 0);
        close(session, fixture).await;
    });
}
