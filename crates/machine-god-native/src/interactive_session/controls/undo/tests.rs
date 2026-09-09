use crate::interactive_session::controls::undo::run;
use crate::interactive_session::tests::{Gate, close, executor, outcome, owner, support};
use crate::interactive_session::{
    NativeInteractiveControl as Control, NativeInteractiveControlOutcome, NativeInteractiveOutcome,
    NativeInteractiveSession, NativeInteractiveTransition,
};
use crate::interactive_session::{
    NativeInteractiveControlError as Error, NativeInteractiveControlReceipt as Receipt,
};
use crate::{FileUndoError, FileUndoOutcome};
use crate::{FileUndoUnavailableReason, NativeConversationRuntimePhase};
use futures_util::future::poll_fn;
use machine_god_core::CancellationToken;
use machine_god_core::TurnEvent;
use serde_json::{Value, json};
use std::sync::Arc;
use std::{
    fs,
    future::Future,
    os::fd::AsFd,
    task::{Context, Poll, Waker},
};
use support::Fixture;

async fn receipt(session: &mut NativeInteractiveSession) -> NativeInteractiveControlOutcome {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 300);
            session
                .take_control_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}

async fn tool(fixture: &Fixture, session: &mut NativeInteractiveSession, name: &str, args: Value) {
    fixture.transport.push(support::call(name, &args));
    fixture.transport.push(support::answer());
    session
        .enqueue("perform scripted file operation".into())
        .unwrap();
    let mut observed = false;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 200);
            if let Some(event) = session.take_presentation() {
                if let TurnEvent::ToolFinished { output, .. } = event.payload {
                    assert!(!output.is_error, "{output:?}");
                    observed = true;
                }
                cx.waker().wake_by_ref();
            }
            session.take_outcome().map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap();
    assert!(observed);
    assert!(
        matches!(result, NativeInteractiveOutcome::Turn(Ok(event)) if matches!(event.payload, TurnEvent::Completed { .. }))
    );
}

async fn undo(session: &mut NativeInteractiveSession) -> Result<FileUndoOutcome, Error> {
    let before = session.runtime().record();
    let id = session.request_control(Control::UndoLast, 300).unwrap();
    assert_eq!(session.runtime().record(), before);
    let result = receipt(session).await;
    assert_eq!(result.id, id);
    assert_eq!(session.runtime().record(), before);
    result.result.map(|value| match value {
        Receipt::Undone(outcome) => outcome,
        other => panic!("unexpected receipt: {other:?}"),
    })
}

#[test]
fn all_five_real_tools_undo_in_order_without_transcript_mutation() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        assert_eq!(undo(&mut session).await.unwrap(), FileUndoOutcome::Empty);
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"source","content":"first"}),
        )
        .await;
        tool(
            &fixture,
            &mut session,
            "edit_file",
            json!({"path":"source","old_string":"first","new_string":"second"}),
        )
        .await;
        tool(
            &fixture,
            &mut session,
            "copy_file",
            json!({"source":"source","destination":"copy"}),
        )
        .await;
        tool(
            &fixture,
            &mut session,
            "rename_file",
            json!({"old_path":"copy","new_path":"renamed"}),
        )
        .await;
        tool(
            &fixture,
            &mut session,
            "delete_file",
            json!({"path":"renamed"}),
        )
        .await;
        assert_eq!(
            undo(&mut session).await.unwrap(),
            FileUndoOutcome::Restored("renamed".into())
        );
        assert_eq!(
            fs::read(fixture.workspace.join("renamed")).unwrap(),
            b"second"
        );
        assert_eq!(
            undo(&mut session).await.unwrap(),
            FileUndoOutcome::Restored("copy".into())
        );
        assert!(!fixture.workspace.join("renamed").exists());
        assert_eq!(
            undo(&mut session).await.unwrap(),
            FileUndoOutcome::Removed("copy".into())
        );
        assert_eq!(
            undo(&mut session).await.unwrap(),
            FileUndoOutcome::Restored("source".into())
        );
        assert_eq!(
            fs::read(fixture.workspace.join("source")).unwrap(),
            b"first"
        );
        assert_eq!(
            undo(&mut session).await.unwrap(),
            FileUndoOutcome::Removed("source".into())
        );
        assert!(!fixture.workspace.join("source").exists());
        assert_eq!(undo(&mut session).await.unwrap(), FileUndoOutcome::Empty);
        close(session, fixture).await;
    });
}

#[test]
fn busy_changed_nonundoable_and_ambiguous_are_not_empty_or_retried() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let reserved = fixture.undo.reserve_clear().unwrap();
        assert!(matches!(
            undo(&mut session).await,
            Err(Error::Undo(FileUndoError::Busy))
        ));
        drop(reserved);
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"changed","content":"tracked"}),
        )
        .await;
        fs::write(fixture.workspace.join("changed"), b"external").unwrap();
        assert!(matches!(
            undo(&mut session).await,
            Err(Error::Undo(FileUndoError::Changed))
        ));
        assert_eq!(
            fs::read(fixture.workspace.join("changed")).unwrap(),
            b"external"
        );
        fixture.undo.clear().unwrap();
        let large = fs::File::create(fixture.workspace.join("large")).unwrap();
        large
            .set_len(u64::try_from(crate::MAX_FILE_UNDO_PREIMAGE_BYTES).unwrap() + 1)
            .unwrap();
        drop(large);
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"large","content":"replacement"}),
        )
        .await;
        assert!(matches!(
            undo(&mut session).await,
            Err(Error::Undo(FileUndoError::NotUndoable(
                FileUndoUnavailableReason::PreimageTooLarge
            )))
        ));
        assert_eq!(
            fs::read(fixture.workspace.join("large")).unwrap(),
            b"replacement"
        );
        fixture.undo.clear().unwrap();
        let root = fs::File::open(&fixture.workspace).unwrap();
        // Establish the same uncertainty barrier as interrupted native publication.
        let mut publication = fixture
            .undo
            .begin(
                root.as_fd(),
                crate::file_undo::Operation::Replace("uncertain"),
                &CancellationToken::new(),
            )
            .unwrap();
        publication.uncertain();
        drop(publication);
        assert!(matches!(
            undo(&mut session).await,
            Err(Error::Undo(FileUndoError::Ambiguous))
        ));
        assert!(matches!(
            undo(&mut session).await,
            Err(Error::Undo(FileUndoError::Ambiguous))
        ));
        close(session, fixture).await;
    });
}

#[test]
fn active_turn_and_occupied_presentation_do_not_block_undo_or_cancel_turn() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        fixture.transport.push(support::call(
            "write_file",
            &json!({"path":"active","content":"created"}),
        ));
        fixture.transport.push(support::answer());
        session.enqueue("write then answer".into()).unwrap();
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 200);
            if session
                .presentation
                .as_ref()
                .is_some_and(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
            {
                return Poll::Ready(());
            }
            if session.take_presentation().is_some() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        })
        .await;
        assert!(session.runtime().status().active);
        let before = session.runtime().record();
        assert_eq!(
            undo(&mut session).await.unwrap(),
            FileUndoOutcome::Removed("active".into())
        );
        assert!(session.runtime().status().active);
        assert_eq!(session.runtime().record(), before);
        assert!(session.presentation.is_some());
        assert!(!session.cancel_requested);
        close(session, fixture).await;
    });
}

fn paused_inverse(
    session: &mut NativeInteractiveSession,
    fixture: &Fixture,
) -> (Arc<Gate>, std::sync::mpsc::SyncSender<()>) {
    session.request_control(Control::UndoLast, 300).unwrap();
    let entered = Arc::new(Gate::default());
    let signal = entered.clone();
    let tracker = fixture.undo.clone();
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    session.control.as_mut().unwrap().future = run(
        session.runtime().clone(),
        fixture.host.control_workers().unwrap(),
        move || {
            signal.release();
            let _ = wait.recv();
            tracker.undo_last(&CancellationToken::new())
        },
    );
    (entered, release)
}

async fn wait_entered(session: &mut NativeInteractiveSession, entered: &Gate) {
    poll_fn(|cx| {
        entered.wake.register(cx.waker());
        let _ = session.poll_progress(cx, 300);
        if entered.ready.load(std::sync::atomic::Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

#[test]
fn actual_worker_retains_admission_after_outer_poll_and_response_are_dropped() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"owned","content":"created"}),
        )
        .await;
        let runtime = session.runtime().clone();
        let (entered, release) = paused_inverse(&mut session, &fixture);
        wait_entered(&mut session, &entered).await;
        let mut outer = Box::pin(poll_fn(|cx| session.poll_progress(cx, 300)));
        let _ = outer.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        drop(outer);
        let mut fence = runtime.begin_quiescence().unwrap();
        assert!(fence.try_retire().is_err());
        // Even abandoning the owner/response cannot release the worker's permit.
        drop(session);
        assert!(fence.try_retire().is_err());
        release.send(()).unwrap();
        fence.wait_idle().await.unwrap();
        fence.try_retire().unwrap();
        assert_eq!(
            runtime.status().phase,
            NativeConversationRuntimePhase::Retired
        );
        assert!(!fixture.workspace.join("owned").exists());
        drop(fence);
        drop(runtime);
        fixture.finish();
    });
}

#[test]
fn pending_inverse_settles_before_transition_and_shutdown_with_exact_receipt() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"pending","content":"created"}),
        )
        .await;
        let (entered, release) = paused_inverse(&mut session, &fixture);
        wait_entered(&mut session, &entered).await;
        session.enqueue("never admitted".into()).unwrap();
        session
            .request_transition(NativeInteractiveTransition::New, 400)
            .unwrap();
        session.request_shutdown();
        let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 400);
        assert!(session.transition.is_none());
        assert!(fixture.workspace.join("pending").exists());
        release.send(()).unwrap();
        assert!(matches!(
            receipt(&mut session).await.result,
            Ok(Receipt::Undone(FileUndoOutcome::Removed(_)))
        ));
        let _ = outcome(&mut session).await;
        if !session.is_closed() {
            let _ = outcome(&mut session).await;
        }
        assert!(session.is_closed());
        assert!(!fixture.workspace.join("pending").exists());
        assert_eq!(fixture.transport.requests().len(), 2);
        drop(session);
        fixture.finish();
    });
}

#[test]
fn failed_undo_preserves_both_receipts_and_does_not_start_transition() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"changed","content":"tracked"}),
        )
        .await;
        let current = session.runtime().clone();
        let (entered, release) = paused_inverse(&mut session, &fixture);
        wait_entered(&mut session, &entered).await;
        fs::write(fixture.workspace.join("changed"), b"external").unwrap();
        session
            .request_transition(NativeInteractiveTransition::Reset, 400)
            .unwrap();
        release.send(()).unwrap();
        assert!(matches!(
            receipt(&mut session).await.result,
            Err(Error::Undo(FileUndoError::Changed))
        ));
        assert!(matches!(
            session.take_outcome(),
            Some(NativeInteractiveOutcome::Rejected {
                error: crate::NativeInteractiveError::ControlFailed,
                ..
            })
        ));
        assert!(Arc::ptr_eq(session.runtime(), &current));
        assert_eq!(current.status().phase, NativeConversationRuntimePhase::Open);
        assert!(matches!(
            undo(&mut session).await,
            Err(Error::Undo(FileUndoError::Changed))
        ));
        drop(current);
        close(session, fixture).await;
    });
}

#[test]
fn infrastructure_and_quiescing_admission_are_typed_undo_failures() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        session.request_control(Control::UndoLast, 300).unwrap();
        let fence = session.runtime().begin_quiescence().unwrap();
        assert!(matches!(
            receipt(&mut session).await.result,
            Err(Error::Undo(FileUndoError::Unavailable))
        ));
        drop(fence);
        // Close the exact host scope: failure must not become metadata-save text.
        fixture.host.control_workers().unwrap().close();
        assert!(matches!(
            undo(&mut session).await,
            Err(Error::Undo(FileUndoError::Unavailable))
        ));
        close(session, fixture).await;
    });
}

#[test]
fn same_session_resume_keeps_undo_and_confirmed_new_session_clears_it() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"kept","content":"tracked"}),
        )
        .await;
        let id = session.runtime().id();
        session
            .request_transition(
                NativeInteractiveTransition::Resume(crate::NativeResumeTarget::Exact(id)),
                400,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        assert_eq!(
            undo(&mut session).await.unwrap(),
            FileUndoOutcome::Removed("kept".into())
        );
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"not-undone","content":"tracked"}),
        )
        .await;
        session
            .request_transition(NativeInteractiveTransition::New, 500)
            .unwrap();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        assert_eq!(undo(&mut session).await.unwrap(), FileUndoOutcome::Empty);
        assert_eq!(
            fs::read(fixture.workspace.join("not-undone")).unwrap(),
            b"tracked"
        );
        close(session, fixture).await;
    });
}

struct ExitPause {
    entered: Arc<Gate>,
    release: std::sync::mpsc::Receiver<()>,
}
impl Drop for ExitPause {
    fn drop(&mut self) {
        self.entered.release();
        let _ = self.release.recv();
    }
}
thread_local! {
    static EXIT_PAUSE: std::cell::RefCell<Option<ExitPause>> = const { std::cell::RefCell::new(None) };
}

#[test]
fn inverse_receipt_does_not_claim_joined_thread_local_destruction() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"joined","content":"tracked"}),
        )
        .await;
        let workers = fixture.host.control_workers().unwrap();
        let completion = fixture.host.terminal_shutdown_completion().unwrap();
        let entered = Arc::new(Gate::default());
        let (release, wait) = std::sync::mpsc::sync_channel(1);
        let pause = ExitPause {
            entered: entered.clone(),
            release: wait,
        };
        let tracker = fixture.undo.clone();
        session.request_control(Control::UndoLast, 300).unwrap();
        session.control.as_mut().unwrap().future =
            run(session.runtime().clone(), workers.clone(), move || {
                EXIT_PAUSE.with(|slot| *slot.borrow_mut() = Some(pause));
                tracker.undo_last(&CancellationToken::new())
            });
        assert!(matches!(
            receipt(&mut session).await.result,
            Ok(Receipt::Undone(FileUndoOutcome::Removed(_)))
        ));
        poll_fn(|cx| {
            entered.wake.register(cx.waker());
            if entered.ready.load(std::sync::atomic::Ordering::SeqCst) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        workers.close();
        assert!(!completion.is_complete());
        assert!(!fixture.workspace.join("joined").exists());
        release.send(()).unwrap();
        drop(workers);
        close(session, fixture).await;
        assert!(completion.is_complete());
    });
}

#[test]
fn pending_inverse_finishes_before_explicit_active_turn_cancellation() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        fixture.transport.push(support::call(
            "write_file",
            &json!({"path":"cancel","content":"created"}),
        ));
        fixture.transport.push(support::answer());
        session.enqueue("write before cancellation".into()).unwrap();
        poll_fn(|cx| {
            let _ = session.poll_progress(cx, 200);
            if session
                .presentation
                .as_ref()
                .is_some_and(|event| matches!(event.payload, TurnEvent::ToolFinished { .. }))
            {
                return Poll::Ready(());
            }
            if session.take_presentation().is_some() {
                cx.waker().wake_by_ref();
            }
            Poll::Pending
        })
        .await;
        session.enqueue("retained queued prompt".into()).unwrap();
        let (entered, release) = paused_inverse(&mut session, &fixture);
        wait_entered(&mut session, &entered).await;
        assert!(session.request_cancel());
        let before = session.runtime().record();
        let _ = session.poll_progress(&mut Context::from_waker(Waker::noop()), 300);
        assert_eq!(session.runtime().record(), before);
        assert!(fixture.workspace.join("cancel").exists());
        release.send(()).unwrap();
        assert!(matches!(
            receipt(&mut session).await.result,
            Ok(Receipt::Undone(FileUndoOutcome::Removed(_)))
        ));
        let _ = outcome(&mut session).await;
        assert_eq!(session.runtime().status().queued_jobs, 1);
        assert!(!fixture.workspace.join("cancel").exists());
        close(session, fixture).await;
    });
}

struct PanickingPayload;
impl Drop for PanickingPayload {
    fn drop(&mut self) {
        panic!("payload destructor must not run");
    }
}

#[test]
fn inverse_panic_with_panicking_payload_keeps_ambiguous_receipt() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"panic","content":"created"}),
        )
        .await;
        let tracker = fixture.undo.clone();
        session.request_control(Control::UndoLast, 300).unwrap();
        session.control.as_mut().unwrap().future = run(
            session.runtime().clone(),
            fixture.host.control_workers().unwrap(),
            move || {
                tracker.undo_last(&CancellationToken::new()).unwrap();
                std::panic::panic_any(PanickingPayload);
            },
        );
        assert!(matches!(
            receipt(&mut session).await.result,
            Err(Error::Undo(FileUndoError::Ambiguous))
        ));
        assert!(!fixture.workspace.join("panic").exists());
        close(session, fixture).await;
    });
}

struct PanickingWake;
impl std::task::Wake for PanickingWake {
    fn wake(self: Arc<Self>) {
        std::panic::panic_any(PanickingPayload);
    }
}

#[test]
fn inverse_admission_release_panic_cannot_report_unpublished_unavailability() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        tool(
            &fixture,
            &mut session,
            "write_file",
            json!({"path":"wake","content":"created"}),
        )
        .await;
        let (entered, release) = paused_inverse(&mut session, &fixture);
        wait_entered(&mut session, &entered).await;
        let mut fence = session.runtime().begin_quiescence().unwrap();
        let mut waiting = fence.wait_idle();
        let wake = Waker::from(Arc::new(PanickingWake));
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(&wake))
                .is_pending()
        );
        release.send(()).unwrap();
        assert!(matches!(
            receipt(&mut session).await.result,
            Err(Error::Undo(FileUndoError::Ambiguous))
        ));
        assert!(!fixture.workspace.join("wake").exists());
        drop(waiting);
        drop(fence);
        close(session, fixture).await;
    });
}
