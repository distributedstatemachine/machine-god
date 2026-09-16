//! Same-workspace replacement stays in the current native manager's lane.
use super::{
    AcpSessionError, BoxFuture, Context, NativeAcpHostReuse, NativeAcpSelectionOutcome,
    NativeAcpSelectionOwner, NativeAcpSessionSelection, Pending, Phase, Poll, Request, cleanup,
    reuse,
};
use crate::{NativeInteractiveOutcome, NativeInteractiveRequestId};

impl NativeAcpSelectionOwner {
    pub(super) fn begin_preparation(&self, request: &Request) -> Phase {
        if let Some(current) = &self.current
            && current.host.managed_agents_selected()
        {
            Phase::CheckingReuse(
                self.factory.prepare_reuse(
                    current.host.clone(),
                    request.workspace.clone(),
                    request
                        .configuration
                        .as_ref()
                        .expect("selection configuration")
                        .network_requirement(),
                    request.cancellation.clone(),
                ),
            )
        } else {
            self.prepare_fresh(request)
        }
    }

    fn prepare_fresh(&self, request: &Request) -> Phase {
        Phase::Preparing(
            self.factory.prepare(
                request.workspace.clone(),
                request
                    .configuration
                    .as_ref()
                    .expect("selection configuration")
                    .network_requirement(),
                request.cancellation.clone(),
            ),
        )
    }

    pub(super) fn check_reuse(
        &mut self,
        mut pending: Pending,
        mut future: BoxFuture<'static, Result<Option<NativeAcpHostReuse>, AcpSessionError>>,
        cx: &mut Context<'_>,
    ) -> bool {
        let result = match future.as_mut().poll(cx) {
            Poll::Pending => {
                pending.phase = Phase::CheckingReuse(future);
                self.pending = Some(pending);
                return false;
            }
            Poll::Ready(result) => result,
        };
        pending.phase = if pending.request.cancellation.is_cancelled() {
            rejected(AcpSessionError::Cancelled)
        } else {
            match result {
                Err(error) => rejected(error),
                Ok(None) => self.prepare_fresh(&pending.request),
                Ok(Some(reuse)) => self.admit_reuse(&mut pending.request, reuse),
            }
        };
        self.pending = Some(pending);
        true
    }

    fn admit_reuse(&mut self, request: &mut Request, reuse: NativeAcpHostReuse) -> Phase {
        let Some(current) = &mut self.current else {
            return rejected(AcpSessionError::Closed);
        };
        if !reuse.matches(&current.host, &request.workspace)
            || request.previous.as_ref() != Some(&current.session.principal())
        {
            return rejected(AcpSessionError::InvalidConfiguration);
        }
        match reuse::Stage::new(
            &mut current.session,
            reuse,
            request
                .configuration
                .take()
                .expect("selection configuration"),
            request.cancellation.clone(),
        ) {
            Ok(stage) => Phase::ReuseStarting(stage),
            Err(error) => rejected(error),
        }
    }

    pub(super) fn start_reuse(
        &mut self,
        mut pending: Pending,
        mut stage: Box<reuse::Stage>,
        cx: &mut Context<'_>,
    ) -> bool {
        let result = match self.current.as_ref() {
            Some(current) => stage.poll_start(&current.session, cx),
            None => Poll::Ready(Err(AcpSessionError::Closed)),
        };
        let result = match result {
            Poll::Pending => {
                pending.phase = Phase::ReuseStarting(stage);
                self.pending = Some(pending);
                return false;
            }
            Poll::Ready(result) => result,
        };
        pending.phase = if pending.request.cancellation.is_cancelled() {
            reject_stage(stage, AcpSessionError::Cancelled)
        } else if let Err(error) = result {
            reject_stage(stage, error)
        } else {
            self.cancel_current();
            Phase::ReuseDraining(stage)
        };
        self.pending = Some(pending);
        true
    }

    pub(super) fn drain_reuse(
        &mut self,
        mut pending: Pending,
        mut stage: Box<reuse::Stage>,
    ) -> bool {
        if pending.request.cancellation.is_cancelled() {
            pending.phase = reject_stage(stage, AcpSessionError::Cancelled);
        } else if let Some(current) = &mut self.current {
            if self.turn_outcome.is_some()
                || current.session.has_pending_prompt()
                || current.session.has_pending_model_save()
                || current.session.has_pending_command_control()
            {
                pending.phase = Phase::ReuseDraining(stage);
                self.pending = Some(pending);
                return false;
            }
            let selection = pending
                .request
                .selection
                .take()
                .expect("replacement selection");
            let replay = matches!(selection, NativeAcpSessionSelection::Load(_));
            match stage.adopt(&mut current.session, selection, pending.request.now_ms) {
                Ok(request) => {
                    pending.request.candidate_may_have_persisted = true;
                    pending.phase = Phase::ReuseTransition { request, replay };
                }
                Err(error) => pending.phase = reject_stage(stage, error),
            }
        } else {
            pending.phase = reject_stage(stage, AcpSessionError::Closed);
        }
        self.pending = Some(pending);
        true
    }

    pub(super) fn finish_reuse(
        &mut self,
        mut pending: Pending,
        request: NativeInteractiveRequestId,
        replay: bool,
    ) -> bool {
        let Some(outcome) = self.reuse_outcome.take() else {
            pending.phase = Phase::ReuseTransition { request, replay };
            self.pending = Some(pending);
            return false;
        };
        match outcome {
            NativeInteractiveOutcome::Transition(receipt) if receipt.request == request => {
                let current = self.current.as_mut().expect("retained transition owner");
                if pending.request.previous.as_ref() != Some(&receipt.source)
                    || receipt.destination != current.session.principal()
                    || current.session.selection_ready().is_err()
                {
                    self.fence_reuse(pending.request, AcpSessionError::Unavailable);
                } else {
                    current.session.refresh_selected_parent(replay);
                    self.outcome = Some(NativeAcpSelectionOutcome::Selected {
                        id: pending.request.id,
                        previous: pending.request.previous,
                        current: receipt.destination,
                    });
                }
            }
            NativeInteractiveOutcome::Rejected {
                request: actual,
                error,
                ..
            } if actual == request => {
                pending.phase = rejected(if pending.request.cancellation.is_cancelled() {
                    AcpSessionError::Cancelled
                } else {
                    error.into()
                });
                self.pending = Some(pending);
            }
            NativeInteractiveOutcome::Indeterminate {
                request: actual,
                error,
                ..
            } if actual == request => {
                self.fence_reuse(pending.request, error.into());
            }
            _ => self.fence_reuse(pending.request, AcpSessionError::Unavailable),
        }
        true
    }

    fn fence_reuse(&mut self, request: Request, error: AcpSessionError) {
        self.fenced = true;
        self.outcome = Some(NativeAcpSelectionOutcome::Indeterminate {
            id: request.id,
            error,
            previous: request.previous,
            candidate: self
                .current
                .as_ref()
                .map(|current| current.session.principal()),
        });
    }
}

fn rejected(error: AcpSessionError) -> Phase {
    Phase::Rejecting {
        error,
        future: None,
    }
}
fn reject_stage(stage: Box<reuse::Stage>, error: AcpSessionError) -> Phase {
    Phase::Rejecting {
        error,
        future: Some(cleanup::reject_stage(stage)),
    }
}
