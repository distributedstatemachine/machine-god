use crate::interactive_session::clipboard::{
    NativeInteractiveCopyError as Error, NativeInteractiveCopyReceipt as Receipt, Phase,
};
use crate::interactive_session::tests::{Gate, close, deferred, executor, outcome, owner, support};
use crate::interactive_session::{
    NativeInteractiveControl, NativeInteractiveError, NativeInteractiveOutcome,
    NativeInteractiveSession, NativeInteractiveTransition,
};
use crate::{NativeClipboard, NativeClipboardError, NativeClipboardExecutable};
use futures_util::{StreamExt, future::poll_fn};
use machine_god_core::{BackgroundOutputOwner, TurnEvent};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};
use support::Fixture;

async fn copied(
    session: &mut NativeInteractiveSession,
) -> crate::interactive_session::NativeInteractiveCopyOutcome {
    tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 300);
            session
                .take_copy_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}

async fn answer(fixture: &Fixture, session: &mut NativeInteractiveSession) {
    let now_ms = crate::NativeSessionMetadata::from_metadata(&session.runtime().record().metadata)
        .unwrap()
        .updated_at_ms()
        .unwrap_or(100)
        + 1;
    fixture.transport.push(support::answer());
    session.enqueue("answer".into()).unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, now_ms);
            if session.take_presentation().is_some() {
                cx.waker().wake_by_ref();
            }
            session.take_outcome().map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap();
    assert!(
        matches!(result, NativeInteractiveOutcome::Turn(Ok(event)) if matches!(event.payload, TurnEvent::Completed { .. }))
    );
}

fn backend(fixture: &Fixture, session: &mut NativeInteractiveSession, script: &str) {
    let path = fixture.workspace.join("clipboard-fixture");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    session.clipboard = NativeClipboard::new(
        NativeClipboardExecutable::new(&path, fs::File::open(&path).unwrap()).unwrap(),
        fixture.workspace.clone(),
        Vec::new(),
        fixture.host.control_workers().unwrap(),
    );
}

fn pending(session: &mut NativeInteractiveSession, gate: &Arc<Gate>) -> BackgroundOutputOwner {
    session.request_copy().unwrap();
    let copy = session.copy.as_mut().unwrap();
    let cancel = copy.cancel.clone();
    let wait = deferred((), gate);
    copy.phase = Phase::Copying(Box::pin(async move {
        wait.await;
        if cancel.is_cancelled() {
            Err(NativeClipboardError::Cancelled)
        } else {
            Ok(())
        }
    }));
    copy.source.clone()
}

#[test]
fn empty_is_success_without_backend_and_unread_receipt_reserves_only_copy_lane() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let record = session.runtime().record();
        let id = session.request_copy().unwrap();
        assert_eq!(id.get(), 1);
        assert!(matches!(
            session.request_copy(),
            Err(NativeInteractiveError::Busy)
        ));
        let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 200);
        assert!(!session.has_pending_copy());
        assert!(matches!(
            session.request_copy(),
            Err(NativeInteractiveError::Busy)
        ));
        assert_eq!(session.runtime().record(), record);
        // The unread copy receipt does not prevent actual provider admission.
        answer(&fixture, &mut session).await;
        assert_eq!(
            session.take_copy_outcome().unwrap().result,
            Ok(Receipt::Empty)
        );
        assert_eq!(session.request_copy().unwrap().get(), 2);
        assert_eq!(
            copied(&mut session).await.result,
            Err(Error::Clipboard(NativeClipboardError::Unavailable))
        );
        close(session, fixture).await;
    });
}

#[test]
fn blocked_clipboard_does_not_stall_model_or_metadata_control() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let gate = Arc::new(Gate::default());
        pending(&mut session, &gate);
        let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 200);
        session
            .request_control(
                NativeInteractiveControl::Rename {
                    title: "renamed".into(),
                },
                201,
            )
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = session.poll_progress(cx, 202);
                session
                    .take_control_outcome()
                    .map_or(Poll::Pending, |result| {
                        assert!(result.result.is_ok());
                        Poll::Ready(())
                    })
            }),
        )
        .await
        .unwrap();
        answer(&fixture, &mut session).await;
        assert!(session.has_pending_copy());
        assert!(gate.polls.load(Ordering::SeqCst) > 1);
        assert!(!session.request_cancel());
        assert!(!session.copy.as_ref().unwrap().cancel.is_cancelled());
        gate.release();
        assert_eq!(copied(&mut session).await.result, Ok(Receipt::Copied));
        close(session, fixture).await;
    });
}

#[test]
fn transition_cancels_started_copy_without_waiting_or_retargeting_source() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let gate = Arc::new(Gate::default());
        let source = pending(&mut session, &gate);
        let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 200);
        session
            .request_transition(NativeInteractiveTransition::New, 201)
            .unwrap();
        assert!(session.copy.as_ref().unwrap().cancel.is_cancelled());
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        assert_ne!(
            crate::interactive_session::transition::principal(session.runtime()),
            source
        );
        assert!(session.has_pending_copy());
        answer(&fixture, &mut session).await;
        gate.release();
        let receipt = copied(&mut session).await;
        assert_eq!(receipt.source, source);
        assert_eq!(receipt.result, Err(Error::Cancelled));
        close(session, fixture).await;
    });
}

#[test]
fn shutdown_settles_before_copy_response_and_preserves_that_response_after_closure() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let gate = Arc::new(Gate::default());
        let source = pending(&mut session, &gate);
        let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 200);
        session.request_shutdown();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Shutdown
        ));
        assert!(session.is_closed());
        assert!(session.has_pending_copy());
        gate.release();
        let receipt = copied(&mut session).await;
        assert_eq!(receipt.source, source);
        assert_eq!(receipt.result, Err(Error::Cancelled));
        assert!(!session.has_pending_copy());
        drop(session);
        fixture.finish();
    });
}

#[test]
fn selection_is_discarded_on_transition_and_shutdown_without_backend_poll() {
    executor().block_on(async {
        for shutdown in [false, true] {
            let fixture = Fixture::new();
            let mut session = owner(&fixture).await;
            answer(&fixture, &mut session).await;
            backend(&fixture, &mut session, "#!/bin/sh\n/bin/cat > copied\n");
            session.request_copy().unwrap();
            if shutdown {
                session.request_shutdown();
            } else {
                session
                    .request_transition(NativeInteractiveTransition::New, 201)
                    .unwrap();
            }
            assert!(!session.has_pending_copy());
            assert_eq!(
                session.take_copy_outcome().unwrap().result,
                Err(Error::Cancelled)
            );
            assert!(!fixture.workspace.join("copied").exists());
            if shutdown {
                assert!(matches!(
                    outcome(&mut session).await,
                    NativeInteractiveOutcome::Shutdown
                ));
                drop(session);
                fixture.finish();
            } else {
                assert!(matches!(
                    outcome(&mut session).await,
                    NativeInteractiveOutcome::Transition(_)
                ));
                close(session, fixture).await;
            }
        }
    });
}

#[test]
fn actual_explicit_backend_copies_accepted_snapshot_and_keeps_failure_receipt() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        answer(&fixture, &mut session).await;
        backend(&fixture, &mut session, "#!/bin/sh\n/bin/cat > copied\n");
        session.request_copy().unwrap();
        let source = crate::interactive_session::transition::principal(session.runtime());
        // Publish a different real answer before the copy's very first poll.
        // The runtime can progress independently of presentation scheduling.
        fixture.transport.push(String::from_utf8(support::answer()).unwrap()
            .replace("complete", "newer reply").into_bytes());
        session.enqueue("new answer".into()).unwrap();
        let runtime = session.runtime().clone();
        let mut turn = runtime.start_next(250).await.unwrap().unwrap();
        while let Some(event) = turn.next().await { event.unwrap(); }
        drop(turn);
        drop(runtime);
        assert!(session.runtime().record().messages.iter().any(|message| {
            message.content.iter().any(|block| matches!(block, machine_god_core::ContentBlock::Text { text } if text == "newer reply"))
        }));
        let before = session.runtime().record();
        // Mutate no canonical state to obtain the reply: actual backend owns I/O.
        let receipt = copied(&mut session).await;
        assert_eq!(receipt.result, Ok(Receipt::Copied));
        assert_eq!(receipt.source, source);
        assert_eq!(
            fs::read(fixture.workspace.join("copied")).unwrap(),
            b"complete"
        );
        assert_eq!(session.runtime().record(), before);
        backend(&fixture, &mut session, "#!/bin/sh\nexit 7\n");
        session.request_copy().unwrap();
        let receipt = copied(&mut session).await;
        assert!(matches!(
            receipt.result,
            Err(Error::Clipboard(
                NativeClipboardError::ExitFailed | NativeClipboardError::WriteFailed
            ))
        ));
        close(session, fixture).await;
    });
}

#[test]
fn actual_started_child_cancellation_settles_response_and_full_host_join() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        answer(&fixture, &mut session).await;
        backend(
            &fixture,
            &mut session,
            "#!/bin/sh\nprintf started > started\nwhile :; do :; done\n",
        );
        session.request_copy().unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !fixture.workspace.join("started").exists() {
                let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 200);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        session.request_shutdown();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Shutdown
        ));
        assert_eq!(copied(&mut session).await.result, Err(Error::Cancelled));
        assert!(!session.has_pending_copy());
        drop(session);
        // This is the real composed host scope, not a separate test executor's
        // scope. Its completion waits actual clipboard worker/child cleanup.
        fixture.finish();
    });
}

#[test]
fn turn_cancellation_does_not_cancel_pending_clipboard() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let gate = Arc::new(Gate::default());
        pending(&mut session, &gate);
        fixture.transport.push(support::answer());
        session.enqueue("cancel current turn".into()).unwrap();
        let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 200);
        assert!(session.request_cancel());
        assert!(!session.copy.as_ref().unwrap().cancel.is_cancelled());
        let _ = outcome(&mut session).await;
        gate.release();
        assert_eq!(copied(&mut session).await.result, Ok(Receipt::Copied));
        close(session, fixture).await;
    });
}

#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn empty_takes_are_inert_and_copy_identity_exhaustion_never_reserves_work() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let counter = Arc::new(Wakes::default());
        let waker = Waker::from(counter.clone());
        assert!(
            session
                .poll_progress(&mut Context::from_waker(&waker), 200)
                .is_pending()
        );
        assert!(session.take_copy_outcome().is_none());
        assert_eq!(counter.0.load(Ordering::SeqCst), 0);
        session.next_copy = u64::MAX;
        assert!(matches!(
            session.request_copy(),
            Err(NativeInteractiveError::IdentityExhausted)
        ));
        assert!(!session.has_pending_copy());
        assert_eq!(session.next_copy, u64::MAX);
        close(session, fixture).await;
    });
}

#[test]
fn optional_invalid_clipboard_configuration_does_not_fail_startup_or_empty_copy() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let executable = std::env::current_exe().unwrap();
        let options = crate::NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            crate::NativeModelPreferences::new(
                "workspace/default",
                crate::NativeReasoningEffort::default(),
                false,
            )
            .unwrap(),
        )
        .unwrap()
        .with_clipboard(
            NativeClipboardExecutable::new(&executable, fs::File::open(&executable).unwrap())
                .unwrap(),
            vec![
                ("DUPLICATE".into(), "one".into()),
                ("DUPLICATE".into(), "two".into()),
            ],
        );
        let mut session = NativeInteractiveSession::open(
            fixture.host.clone(),
            options,
            crate::NativeInteractiveInitialSession::Fresh,
            100,
        )
        .await
        .unwrap();
        session.request_copy().unwrap();
        assert_eq!(copied(&mut session).await.result, Ok(Receipt::Empty));
        answer(&fixture, &mut session).await;
        session.request_copy().unwrap();
        assert_eq!(
            copied(&mut session).await.result,
            Err(Error::Clipboard(NativeClipboardError::InvalidAuthority))
        );
        close(session, fixture).await;
    });
}
