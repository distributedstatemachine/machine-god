use super::transition::{Phase, conversation_principal, principal};
use super::{
    Arc, BackgroundOutputOwner, Context, NativeConversationError, NativeConversationRuntime,
    NativeInteractiveError, NativeInteractiveOutcome, NativeInteractiveRequestId,
    NativeInteractiveSession, NativeInteractiveTransition, NativeInteractiveTransitionReceipt,
    Poll, Request, Transition, transition,
};
use crate::{NativeConversation, NativeRuntimeQuiescence};
use futures_core::Stream;
use machine_god_core::BoxFuture;
use machine_god_core::TurnEvent;
use std::pin::Pin;

const MAX_PROGRESS_STEPS: usize = 32;

impl NativeInteractiveSession {
    pub(super) fn drive(&mut self, cx: &mut Context<'_>, now_ms: i64) -> Poll<()> {
        self.wake = Some(cx.waker().clone());
        // Clipboard response/cleanup is independent of durable controls and
        // never prevents active provider, transition, or shutdown progress.
        self.poll_copy(cx);
        // Finish accepted saves before closing admission or the actual turn's
        // metadata editor. This lane never waits for presentation consumption.
        if self.poll_control(cx).is_pending() {
            return self.readiness();
        }
        for _ in 0..MAX_PROGRESS_STEPS {
            if self.closed
                || self.shutdown_error.is_some()
                || (self.outcome.is_some() && !self.shutting_down)
            {
                return Poll::Ready(());
            }
            if !self.begin_requested_transition(now_ms) {
                return Poll::Ready(());
            }
            let draining = self.transition.is_some() || self.shutting_down || self.cancel_requested;
            if draining {
                self.presentation.take();
            }
            // The progress budget may have retained an admission before its
            // first poll. An unread control receipt must not let it take a job.
            if !draining && self.control_outcome.is_some() && !self.current.status().active {
                return Poll::Ready(());
            }
            match self.poll_retained_admission(cx) {
                Poll::Pending => return self.readiness(),
                Poll::Ready(Err(error)) => {
                    self.fail_turn(error.into());
                    return Poll::Ready(());
                }
                Poll::Ready(Ok(())) => {}
            }
            self.dispatch_requested_cancel();
            if let Some(turn) = &mut self.turn {
                if !draining && self.presentation.is_some() {
                    return Poll::Ready(());
                }
                match Pin::new(turn).poll_next(cx) {
                    Poll::Pending => return self.readiness(),
                    Poll::Ready(Some(Ok(event))) => {
                        if matches!(
                            event.payload,
                            TurnEvent::Completed { .. } | TurnEvent::Failed { .. }
                        ) {
                            self.turn.take();
                            self.cancel_requested = false;
                            if let Some(transition) = &mut self.transition {
                                transition.terminal = Some(event);
                            } else {
                                self.outcome = Some(NativeInteractiveOutcome::Turn(Ok(event)));
                                return Poll::Ready(());
                            }
                        } else if !draining {
                            self.presentation = Some(event);
                            return Poll::Ready(());
                        }
                    }
                    Poll::Ready(Some(Err(error))) => {
                        self.turn.take();
                        self.fail_turn(error.into());
                        return Poll::Ready(());
                    }
                    Poll::Ready(None) => {
                        self.turn.take();
                        self.fail_turn(NativeInteractiveError::Conversation(
                            NativeConversationError::Engine,
                        ));
                        return Poll::Ready(());
                    }
                }
                continue;
            }
            self.cancel_requested = false;
            if let Some(transition) = self.transition.take() {
                if self.drive_transition(transition, cx).is_pending() {
                    return self.readiness();
                }
                continue;
            }
            if self.presentation.is_some() {
                return Poll::Ready(());
            }
            if self.control_outcome.is_some() {
                return Poll::Ready(());
            }
            if self.current.status().queued_jobs == 0 {
                return self.readiness();
            }
            let runtime = Arc::clone(&self.current);
            self.admission = Some(Box::pin(async move { runtime.start_next(now_ms).await }));
        }
        cx.waker().wake_by_ref();
        self.readiness()
    }
    fn poll_retained_admission(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), crate::NativeConversationRuntimeError>> {
        let Some(admission) = &mut self.admission else {
            return Poll::Ready(Ok(()));
        };
        // Controls have settled. Signal pre-handle reads without losing the
        // later cancellation of a core handle still being reserved.
        if self.cancel_requested {
            let _ = self.current.request_active_cancel();
        }
        let result = match admission.as_mut().poll(cx) {
            Poll::Pending => {
                // First poll may just have installed the taken job's token.
                // Do not wait for the read worker to wake us to deliver cancel.
                if self.cancel_requested {
                    let _ = self.current.request_active_cancel();
                }
                return Poll::Pending;
            }
            Poll::Ready(result) => result,
        };
        self.admission.take();
        match result {
            Ok(turn) => {
                self.turn = turn;
                Poll::Ready(Ok(()))
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn begin_requested_transition(&mut self, now_ms: i64) -> bool {
        if self.transition.is_none() && (self.pending.is_some() || self.shutting_down) {
            let request = self.pending.take().unwrap_or(Request {
                id: NativeInteractiveRequestId(0),
                kind: NativeInteractiveTransition::New,
                now_ms,
            });
            match self.current.begin_quiescence() {
                Ok(guard) => {
                    self.transition = Some(Transition {
                        request,
                        guard: Some(guard),
                        phase: Phase::Draining,
                        terminal: None,
                        prepared: None,
                    });
                }
                Err(error) => {
                    if self.shutting_down && self.outcome.is_some() {
                        self.shutdown_error = Some(error.into());
                        return false;
                    }
                    self.outcome = Some(NativeInteractiveOutcome::Rejected {
                        request: request.id,
                        error: error.into(),
                        settled_turn: None,
                        candidate: None,
                    });
                    return false;
                }
            }
        }
        true
    }
    fn dispatch_requested_cancel(&self) {
        if self.cancel_requested
            && let Some(handle) = self
                .turn
                .as_ref()
                .and_then(crate::NativeConversationRuntimeTurn::handle)
        {
            let _ = handle.cancel();
        }
    }
    fn readiness(&self) -> Poll<()> {
        if self.outcome.is_some()
            || self.control_outcome.is_some()
            || self.copy_outcome.is_some()
            || self.presentation.is_some()
            || self.closed
            || self.shutdown_error.is_some()
        {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
    fn fail_turn(&mut self, error: NativeInteractiveError) {
        self.cancel_requested = false;
        if let Some(mut transition) = self.transition.take() {
            let request = self
                .pending
                .take()
                .map_or(transition.request.id, |request| request.id);
            self.outcome = Some(NativeInteractiveOutcome::Rejected {
                request,
                error,
                settled_turn: transition.terminal.take(),
                candidate: transition.prepared.take(),
            });
        } else {
            self.outcome = Some(NativeInteractiveOutcome::Turn(Err(error)));
        }
    }
    fn reject(&mut self, mut transition: Transition, error: NativeInteractiveError) {
        if self.shutting_down && self.outcome.is_some() {
            self.shutdown_error = Some(error);
            return;
        }
        self.outcome = Some(NativeInteractiveOutcome::Rejected {
            request: transition.request.id,
            error,
            settled_turn: transition.terminal.take(),
            candidate: transition.prepared.take(),
        });
    }
    fn supersede(
        &mut self,
        mut transition: Transition,
        candidate: Option<BackgroundOutputOwner>,
        error: Option<NativeInteractiveError>,
    ) {
        let candidate = candidate.or_else(|| transition.prepared.take());
        transition.prepared.take();
        if self.shutting_down {
            // Preserve the completed publication observation before shutdown.
            self.outcome = Some(NativeInteractiveOutcome::Superseded {
                request: transition.request.id,
                candidate,
                error,
                settled_turn: transition.terminal.take(),
            });
            transition.phase = Phase::Draining;
            self.transition = Some(transition);
        } else if let Some(next) = self.pending.take() {
            self.outcome = Some(NativeInteractiveOutcome::Superseded {
                request: transition.request.id,
                candidate,
                error,
                settled_turn: transition.terminal.take(),
            });
            transition.request = next;
            transition.phase = Phase::Draining;
            self.transition = Some(transition);
        }
    }
    fn drive_transition(&mut self, mut transition: Transition, cx: &mut Context<'_>) -> Poll<()> {
        let phase = std::mem::replace(&mut transition.phase, Phase::Draining);
        match phase {
            Phase::Draining => {
                let Some(mut guard) = transition.guard.take() else {
                    self.reject(transition, NativeInteractiveError::Unavailable);
                    return Poll::Ready(());
                };
                transition.phase = Phase::Waiting(Box::pin(async move {
                    guard.wait_idle().await?;
                    Ok(guard)
                }));
            }
            Phase::Waiting(future) => return self.drive_waiting(transition, future, cx),
            Phase::Preparing(future) => return self.drive_preparing(transition, future, cx),
            Phase::Composing(future) => return self.drive_composing(transition, future, cx),
            Phase::Ready(candidate) => return self.begin_commit(transition, candidate, cx),
            Phase::Committing {
                candidate,
                undo,
                mut future,
            } => match future.as_mut().poll(cx) {
                Poll::Pending => {
                    transition.phase = Phase::Committing {
                        candidate,
                        undo,
                        future,
                    };
                    self.transition = Some(transition);
                    return Poll::Pending;
                }
                Poll::Ready(result) => {
                    self.finish_commit(transition, candidate, undo, result);
                    return Poll::Ready(());
                }
            },
            Phase::Fenced {
                candidate,
                undo,
                reset,
                handoff,
            } => {
                if self.shutting_down {
                    match Self::retire_fenced(&mut transition, &candidate) {
                        Ok(()) => {
                            self.closed = true;
                            if self.outcome.is_none() {
                                self.outcome = Some(NativeInteractiveOutcome::Shutdown);
                            }
                        }
                        Err(error) => self.shutdown_error = Some(error),
                    }
                }
                transition.phase = Phase::Fenced {
                    candidate,
                    undo,
                    reset,
                    handoff,
                };
                self.transition = Some(transition);
                return Poll::Pending;
            }
        }
        self.transition = Some(transition);
        Poll::Ready(())
    }

    fn drive_waiting(
        &mut self,
        mut transition: Transition,
        mut future: BoxFuture<'static, Result<NativeRuntimeQuiescence, NativeInteractiveError>>,
        cx: &mut Context<'_>,
    ) -> Poll<()> {
        match future.as_mut().poll(cx) {
            Poll::Pending => {
                transition.phase = Phase::Waiting(future);
                self.transition = Some(transition);
                return Poll::Pending;
            }
            Poll::Ready(Err(error)) => {
                self.reject(transition, error);
                return Poll::Ready(());
            }
            Poll::Ready(Ok(guard)) => {
                transition.guard = Some(guard);
                if self.shutting_down {
                    match transition
                        .guard
                        .as_mut()
                        .expect("returned guard")
                        .try_retire()
                    {
                        Ok(()) => {
                            self.closed = true;
                            if self.outcome.is_none() {
                                self.outcome = Some(NativeInteractiveOutcome::Shutdown);
                            }
                        }
                        Err(error) => {
                            self.shutdown_error = Some(error.into());
                            self.transition = Some(transition);
                        }
                    }
                    return Poll::Ready(());
                }
                if let Some(next) = self.pending.take() {
                    transition.request = next;
                }
                let host = Arc::clone(&self.host);
                let options = self.options.clone();
                let kind = transition.request.kind.clone();
                let now_ms = transition.request.now_ms;
                transition.phase = Phase::Preparing(Box::pin(async move {
                    transition::prepare(&host, &options, kind, now_ms).await
                }));
            }
        }
        self.transition = Some(transition);
        Poll::Ready(())
    }

    fn drive_preparing(
        &mut self,
        mut transition: Transition,
        mut future: BoxFuture<'static, Result<NativeConversation, NativeInteractiveError>>,
        cx: &mut Context<'_>,
    ) -> Poll<()> {
        match future.as_mut().poll(cx) {
            Poll::Pending => {
                transition.phase = Phase::Preparing(future);
                self.transition = Some(transition);
                return Poll::Pending;
            }
            Poll::Ready(result) => {
                if self.pending.is_some() || self.shutting_down {
                    let (candidate, error) = match result {
                        Ok(conversation) => (Some(conversation_principal(&conversation)), None),
                        Err(error) => (None, Some(error)),
                    };
                    self.supersede(transition, candidate, error);
                    return Poll::Ready(());
                }
                let conversation = match result {
                    Ok(value) => value,
                    Err(error) => {
                        self.reject(transition, error);
                        return Poll::Ready(());
                    }
                };
                let source = principal(&self.current);
                let destination = conversation_principal(&conversation);
                transition.prepared = Some(destination.clone());
                if source == destination {
                    self.outcome = Some(NativeInteractiveOutcome::Transition(
                        NativeInteractiveTransitionReceipt {
                            request: transition.request.id,
                            source,
                            destination,
                            unchanged: true,
                            reset: None,
                            handoff: None,
                            settled_turn: transition.terminal.take(),
                        },
                    ));
                    return Poll::Ready(());
                }
                let snapshot = match transition
                    .guard
                    .as_ref()
                    .expect("waiting returns guard")
                    .selection_snapshot()
                {
                    Ok(value) => value,
                    Err(error) => {
                        self.reject(transition, error.into());
                        return Poll::Ready(());
                    }
                };
                let host = Arc::clone(&self.host);
                let options = self.options.clone();
                let policy = snapshot.permission_policy().cloned();
                let catalog = snapshot.model_catalog().cloned();
                let now_ms = transition.request.now_ms;
                transition.phase = Phase::Composing(Box::pin(async move {
                    transition::compose(
                        &host,
                        &options,
                        conversation,
                        policy,
                        catalog,
                        false,
                        now_ms,
                    )
                    .await
                }));
            }
        }
        self.transition = Some(transition);
        Poll::Ready(())
    }

    fn drive_composing(
        &mut self,
        mut transition: Transition,
        mut future: BoxFuture<
            'static,
            Result<Arc<NativeConversationRuntime>, NativeInteractiveError>,
        >,
        cx: &mut Context<'_>,
    ) -> Poll<()> {
        match future.as_mut().poll(cx) {
            Poll::Pending => {
                transition.phase = Phase::Composing(future);
                self.transition = Some(transition);
                return Poll::Pending;
            }
            Poll::Ready(result) => {
                if self.pending.is_some() || self.shutting_down {
                    let (candidate, error) = match result {
                        Ok(runtime) => (Some(principal(&runtime)), None),
                        Err(error) => (None, Some(error)),
                    };
                    self.supersede(transition, candidate, error);
                    return Poll::Ready(());
                }
                match result {
                    Ok(candidate) => transition.phase = Phase::Ready(candidate),
                    Err(error) => {
                        self.reject(transition, error);
                        return Poll::Ready(());
                    }
                }
            }
        }
        self.transition = Some(transition);
        Poll::Ready(())
    }

    fn begin_commit(
        &mut self,
        mut transition: Transition,
        candidate: Arc<NativeConversationRuntime>,
        cx: &mut Context<'_>,
    ) -> Poll<()> {
        if self.pending.is_some() || self.shutting_down {
            self.supersede(transition, Some(principal(&candidate)), None);
            return Poll::Ready(());
        }
        let undo = match self
            .host
            .undo_tracker()
            .ok_or(NativeInteractiveError::Configuration)
            .and_then(|tracker| {
                tracker
                    .reserve_clear()
                    .map_err(NativeInteractiveError::Undo)
            }) {
            Ok(undo) => undo,
            Err(error) => {
                self.reject(transition, error);
                return Poll::Ready(());
            }
        };
        let mut future = match transition::commit(
            &self.host,
            &transition.request.kind,
            principal(&self.current),
            principal(&candidate),
        ) {
            Ok(future) => future,
            Err(error) => {
                self.reject(transition, error);
                return Poll::Ready(());
            }
        };
        // No return/yield between freezing this request and first poll.
        match future.as_mut().poll(cx) {
            Poll::Pending => {
                transition.phase = Phase::Committing {
                    candidate,
                    undo: Some(undo),
                    future,
                };
            }
            Poll::Ready(result) => {
                self.finish_commit(transition, candidate, Some(undo), result);
                return Poll::Ready(());
            }
        }

        self.transition = Some(transition);
        Poll::Ready(())
    }

    fn finish_commit(
        &mut self,
        mut transition: Transition,
        candidate: Arc<NativeConversationRuntime>,
        undo: Option<crate::file_undo::FileUndoClearReservation>,
        result: transition::CommitResult,
    ) {
        match result.handoff {
            Ok(handoff) => {
                if let Err(error) = transition
                    .guard
                    .as_mut()
                    .expect("commit retains guard")
                    .try_retire()
                {
                    transition.phase = Phase::Fenced {
                        candidate,
                        undo,
                        reset: result.reset,
                        handoff: Some(handoff),
                    };
                    self.outcome = Some(NativeInteractiveOutcome::Indeterminate {
                        request: transition.request.id,
                        error: error.into(),
                        settled_turn: transition.terminal.take(),
                    });
                    self.transition = Some(transition);
                    return;
                }
                transition.guard.take();
                if let Some(undo) = undo {
                    undo.commit();
                }
                let source = principal(&self.current);
                let destination = principal(&candidate);
                self.current = candidate;
                self.outcome = Some(NativeInteractiveOutcome::Transition(
                    NativeInteractiveTransitionReceipt {
                        request: transition.request.id,
                        source,
                        destination,
                        unchanged: false,
                        reset: result.reset,
                        handoff: Some(handoff),
                        settled_turn: transition.terminal.take(),
                    },
                ));
            }
            Err(error) if result.affected => {
                transition.phase = Phase::Fenced {
                    candidate,
                    undo,
                    reset: result.reset,
                    handoff: None,
                };
                self.outcome = Some(NativeInteractiveOutcome::Indeterminate {
                    request: transition.request.id,
                    error: NativeInteractiveError::Terminal(error),
                    settled_turn: transition.terminal.take(),
                });
                self.transition = Some(transition);
            }
            Err(error) => self.reject(transition, NativeInteractiveError::Terminal(error)),
        }
    }

    fn retire_fenced(
        transition: &mut Transition,
        candidate: &Arc<NativeConversationRuntime>,
    ) -> Result<(), NativeInteractiveError> {
        if let Some(guard) = &mut transition.guard {
            guard.try_retire()?;
            transition.guard.take();
        }
        match candidate.begin_quiescence() {
            Ok(mut guard) => guard.try_retire().map_err(Into::into),
            Err(super::NativeConversationRuntimeError::Retired) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
