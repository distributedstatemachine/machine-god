use super::{
    AcpSessionError, Arc, CancellationToken, Context, Current, NativeAcpPreparedHost,
    NativeAcpSelectionId, NativeAcpSelectionOutcome, NativeAcpSelectionOwner,
    NativeAcpSelectionTurnOutcome, NativeAcpSession, NativeRuntimeQuiescence, PathBuf, Pending,
    Phase, Poll, Request, cleanup,
};
use crate::NativeInteractiveOutcome;
use std::time::Duration;

impl NativeAcpSelectionOwner {
    #[allow(clippy::too_many_lines)] // One bounded owner pump; phase work is delegated below.
    pub(super) fn drive(&mut self, cx: &mut Context<'_>, now_ms: i64) -> Poll<()> {
        self.wake = Some(cx.waker().clone());
        let mut ready = false;
        if self.turn_outcome.is_none()
            && let Some(current) = &mut self.current
        {
            ready = current.session.poll_progress(cx, now_ms).is_ready();
            if let Some(NativeInteractiveOutcome::Turn(outcome)) = current.session.take_outcome() {
                self.turn_outcome = Some(NativeAcpSelectionTurnOutcome {
                    owner: current.session.principal(),
                    outcome,
                });
            }
        }
        for _ in 0..16 {
            if self.outcome.is_some() {
                return Poll::Ready(());
            }
            if self.pending.is_none() {
                if self.shutdown
                    && self.fenced
                    && (self.current.is_some() || self.fenced_candidate.is_some())
                {
                    self.fenced_guard.take();
                    let previous = self.current.take();
                    let candidate = self.fenced_candidate.take();
                    let principal = previous
                        .as_ref()
                        .map(|previous| previous.session.principal());
                    self.pending = Some(Pending {
                        request: Request {
                            id: NativeAcpSelectionId(0),
                            selection: None,
                            workspace: PathBuf::new(),
                            configuration: None,
                            now_ms,
                            cancellation: CancellationToken::new(),
                            previous: principal,
                            candidate_may_have_persisted: false,
                        },
                        phase: Phase::Retiring {
                            candidate: None,
                            future: Box::pin(async move {
                                let mut result = cleanup::Receipt {
                                    complete: false,
                                    workers: Vec::new(),
                                };
                                for current in [candidate, previous].into_iter().flatten() {
                                    let receipt = cleanup::retire(
                                        current.host,
                                        Some(current.session),
                                        false,
                                        now_ms,
                                    )
                                    .await;
                                    result.workers.extend(receipt.workers);
                                }
                                result
                            }),
                        },
                    });
                    continue;
                }
                if self.shutdown
                    && let Some(current) = self.current.as_mut()
                {
                    let id = current.session.id();
                    self.pending = Some(Pending {
                        request: Request {
                            id: NativeAcpSelectionId(0),
                            selection: None,
                            workspace: current.host.workspace_root().to_path_buf(),
                            configuration: None,
                            now_ms,
                            cancellation: CancellationToken::new(),
                            previous: Some(current.session.principal()),
                            candidate_may_have_persisted: false,
                        },
                        phase: Phase::Draining(None),
                    });
                    let _ = current.session.request_cancel(&id);
                } else {
                    return if ready || self.turn_outcome.is_some() || self.is_closed() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    };
                }
            }
            let pending = self.pending.take().expect("pending operation");
            if self.advance(pending, cx) {
                continue;
            }
            return if ready || self.turn_outcome.is_some() || self.outcome.is_some() {
                Poll::Ready(())
            } else {
                Poll::Pending
            };
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }

    #[allow(clippy::too_many_lines)] // Exhaustive phase ownership transfer stays in one match.
    fn advance(&mut self, mut pending: Pending, cx: &mut Context<'_>) -> bool {
        let phase = std::mem::replace(&mut pending.phase, Phase::Requested);
        match phase {
            Phase::Requested => {
                if pending.request.selection.is_none() {
                    pending.phase = Phase::Draining(None);
                    self.cancel_current();
                } else if pending.request.cancellation.is_cancelled() {
                    pending.phase = Phase::Rejecting {
                        error: AcpSessionError::Cancelled,
                        future: None,
                    };
                } else {
                    pending.phase = Phase::Preparing(self.factory.prepare(
                        pending.request.workspace.clone(),
                        pending.request.cancellation.clone(),
                    ));
                }
            }
            Phase::Preparing(mut future) => match future.as_mut().poll(cx) {
                Poll::Pending => {
                    pending.phase = Phase::Preparing(future);
                    self.pending = Some(pending);
                    return false;
                }
                Poll::Ready(Err(error)) => {
                    pending.phase = Phase::Rejecting {
                        error,
                        future: None,
                    }
                }
                Poll::Ready(Ok(host)) => {
                    // An invalid factory alias is not an independently owned
                    // candidate: rejecting it must never close the active host.
                    if self
                        .current
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(&current.host, &host.host))
                    {
                        pending.phase = Phase::Rejecting {
                            error: AcpSessionError::InvalidConfiguration,
                            future: None,
                        };
                        self.pending = Some(pending);
                        return true;
                    }
                    let invalid = host.validate().is_err()
                        || host.host.workspace_root() != pending.request.workspace
                        || self.current.as_ref().is_some_and(|current| {
                            Arc::ptr_eq(&current.host, &host.host)
                                || Arc::ptr_eq(
                                    &current.permission_contexts,
                                    &host.permission_contexts,
                                )
                                || current
                                    .host
                                    .mcp_contexts()
                                    .zip(host.host.mcp_contexts())
                                    .is_some_and(|(current, candidate)| {
                                        Arc::ptr_eq(&current, &candidate)
                                    })
                        });
                    if invalid || pending.request.cancellation.is_cancelled() {
                        let error = if invalid {
                            AcpSessionError::InvalidConfiguration
                        } else {
                            AcpSessionError::Cancelled
                        };
                        pending.phase =
                            Self::reject_host(host, None, error, pending.request.now_ms);
                    } else {
                        let configuration = pending
                            .request
                            .configuration
                            .take()
                            .expect("selection config");
                        let selected = host.host.clone();
                        let cancellation = pending.request.cancellation.clone();
                        let future = Box::pin(async move {
                            let deadline = selected
                                .mcp_deadline_after(Duration::from_secs(30))
                                .map_err(|_| AcpSessionError::Unavailable)?;
                            let owner = selected
                                .mcp_ephemeral_owner()
                                .ok_or(AcpSessionError::InvalidConfiguration)?;
                            let receipt = owner
                                .replace(configuration, cancellation, deadline)
                                .await
                                .map_err(|_| AcpSessionError::Unavailable)?;
                            if receipt.closed_after_publication() {
                                return Err(AcpSessionError::Unavailable);
                            }
                            owner.ready().map_err(|_| AcpSessionError::Unavailable)
                        });
                        pending.phase = Phase::Starting { host, future };
                    }
                }
            },
            Phase::Starting { host, mut future } => match future.as_mut().poll(cx) {
                Poll::Pending => {
                    pending.phase = Phase::Starting { host, future };
                    self.pending = Some(pending);
                    return false;
                }
                Poll::Ready(result) => {
                    if let Err(error) = result {
                        pending.phase =
                            Self::reject_host(host, None, error, pending.request.now_ms);
                    } else if pending.request.cancellation.is_cancelled() {
                        pending.phase = Self::reject_host(
                            host,
                            None,
                            AcpSessionError::Cancelled,
                            pending.request.now_ms,
                        );
                    } else {
                        pending.phase = Phase::Draining(Some(host));
                        self.cancel_current();
                    }
                }
            },
            Phase::Draining(host) => {
                if pending.request.cancellation.is_cancelled() && !self.shutdown {
                    pending.phase = match host {
                        Some(host) => Self::reject_host(
                            host,
                            None,
                            AcpSessionError::Cancelled,
                            pending.request.now_ms,
                        ),
                        None => Phase::Rejecting {
                            error: AcpSessionError::Cancelled,
                            future: None,
                        },
                    };
                } else if self.turn_outcome.is_some()
                    || self.current.as_ref().is_some_and(|current| {
                        current.session.has_pending_prompt()
                            || current.session.has_pending_model_save()
                    })
                {
                    pending.phase = Phase::Draining(host);
                    self.pending = Some(pending);
                    return false;
                } else if let Some(current) = &self.current {
                    match current.session.runtime().begin_quiescence() {
                        Ok(mut guard) => {
                            pending.phase = Phase::Quiescing {
                                host,
                                future: Box::pin(async move {
                                    guard
                                        .wait_idle()
                                        .await
                                        .map_err(|_| AcpSessionError::Unavailable)?;
                                    Ok(guard)
                                }),
                            }
                        }
                        Err(_) => {
                            pending.phase = match host {
                                Some(host) => Self::reject_host(
                                    host,
                                    None,
                                    AcpSessionError::Busy,
                                    pending.request.now_ms,
                                ),
                                None => Phase::Rejecting {
                                    error: AcpSessionError::Busy,
                                    future: None,
                                },
                            }
                        }
                    }
                } else if let Some(host) = host {
                    Self::begin_open(&mut pending, host, None);
                } else {
                    self.outcome = Some(NativeAcpSelectionOutcome::Closed {
                        id: pending.request.id,
                        previous: pending.request.previous,
                    });
                    return true;
                }
            }
            Phase::Quiescing { host, mut future } => match future.as_mut().poll(cx) {
                Poll::Pending => {
                    pending.phase = Phase::Quiescing { host, future };
                    self.pending = Some(pending);
                    return false;
                }
                Poll::Ready(Err(error)) => {
                    pending.phase = match host {
                        Some(host) => Self::reject_host(host, None, error, pending.request.now_ms),
                        None => Phase::Rejecting {
                            error,
                            future: None,
                        },
                    }
                }
                Poll::Ready(Ok(guard)) => {
                    if let Some(host) = host {
                        if pending.request.cancellation.is_cancelled() {
                            drop(guard);
                            pending.phase = Self::reject_host(
                                host,
                                None,
                                AcpSessionError::Cancelled,
                                pending.request.now_ms,
                            );
                        } else {
                            Self::begin_open(&mut pending, host, Some(guard));
                        }
                    } else if !self.begin_retire(&mut pending, None, Some(guard)) {
                        return true;
                    }
                }
            },
            Phase::Opening {
                host,
                guard,
                mut future,
            } => match future.as_mut().poll(cx) {
                Poll::Pending => {
                    pending.phase = Phase::Opening {
                        host,
                        guard,
                        future,
                    };
                    self.pending = Some(pending);
                    return false;
                }
                Poll::Ready(result) => {
                    let result = result.map_err(|error| (None, error)).and_then(|session| {
                        if host
                            .host
                            .mcp_ephemeral_owner()
                            .is_none_or(|owner| owner.ready().is_err())
                        {
                            Err((Some(Box::new(session)), AcpSessionError::Unavailable))
                        } else if pending.request.cancellation.is_cancelled() {
                            Err((Some(Box::new(session)), AcpSessionError::Cancelled))
                        } else {
                            Ok(session)
                        }
                    });
                    match result {
                        Ok(session) => {
                            if !self.begin_retire(
                                &mut pending,
                                Some(Current {
                                    session,
                                    host: host.host,
                                    permission_contexts: host.permission_contexts,
                                }),
                                guard,
                            ) {
                                return true;
                            }
                        }
                        Err((session, error)) => {
                            drop(guard);
                            pending.phase = Self::reject_host(
                                host,
                                session.map(|session| *session),
                                error,
                                pending.request.now_ms,
                            );
                        }
                    }
                }
            },
            Phase::Retiring {
                candidate,
                mut future,
            } => match future.as_mut().poll(cx) {
                Poll::Pending => {
                    pending.phase = Phase::Retiring { candidate, future };
                    self.pending = Some(pending);
                    return false;
                }
                Poll::Ready(receipt) => {
                    if !receipt.complete {
                        self.fenced = true;
                        self.retained_cleanup.push(receipt);
                        let candidate_principal = candidate
                            .as_ref()
                            .map(|candidate| candidate.session.principal());
                        self.current = candidate.map(|candidate| *candidate);
                        self.outcome = Some(NativeAcpSelectionOutcome::Indeterminate {
                            id: pending.request.id,
                            error: AcpSessionError::Unavailable,
                            previous: pending.request.previous,
                            candidate: candidate_principal,
                        });
                    } else if let Some(candidate) = candidate {
                        let current = candidate.session.principal();
                        self.current = Some(*candidate);
                        self.outcome = Some(NativeAcpSelectionOutcome::Selected {
                            id: pending.request.id,
                            previous: pending.request.previous,
                            current,
                        });
                    } else {
                        self.outcome = Some(NativeAcpSelectionOutcome::Closed {
                            id: pending.request.id,
                            previous: pending.request.previous,
                        });
                    }
                    return true;
                }
            },
            Phase::Rejecting { error, mut future } => {
                if let Some(job) = &mut future {
                    match job.as_mut().poll(cx) {
                        Poll::Pending => {
                            pending.phase = Phase::Rejecting { error, future };
                            self.pending = Some(pending);
                            return false;
                        }
                        Poll::Ready(receipt) if !receipt.complete => {
                            self.fenced = true;
                            self.retained_cleanup.push(receipt);
                            self.outcome = Some(NativeAcpSelectionOutcome::Indeterminate {
                                id: pending.request.id,
                                error,
                                previous: pending.request.previous,
                                candidate: None,
                            });
                            return true;
                        }
                        Poll::Ready(_) => {}
                    }
                }
                self.outcome = Some(NativeAcpSelectionOutcome::Rejected {
                    id: pending.request.id,
                    error,
                    old_preserved: self.current.is_some(),
                    candidate_may_have_persisted: pending.request.candidate_may_have_persisted,
                });
                return true;
            }
        }
        self.pending = Some(pending);
        true
    }

    fn cancel_current(&mut self) {
        if let Some(current) = &mut self.current {
            let _ = current.session.request_cancel(&current.session.id());
        }
        self.notify();
    }
    fn reject_host(
        host: NativeAcpPreparedHost,
        session: Option<NativeAcpSession>,
        error: AcpSessionError,
        now_ms: i64,
    ) -> Phase {
        Phase::Rejecting {
            error,
            future: Some(cleanup::retire(host.host, session, false, now_ms)),
        }
    }
    fn begin_open(
        pending: &mut Pending,
        host: NativeAcpPreparedHost,
        guard: Option<NativeRuntimeQuiescence>,
    ) {
        if host
            .host
            .mcp_ephemeral_owner()
            .is_none_or(|owner| owner.ready().is_err())
        {
            drop(guard);
            pending.phase = Self::reject_host(
                host,
                None,
                AcpSessionError::Unavailable,
                pending.request.now_ms,
            );
            return;
        }
        pending.request.candidate_may_have_persisted = true;
        let future = NativeAcpSession::open(
            host.host.clone(),
            host.options.clone(),
            pending.request.selection.take().expect("session selection"),
            pending.request.now_ms,
        );
        pending.phase = Phase::Opening {
            host,
            guard,
            future,
        };
    }
    fn begin_retire(
        &mut self,
        pending: &mut Pending,
        candidate: Option<Current>,
        mut guard: Option<NativeRuntimeQuiescence>,
    ) -> bool {
        if guard
            .as_mut()
            .is_some_and(|guard| guard.try_retire().is_err())
        {
            // Retirement uncertainty keeps both owners; never reopen or
            // describe the previous selection as usable after this cutoff.
            self.fenced = true;
            let candidate_principal = candidate
                .as_ref()
                .map(|candidate| candidate.session.principal());
            self.fenced_candidate = candidate;
            self.fenced_guard = guard.take();
            self.outcome = Some(NativeAcpSelectionOutcome::Indeterminate {
                id: pending.request.id,
                error: AcpSessionError::Unavailable,
                previous: pending.request.previous.clone(),
                candidate: candidate_principal,
            });
            return false;
        }
        drop(guard);
        if let Some(previous) = self.current.take() {
            pending.phase = Phase::Retiring {
                candidate: candidate.map(Box::new),
                future: cleanup::retire(
                    previous.host,
                    Some(previous.session),
                    true,
                    pending.request.now_ms,
                ),
            };
        } else {
            pending.phase = Phase::Retiring {
                candidate: candidate.map(Box::new),
                future: Box::pin(async {
                    cleanup::Receipt {
                        complete: true,
                        workers: Vec::new(),
                    }
                }),
            };
        }
        true
    }
}
