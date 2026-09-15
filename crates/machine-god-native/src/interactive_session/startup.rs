//! Native pre-selection custody, including retryable opens and cancelled startup.
use super::{
    Arc, BoxFuture, Context, NativeInteractiveError, NativeInteractiveInitialSession,
    NativeInteractiveSession, NativeInteractiveSessionOptions, NativeReferenceHost, Poll, fmt,
    managed,
};
use crate::NativeManagedAgents;
use machine_god_core::CancellationToken;

type OpenResult = Result<NativeInteractiveSession, (NativeInteractiveError, Box<managed::Owner>)>;

async fn open_owned(
    host: Arc<NativeReferenceHost>,
    options: NativeInteractiveSessionOptions,
    initial: NativeInteractiveInitialSession,
    now_ms: i64,
    owner: Box<managed::Owner>,
    cancellation: CancellationToken,
) -> OpenResult {
    let mut owner = Some(owner);
    NativeInteractiveSession::try_open_with_agents(
        host,
        options,
        initial,
        now_ms,
        &mut owner,
        cancellation,
    )
    .await
    .map_err(|error| {
        (
            error,
            owner.expect("failed opening retains original manager"),
        )
    })
}
enum State {
    Idle(Box<managed::Owner>),
    Opening {
        future: BoxFuture<'static, OpenResult>,
        cancellation: CancellationToken,
    },
    ClosingSession(Box<NativeInteractiveSession>),
    Finished,
}

/// Owns the manager before the initial session is selected. Poll wrappers and
/// presentation never own its preparation future. Drop is abandonment, not an
/// owned-cleanup receipt; request shutdown and poll through `Ok(None)` to settle.
pub struct NativeManagedInteractiveStartup {
    host: Arc<NativeReferenceHost>,
    options: NativeInteractiveSessionOptions,
    state: State,
    closing: bool,
    shutdown_failed: bool,
    wake: Option<std::task::Waker>,
}
impl fmt::Debug for NativeManagedInteractiveStartup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedInteractiveStartup { .. }")
    }
}

impl NativeManagedInteractiveStartup {
    #[must_use]
    pub fn is_opening(&self) -> bool {
        matches!(self.state, State::Opening { .. })
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Finished)
    }

    /// Pure binding to the same actual host-service allocation. Failure returns
    /// the original manager for explicit cleanup; it does not close a foreign host.
    /// # Errors
    /// Rejects mismatched hosts or invalid interactive options before admission.
    pub fn new(
        host: Arc<NativeReferenceHost>,
        options: NativeInteractiveSessionOptions,
        agents: NativeManagedAgents,
    ) -> Result<Self, (NativeInteractiveError, Box<NativeManagedAgents>)> {
        let valid = if agents.belongs_to(&host) {
            options.validate_for_host(&host)
        } else {
            Err(NativeInteractiveError::Configuration)
        };
        if let Err(error) = valid {
            return Err((error, Box::new(agents)));
        }
        Ok(Self {
            host,
            options,
            state: State::Idle(managed::Owner::new(agents)),
            closing: false,
            shutdown_failed: false,
            wake: None,
        })
    }

    /// Records one initial selection without polling a session, provider or tool.
    /// A failed selection retains the manager for an explicit later attempt.
    /// # Errors
    /// Rejects concurrent opening, shutdown and an already-transferred owner.
    pub fn request_open(
        &mut self,
        initial: NativeInteractiveInitialSession,
        now_ms: i64,
    ) -> Result<(), NativeInteractiveError> {
        if self.closing || matches!(self.state, State::Finished) {
            return Err(NativeInteractiveError::Closed);
        }
        let State::Idle(owner) = &self.state else {
            return Err(NativeInteractiveError::Busy);
        };
        if owner.agents.is_closing() {
            return Err(NativeInteractiveError::Closed);
        }
        let State::Idle(owner) = std::mem::replace(&mut self.state, State::Finished) else {
            unreachable!("validated idle owner");
        };
        let host = self.host.clone();
        let options = self.options.clone();
        let cancellation = CancellationToken::new();
        let selected_cancel = cancellation.clone();
        self.state = State::Opening {
            cancellation,
            future: Box::pin(open_owned(
                host,
                options,
                initial,
                now_ms,
                owner,
                selected_cancel,
            )),
        };
        self.notify();
        Ok(())
    }

    /// Cancels preparation but retains its original future until cleanup settles.
    pub fn request_shutdown(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        match &mut self.state {
            State::Idle(owner) => owner.agents.request_shutdown(),
            State::Opening { cancellation, .. } => {
                cancellation.cancel();
            }
            State::ClosingSession(session) => session.request_shutdown(),
            State::Finished => {}
        }
        self.notify();
    }

    /// Returns a session once, a retained opening failure, or `None` after owned
    /// shutdown (and on subsequent polls after ownership was transferred).
    /// # Errors
    /// Opening/cleanup failed. The startup keeps any untransferred manager and
    /// cleanup custody; an error is not a successful shutdown receipt.
    pub fn poll_open(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Poll<Result<Option<NativeInteractiveSession>, NativeInteractiveError>> {
        self.wake = Some(cx.waker().clone());
        match &mut self.state {
            State::Idle(owner) => {
                if self.closing {
                    match owner.agents.poll_shutdown(cx, now_ms) {
                        Poll::Ready(Ok(())) => {
                            self.state = State::Finished;
                            self.finished()
                        }
                        Poll::Ready(Err(error)) => {
                            self.shutdown_failed = true;
                            Poll::Ready(Err(NativeInteractiveError::Managed(error)))
                        }
                        Poll::Pending => Poll::Pending,
                    }
                } else {
                    if let Poll::Ready(Err(error)) = owner.agents.poll_progress(cx, now_ms) {
                        return Poll::Ready(Err(NativeInteractiveError::Managed(error)));
                    }
                    Poll::Pending
                }
            }
            State::Opening { future, .. } => {
                let Poll::Ready(result) = future.as_mut().poll(cx) else {
                    return Poll::Pending;
                };
                match result {
                    Ok(mut session) if self.closing => {
                        session.request_shutdown();
                        self.state = State::ClosingSession(Box::new(session));
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    Ok(session) => {
                        self.state = State::Finished;
                        Poll::Ready(Ok(Some(session)))
                    }
                    Err((error, owner)) => {
                        self.state = State::Idle(owner);
                        if self.closing {
                            self.shutdown_failed |=
                                !matches!(error, NativeInteractiveError::Closed);
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        } else {
                            Poll::Ready(Err(error))
                        }
                    }
                }
            }
            State::ClosingSession(session) => {
                let _ = session.poll_progress(cx, now_ms);
                if session.is_closed() {
                    self.state = State::Finished;
                    self.finished()
                } else if session.shutdown_error().is_some() {
                    self.shutdown_failed = true;
                    Poll::Ready(Err(NativeInteractiveError::Unavailable))
                } else {
                    Poll::Pending
                }
            }
            State::Finished => self.finished(),
        }
    }

    fn notify(&mut self) {
        if let Some(wake) = self.wake.take() {
            wake.wake();
        }
    }

    fn finished(&self) -> Poll<Result<Option<NativeInteractiveSession>, NativeInteractiveError>> {
        Poll::Ready(if self.shutdown_failed {
            Err(NativeInteractiveError::Unavailable)
        } else {
            Ok(None)
        })
    }
}
impl Drop for NativeManagedInteractiveStartup {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
