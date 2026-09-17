use super::{NativeMcpRuntimeClock, NoticeError, state::Inner};
use machine_god_core::{BoxFuture, CancellationToken, Cancelled};
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, Weak},
    task::{Context, Poll},
    time::Instant,
};

/// Inert before polling. Owns only a weak registry and explicitly injected clock.
pub(crate) struct NoticeDeadline {
    inner: Weak<Inner>,
    clock: Arc<dyn NativeMcpRuntimeClock>,
    cancellation: CancellationToken,
    cancelled: Option<Cancelled>,
    ticket: Option<u64>,
    deadline: Option<Instant>,
    sleep: Option<BoxFuture<'static, ()>>,
    done: bool,
}
impl NoticeDeadline {
    pub(super) fn new(
        inner: Weak<Inner>,
        clock: Arc<dyn NativeMcpRuntimeClock>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            inner,
            clock,
            cancelled: Some(cancellation.cancelled()),
            cancellation,
            ticket: None,
            deadline: None,
            sleep: None,
            done: false,
        }
    }
    fn unregister(&mut self) {
        let Some(ticket) = self.ticket.take() else {
            return;
        };
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let removed = {
            let mut state = inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.driver.as_ref().is_some_and(|(id, _)| *id == ticket) {
                state.driver.take()
            } else {
                None
            }
        };
        drop(removed);
    }
    fn finish(&mut self, result: Result<(), NoticeError>) -> Poll<Result<(), NoticeError>> {
        self.unregister();
        self.sleep.take();
        self.cancelled.take();
        self.done = true;
        // Timer and waker destructors are outside the lock and may cancel.
        if self.cancellation.is_cancelled() {
            Poll::Ready(Err(NoticeError::Cancelled))
        } else {
            Poll::Ready(result)
        }
    }
}
impl NoticeDeadline {
    /// Subscribe immediately after observing a deadline. If another deadline is
    /// already due but cannot progress (for example inbox pressure), retain its
    /// change waker without repeatedly reporting readiness and spinning the host.
    pub(crate) fn poll_rearm(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), NoticeError>> {
        self.poll_deadline(cx, false)
    }

    fn poll_deadline(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        report_due: bool,
    ) -> Poll<Result<(), NoticeError>> {
        assert!(!self.done, "notice deadline polled after completion");
        if Pin::new(self.cancelled.as_mut().expect("live cancellation observer"))
            .poll(cx)
            .is_ready()
        {
            return self.finish(Err(NoticeError::Cancelled));
        }
        let Some(inner) = self.inner.upgrade() else {
            return self.finish(Err(NoticeError::Stale));
        };
        let now = self.clock.now();
        let mut waker = Some(cx.waker().clone());
        let mut old = None;
        let result = {
            let mut state = inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (|| {
                state.now(now)?;
                state.claim_driver(&mut self.ticket)?;
                old = state
                    .driver
                    .as_mut()
                    .unwrap()
                    .1
                    .replace(waker.take().unwrap());
                Ok(state.earliest())
            })()
        };
        drop(old);
        drop(waker);
        if self.cancellation.is_cancelled() {
            return self.finish(Err(NoticeError::Cancelled));
        }
        let next = match result {
            Ok(next) => next,
            Err(error) => return self.finish(Err(error)),
        };
        if self.deadline != next {
            self.sleep.take();
            self.deadline = next;
        }
        if self.cancellation.is_cancelled() {
            return self.finish(Err(NoticeError::Cancelled));
        }
        let Some(deadline) = next else {
            return Poll::Pending;
        };
        if now >= deadline {
            return if report_due {
                self.finish(Ok(()))
            } else {
                Poll::Pending
            };
        }
        if self.sleep.is_none() {
            let clock = self.clock.clone();
            self.sleep = Some(Box::pin(async move {
                clock.sleep_until(deadline).await;
            }));
        }
        let result = self.sleep.as_mut().unwrap().as_mut().poll(cx);
        if self.cancellation.is_cancelled() {
            return self.finish(Err(NoticeError::Cancelled));
        }
        if result.is_pending() {
            return Poll::Pending;
        }
        let after = self.clock.now();
        let result = {
            let mut state = inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.now(after).map(|()| state.earliest())
        };
        match result {
            Err(error) => self.finish(Err(error)),
            Ok(Some(actual)) if after >= actual => {
                self.sleep.take();
                if report_due {
                    self.finish(Ok(()))
                } else {
                    Poll::Pending
                }
            }
            Ok(Some(actual)) if actual == deadline => self.finish(Err(NoticeError::ClockViolation)),
            Ok(_) => {
                self.sleep.take();
                self.deadline = None;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}
impl Future for NoticeDeadline {
    type Output = Result<(), NoticeError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_deadline(cx, true)
    }
}
impl Drop for NoticeDeadline {
    fn drop(&mut self) {
        self.unregister();
    }
}
impl fmt::Debug for NoticeDeadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NoticeDeadline")
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}
