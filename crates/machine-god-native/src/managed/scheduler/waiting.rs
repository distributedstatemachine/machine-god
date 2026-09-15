use super::{
    RunRef, SchedulerError,
    state::{Inner, RunIdentity},
};
use machine_god_core::{CancellationToken, Cancelled};
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// Inert until first poll; every admission uses the same FIFO grant queue.
pub(crate) struct Acquire {
    run: RunRef,
    ticket: Option<u64>,
    cancellation: Option<Cancelled>,
    done: bool,
}
impl Acquire {
    pub(super) fn new(run: RunRef) -> Self {
        Self {
            run,
            ticket: None,
            cancellation: None,
            done: false,
        }
    }
}
impl Future for Acquire {
    type Output = Result<(), SchedulerError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        assert!(!self.done, "acquisition polled after completion");
        let (inner, identity) = match self.run.resolve() {
            Ok(value) => value,
            Err(error) => {
                self.cancellation.take();
                self.done = true;
                return Poll::Ready(Err(error));
            }
        };
        let cancellation = self
            .cancellation
            .get_or_insert_with(|| identity.handle.cancelled());
        if Pin::new(cancellation).poll(cx).is_ready() {
            abort(&inner, &identity, self.ticket);
            self.cancellation.take();
            self.done = true;
            return Poll::Ready(Err(SchedulerError::Cancelled));
        }
        // Clone invokes caller code before taking the registry lock.
        let waker = cx.waker().clone();
        let result = inner.poll_acquire(identity.id, &mut self.ticket, waker);
        if !matches!(result, Ok(false)) {
            self.cancellation.take();
        }
        if identity.handle.is_cancelled()
            || (matches!(result, Ok(true)) && !inner.is_executing(identity.id))
        {
            abort(&inner, &identity, self.ticket);
            self.cancellation.take();
            self.done = true;
            return Poll::Ready(Err(SchedulerError::Cancelled));
        }
        match result {
            Ok(false) => Poll::Pending,
            Ok(true) => {
                self.done = true;
                Poll::Ready(Ok(()))
            }
            Err(error) => {
                if self.ticket.is_some() {
                    abort(&inner, &identity, self.ticket);
                }
                self.done = true;
                Poll::Ready(Err(error))
            }
        }
    }
}
impl Drop for Acquire {
    fn drop(&mut self) {
        if !self.done
            && self.ticket.is_some()
            && let Ok((inner, identity)) = self.run.resolve()
        {
            abort(&inner, &identity, self.ticket);
        }
    }
}
impl fmt::Debug for Acquire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Acquire")
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

/// A target observation's output is private until execution quota is reacquired.
/// This includes observed errors and timeout values. Cancellation/drop instead
/// cancels the actual caller Turn, admitting only its independent settlement.
pub(crate) struct DependencyWait<F: Future> {
    run: RunRef,
    target: RunRef,
    observation: Option<Pin<Box<F>>>,
    output: Option<F::Output>,
    ticket: Option<u64>,
    cancellation: CancellationToken,
    cancelled: Option<Cancelled>,
    run_cancelled: Option<Cancelled>,
    done: bool,
}
// F is separately pinned; neither it nor its output is projected as pinned.
impl<F: Future> Unpin for DependencyWait<F> {}
impl<F: Future> DependencyWait<F> {
    pub(super) fn new(
        run: RunRef,
        target: RunRef,
        observation: F,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            run,
            target,
            observation: Some(Box::pin(observation)),
            output: None,
            ticket: None,
            cancelled: Some(cancellation.cancelled()),
            cancellation,
            run_cancelled: None,
            done: false,
        }
    }
    fn cancel(
        &mut self,
        inner: &Arc<Inner>,
        identity: &Arc<RunIdentity>,
    ) -> Poll<Result<F::Output, SchedulerError>> {
        abort(inner, identity, self.ticket);
        self.done = true;
        // Arbitrary output/future destructors run outside the scheduler mutex.
        self.observation.take();
        self.output.take();
        self.cancelled.take();
        self.run_cancelled.take();
        Poll::Ready(Err(SchedulerError::Cancelled))
    }
    fn reject(&mut self, error: SchedulerError) -> Poll<Result<F::Output, SchedulerError>> {
        self.done = true;
        self.observation.take();
        self.output.take();
        self.cancelled.take();
        self.run_cancelled.take();
        Poll::Ready(Err(error))
    }
    fn reject_live(
        &mut self,
        error: SchedulerError,
        inner: &Arc<Inner>,
        identity: &Arc<RunIdentity>,
    ) -> Poll<Result<F::Output, SchedulerError>> {
        let result = self.reject(error);
        // Rejected observers and cancellation registrations can have reentrant
        // destructors. Cancellation still wins this same poll.
        if self.cancellation.is_cancelled()
            || identity.handle.is_cancelled()
            || !inner.is_executing(identity.id)
        {
            return self.cancel(inner, identity);
        }
        result
    }
}
impl<F: Future> Future for DependencyWait<F> {
    type Output = Result<F::Output, SchedulerError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        assert!(!self.done, "dependency wait polled after completion");
        let (inner, identity) = match self.run.resolve() {
            Ok(value) => value,
            Err(error) => return self.reject(error),
        };
        let cancelled = Pin::new(self.cancelled.as_mut().expect("live cancellation observer"))
            .poll(cx)
            .is_ready();
        let run_cancelled = self
            .run_cancelled
            .get_or_insert_with(|| identity.handle.cancelled());
        if Pin::new(run_cancelled).poll(cx).is_ready() || cancelled {
            return self.cancel(&inner, &identity);
        }
        if self.ticket.is_none() {
            let Ok((target_inner, target)) = self.target.resolve() else {
                return self.reject_live(SchedulerError::Unschedulable, &inner, &identity);
            };
            if !Arc::ptr_eq(&inner, &target_inner) {
                return self.reject_live(SchedulerError::Foreign, &inner, &identity);
            }
            match inner.begin_wait(identity.id, target.id, cx.waker().clone()) {
                Ok(ticket) => self.ticket = Some(ticket),
                Err(error) => {
                    if !inner.is_executing(identity.id) {
                        return self.cancel(&inner, &identity);
                    }
                    return self.reject_live(error, &inner, &identity);
                }
            }
        }
        let ticket = self.ticket.expect("admitted dependency");
        if self.output.is_none() {
            if inner
                .refresh_wait(identity.id, ticket, cx.waker().clone())
                .is_err()
            {
                return self.cancel(&inner, &identity);
            }
            if self.cancellation.is_cancelled() || identity.handle.is_cancelled() {
                return self.cancel(&inner, &identity);
            }
            let result = self
                .observation
                .as_mut()
                .expect("pending observation")
                .as_mut()
                .poll(cx);
            if let Poll::Ready(value) = result {
                self.output = Some(value);
                self.observation.take();
            }
            if self.cancellation.is_cancelled() || identity.handle.is_cancelled() {
                return self.cancel(&inner, &identity);
            }
            if self.output.is_none() {
                return Poll::Pending;
            }
        }
        let result = inner.poll_reacquire(identity.id, ticket, cx.waker().clone());
        if matches!(result, Ok(true)) {
            self.cancelled.take();
            self.run_cancelled.take();
        }
        // Waking a newly granted task can cancel this run synchronously.
        if self.cancellation.is_cancelled()
            || identity.handle.is_cancelled()
            || (matches!(result, Ok(true)) && !inner.is_executing(identity.id))
        {
            return self.cancel(&inner, &identity);
        }
        match result {
            Ok(false) => Poll::Pending,
            Ok(true) => {
                self.done = true;
                Poll::Ready(Ok(self.output.take().expect("settled observation")))
            }
            Err(_) => self.cancel(&inner, &identity),
        }
    }
}
impl<F: Future> Drop for DependencyWait<F> {
    fn drop(&mut self) {
        if !self.done
            && self.ticket.is_some()
            && let Ok((inner, identity)) = self.run.resolve()
        {
            abort(&inner, &identity, self.ticket);
        }
    }
}
impl<F: Future> fmt::Debug for DependencyWait<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DependencyWait")
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}
fn abort(inner: &Arc<Inner>, identity: &Arc<RunIdentity>, ticket: Option<u64>) {
    inner.stop(identity.id, ticket, true);
    // Even a concurrently retired registry cannot leave core continuing without
    // quota. This is the actual old TurnHandle, never a guessed current run.
    let _ = identity.handle.cancel();
}
