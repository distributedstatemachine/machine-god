use super::transition::{CommitResult, Phase};
use super::*;
use crate as native;
use futures_util::{future::poll_fn, task::AtomicWaker};
use machine_god_core::CancellationToken;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[path = "../../tests/interactive_session/support.rs"]
#[allow(
    dead_code,
    reason = "Shared fixture also supports external composed scenarios."
)]
mod support;
use support::Fixture;

#[path = "controls/tests.rs"]
mod controls;

#[derive(Default)]
struct Gate {
    ready: AtomicBool,
    polls: AtomicUsize,
    drops: AtomicUsize,
    wake: AtomicWaker,
}
impl Gate {
    fn release(&self) {
        self.ready.store(true, Ordering::SeqCst);
        self.wake.wake();
    }
}
struct Probe(Arc<Gate>);
impl Drop for Probe {
    fn drop(&mut self) {
        self.0.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn deferred<T: Send + 'static>(value: T, gate: &Arc<Gate>) -> BoxFuture<'static, T> {
    let gate = gate.clone();
    let probe = Probe(gate.clone());
    Box::pin(async move {
        let _probe = probe;
        poll_fn(|cx| {
            gate.polls.fetch_add(1, Ordering::SeqCst);
            gate.wake.register(cx.waker());
            if gate.ready.load(Ordering::SeqCst) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        value
    })
}
fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
async fn owner(fixture: &Fixture) -> NativeInteractiveSession {
    NativeInteractiveSession::open(
        fixture.host.clone(),
        NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            NativeModelPreferences::new(
                "workspace/default",
                crate::NativeReasoningEffort::default(),
                false,
            )
            .unwrap(),
        )
        .unwrap(),
        NativeInteractiveInitialSession::Fresh,
        100,
    )
    .await
    .unwrap()
}
async fn outcome(owner: &mut NativeInteractiveSession) -> NativeInteractiveOutcome {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 300);
            owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}
fn install(owner: &mut NativeInteractiveSession, phase: Phase) -> NativeInteractiveRequestId {
    let receipt = owner
        .request_transition(NativeInteractiveTransition::New, 200)
        .unwrap();
    let request = owner.pending.take().unwrap();
    owner.transition = Some(Transition {
        request,
        guard: Some(owner.current.begin_quiescence().unwrap()),
        phase,
        terminal: None,
        prepared: None,
    });
    receipt.id
}
async fn candidate(owner: &NativeInteractiveSession) -> Arc<NativeConversationRuntime> {
    let conversation = transition::prepare(
        &owner.host,
        &owner.options,
        NativeInteractiveTransition::New,
        200,
    )
    .await
    .unwrap();
    transition::compose(
        &owner.host,
        &owner.options,
        conversation,
        None,
        None,
        false,
        200,
    )
    .await
    .unwrap()
}
async fn close(mut owner: NativeInteractiveSession, fixture: Fixture) {
    owner.request_shutdown();
    let _ = outcome(&mut owner).await;
    assert!(owner.is_closed());
    drop(owner);
    fixture.finish();
}

#[test]
fn superseded_started_preparation_survives_dropped_poll_wrapper_and_reports_publication() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = owner(&fixture).await;
        let old = owner.current.clone();
        let prepared = transition::prepare(
            &owner.host,
            &owner.options,
            NativeInteractiveTransition::New,
            200,
        )
        .await
        .unwrap();
        let published = transition::conversation_principal(&prepared);
        let gate = Arc::new(Gate::default());
        let first = install(&mut owner, Phase::Preparing(deferred(Ok(prepared), &gate)));
        let mut wrapper = Box::pin(poll_fn(|cx| owner.poll_progress(cx, 200)));
        assert!(
            wrapper
                .as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
                .is_pending()
        );
        drop(wrapper);
        assert_eq!(gate.drops.load(Ordering::SeqCst), 0);
        let second = owner
            .request_transition(NativeInteractiveTransition::Clear, 210)
            .unwrap();
        let third = owner
            .request_transition(NativeInteractiveTransition::Reset, 220)
            .unwrap();
        assert_eq!(second.superseded, Some(first));
        assert_eq!(third.superseded, Some(second.id));
        gate.release();
        let NativeInteractiveOutcome::Superseded {
            request,
            candidate,
            error,
            ..
        } = outcome(&mut owner).await
        else {
            panic!("publication receipt");
        };
        assert_eq!(request, first);
        assert_eq!(candidate, Some(published));
        assert!(error.is_none());
        assert_eq!(gate.drops.load(Ordering::SeqCst), 1);
        let NativeInteractiveOutcome::Transition(receipt) = outcome(&mut owner).await else {
            panic!("latest request receipt");
        };
        assert_eq!(receipt.request, third.id);
        assert!(receipt.reset.is_some());
        assert!(old.enqueue("retired".into()).is_err());
        drop(old);
        close(owner, fixture).await;
    });
}

#[test]
fn started_commit_retains_undo_and_late_request_waits_for_settled_receipt() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = owner(&fixture).await;
        let candidate = candidate(&owner).await;
        let destination = transition::principal(&candidate);
        let gate = Arc::new(Gate::default());
        // The concrete handoff supplies the opaque success receipt. Its delivery
        // is then gated to exercise the owner after effects, before observation.
        let handoff = owner
            .host
            .terminal_lifecycle_requester()
            .unwrap()
            .handoff(
                transition::principal(&owner.current),
                destination.clone(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let undo = fixture.undo.reserve_clear().unwrap();
        let first = install(
            &mut owner,
            Phase::Committing {
                candidate,
                undo: Some(undo),
                future: deferred(
                    CommitResult {
                        reset: None,
                        handoff: Ok(handoff),
                        affected: true,
                    },
                    &gate,
                ),
            },
        );
        assert!(
            owner
                .poll_progress(&mut Context::from_waker(std::task::Waker::noop()), 200)
                .is_pending()
        );
        let second = owner
            .request_transition(NativeInteractiveTransition::New, 210)
            .unwrap();
        assert_eq!(second.superseded, None);
        assert!(matches!(
            fixture.undo.reserve_clear(),
            Err(crate::FileUndoError::Busy)
        ));
        gate.release();
        let NativeInteractiveOutcome::Transition(receipt) = outcome(&mut owner).await else {
            panic!("first receipt");
        };
        assert_eq!(receipt.request, first);
        assert_eq!(receipt.destination, destination);
        assert_eq!(gate.drops.load(Ordering::SeqCst), 1);
        drop(fixture.undo.reserve_clear().unwrap());
        let NativeInteractiveOutcome::Transition(receipt) = outcome(&mut owner).await else {
            panic!("queued receipt");
        };
        assert_eq!(receipt.request, second.id);
        close(owner, fixture).await;
    });
}

#[test]
fn post_reset_handoff_failure_keeps_exact_candidate_receipt_and_undo_fenced_without_replay() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = owner(&fixture).await;
        let candidate = candidate(&owner).await;
        let identity = transition::principal(&candidate);
        let reset = owner
            .host
            .terminal_lifecycle_requester()
            .unwrap()
            .reset_current_workspace(
                transition::principal(&owner.current),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let gate = Arc::new(Gate::default());
        gate.release();
        install(
            &mut owner,
            Phase::Committing {
                candidate,
                undo: Some(fixture.undo.reserve_clear().unwrap()),
                future: deferred(
                    CommitResult {
                        reset: Some(reset),
                        handoff: Err(NativeTerminalTransitionError::Conflict),
                        affected: true,
                    },
                    &gate,
                ),
            },
        );
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Indeterminate { .. }
        ));
        assert!(owner.is_fenced());
        assert_eq!(owner.retained_candidate(), Some(identity));
        assert!(owner.retained_reset_receipt().is_some());
        assert!(owner.current.enqueue("forbidden".into()).is_err());
        assert!(
            owner
                .request_transition(NativeInteractiveTransition::Reset, 400)
                .is_err()
        );
        for _ in 0..5 {
            assert!(
                owner
                    .poll_progress(&mut Context::from_waker(std::task::Waker::noop()), 300)
                    .is_pending()
            );
        }
        assert_eq!(gate.polls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            fixture.undo.reserve_clear(),
            Err(crate::FileUndoError::Busy)
        ));
        close(owner, fixture).await;
    });
}

#[test]
fn undo_reservation_failure_reopens_old_runtime_without_committing_candidate() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = owner(&fixture).await;
        let old = owner.current.clone();
        owner.enqueue("queued".into()).unwrap();
        let candidate = candidate(&owner).await;
        let reservation = fixture.undo.reserve_clear().unwrap();
        install(&mut owner, Phase::Ready(candidate));
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Rejected {
                error: NativeInteractiveError::Undo(crate::FileUndoError::Busy),
                ..
            }
        ));
        assert!(Arc::ptr_eq(&old, &owner.current));
        assert_eq!(old.status().queued_jobs, 1);
        assert_eq!(
            old.status().phase,
            crate::NativeConversationRuntimePhase::Open
        );
        assert!(fixture.transport.requests().is_empty());
        drop(reservation);
        drop(old);
        close(owner, fixture).await;
    });
}

#[test]
fn uncertain_commit_is_driven_through_shutdown_without_replay_or_receipt_loss() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = owner(&fixture).await;
        let candidate = candidate(&owner).await;
        let gate = Arc::new(Gate::default());
        install(
            &mut owner,
            Phase::Committing {
                candidate,
                undo: Some(fixture.undo.reserve_clear().unwrap()),
                future: deferred(
                    CommitResult {
                        reset: None,
                        handoff: Err(NativeTerminalTransitionError::Uncertain),
                        affected: true,
                    },
                    &gate,
                ),
            },
        );
        assert!(
            owner
                .poll_progress(&mut Context::from_waker(std::task::Waker::noop()), 200)
                .is_pending()
        );
        owner.request_shutdown();
        assert!(
            owner
                .poll_progress(&mut Context::from_waker(std::task::Waker::noop()), 300)
                .is_pending()
        );
        assert_eq!(gate.drops.load(Ordering::SeqCst), 0);
        gate.release();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Indeterminate {
                error: NativeInteractiveError::Terminal(NativeTerminalTransitionError::Uncertain),
                ..
            }
        ));
        assert!(owner.is_closed());
        assert!(owner.is_fenced());
        assert_eq!(gate.drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            owner.current.status().phase,
            crate::NativeConversationRuntimePhase::Retired
        );
        drop(owner);
        fixture.finish();
    });
}

#[test]
fn constructor_checks_exact_host_workspace_and_request_exhaustion_preserves_pending_selection() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = owner(&fixture).await;
        let wrong = NativeInteractiveSessionOptions::new(
            fixture.workspace.parent().unwrap().to_owned(),
            owner.options.defaults.clone(),
        )
        .unwrap();
        let opening = NativeInteractiveSession::open(
            fixture.host.clone(),
            wrong.clone(),
            NativeInteractiveInitialSession::Fresh,
            200,
        );
        drop(opening);
        assert!(matches!(
            NativeInteractiveSession::open(
                fixture.host.clone(),
                wrong,
                NativeInteractiveInitialSession::Fresh,
                200
            )
            .await,
            Err(NativeInteractiveError::Configuration)
        ));
        let previous = owner
            .request_transition(NativeInteractiveTransition::Clear, 200)
            .unwrap();
        owner.next_request = u64::MAX;
        assert!(matches!(
            owner.request_transition(NativeInteractiveTransition::Reset, 210),
            Err(NativeInteractiveError::IdentityExhausted)
        ));
        assert_eq!(owner.pending.as_ref().unwrap().id, previous.id);
        assert_eq!(format!("{owner:?}"), "NativeInteractiveSession { .. }");
        close(owner, fixture).await;
    });
}

#[test]
fn empty_presentation_and_outcome_takes_do_not_self_wake_idle_progress() {
    struct Wakes(AtomicUsize);
    impl std::task::Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = owner(&fixture).await;
        let counter = Arc::new(Wakes(AtomicUsize::new(0)));
        let waker = std::task::Waker::from(counter.clone());
        assert!(
            owner
                .poll_progress(&mut Context::from_waker(&waker), 200)
                .is_pending()
        );
        for _ in 0..10 {
            assert!(owner.take_outcome().is_none());
            assert!(owner.take_presentation().is_none());
        }
        assert_eq!(counter.0.load(Ordering::SeqCst), 0);
        owner.enqueue("wake actual admission".into()).unwrap();
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        owner.outcome = Some(NativeInteractiveOutcome::Rejected {
            request: NativeInteractiveRequestId(1),
            error: NativeInteractiveError::Configuration,
            settled_turn: None,
            candidate: None,
        });
        assert!(
            owner
                .poll_progress(&mut Context::from_waker(&waker), 200)
                .is_ready()
        );
        assert!(owner.take_outcome().is_some());
        assert_eq!(counter.0.load(Ordering::SeqCst), 2);
        assert!(owner.take_outcome().is_none());
        assert_eq!(counter.0.load(Ordering::SeqCst), 2);
        close(owner, fixture).await;
    });
}

#[test]
fn shutdown_retires_even_when_previous_control_outcome_has_not_been_consumed() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut owner = owner(&fixture).await;
        let old = owner.current.clone();
        owner.outcome = Some(NativeInteractiveOutcome::Rejected {
            request: NativeInteractiveRequestId(1),
            error: NativeInteractiveError::Configuration,
            settled_turn: None,
            candidate: None,
        });
        owner.request_shutdown();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = owner.poll_progress(cx, 300);
                if owner.is_closed() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            owner.take_outcome(),
            Some(NativeInteractiveOutcome::Rejected { .. })
        ));
        assert!(old.enqueue("retired".into()).is_err());
        drop(old);
        drop(owner);
        fixture.finish();
    });
}
