//! Weak authenticated requests; the outer manager owns durable settlement.

mod response;
#[cfg(test)]
mod tests;

use super::{
    actor::ManagedCommandActor,
    principal::{NativeManagedCallLease, NativePrincipalRequester},
};
use machine_god_core::{
    BoxFuture, CancellationToken, ManagedSubagentAuthority, ManagedSubagentCommand,
    ManagedSubagentError as Error, ManagedSubagentInvocation, ManagedSubagentResult, ToolContext,
};
use response::{Reply, Response};
use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll, Waker},
};

// Reservation, not eager allocation: includes admitted JSON + typed command,
// response normalization scratch + bounded typed response and bookkeeping.
const OPERATION_BYTES: usize = 4 * 1024 * 1024;
const MAX_REQUESTS: usize = 256;

pub(crate) type ManagedCommandResponse = BoxFuture<'static, Result<ManagedSubagentResult, Error>>;

#[derive(Clone, Copy, Debug)]
pub(crate) struct MailboxLimits {
    pub requests: usize,
    pub bytes: usize,
}
impl Default for MailboxLimits {
    fn default() -> Self {
        Self {
            requests: 64,
            bytes: 64 * OPERATION_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MailboxUsage {
    pub requests: usize,
    pub bytes: usize,
}

struct Shared {
    state: Mutex<State>,
    budget: Arc<Budget>,
}
struct State {
    closed: bool,
    queue: VecDeque<ManagedMailboxJob>,
    replies: Vec<Weak<Reply>>,
    waker: Option<Waker>,
}
struct Budget {
    limits: MailboxLimits,
    usage: Mutex<MailboxUsage>,
    wake: ManagedMailboxWake,
}
struct Reservation(Arc<Budget>);
impl Budget {
    fn reserve(self: &Arc<Self>) -> Result<Arc<Reservation>, Error> {
        let mut usage = self.usage.lock().map_err(|_| Error::Unavailable)?;
        if usage.requests >= self.limits.requests
            || usage
                .bytes
                .checked_add(OPERATION_BYTES)
                .is_none_or(|bytes| bytes > self.limits.bytes)
        {
            return Err(Error::ResourceLimit);
        }
        usage.requests += 1;
        usage.bytes += OPERATION_BYTES;
        Ok(Arc::new(Reservation(self.clone())))
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        {
            let mut usage = self
                .0
                .usage
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            usage.requests -= 1;
            usage.bytes -= OPERATION_BYTES;
        }
        self.0.wake.wake();
    }
}

/// Sole mailbox ownership stays with the manager, never the shared engine.
pub(crate) struct ManagedMailbox {
    shared: Arc<Shared>,
    principals: NativePrincipalRequester,
}
impl ManagedMailbox {
    pub(crate) fn new(
        principals: NativePrincipalRequester,
        limits: MailboxLimits,
    ) -> Result<Self, Error> {
        if !(1..=MAX_REQUESTS).contains(&limits.requests)
            || !(OPERATION_BYTES..=MAX_REQUESTS * OPERATION_BYTES).contains(&limits.bytes)
        {
            return Err(Error::ResourceLimit);
        }
        let shared = Arc::new_cyclic(|weak| Shared {
            state: Mutex::new(State {
                closed: false,
                queue: VecDeque::new(),
                replies: Vec::new(),
                waker: None,
            }),
            budget: Arc::new(Budget {
                limits,
                usage: Mutex::new(MailboxUsage::default()),
                wake: ManagedMailboxWake(weak.clone()),
            }),
        });
        Ok(Self { shared, principals })
    }
    pub(crate) fn requester(&self) -> ManagedMailboxRequester {
        ManagedMailboxRequester {
            shared: Arc::downgrade(&self.shared),
            principals: self.principals.clone(),
        }
    }
    /// An explicit native-host command, never a structural model invocation.
    /// Reserve shared queue/result capacity before cloning or normalizing input.
    pub(crate) fn request_human(
        &self,
        command: ManagedSubagentCommand,
        capture: impl FnOnce() -> Result<ManagedCommandActor, Error>,
        cancellation: CancellationToken,
    ) -> Result<ManagedCommandResponse, Error> {
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let reservation = self.shared.budget.reserve()?;
        let normalized = ManagedSubagentCommand::decode(command.to_arguments()?)?;
        // Release the caller's spare allocations before capturing native
        // resources; only the normalized payload enters queue custody.
        drop(command);
        let actor = capture()?;
        if !actor.is_human() || !actor.is_live() {
            return Err(Error::Unavailable);
        }
        let response = self.shared.submit_admitted(
            JobRequest::Human(Box::new(normalized)),
            actor,
            cancellation,
            reservation,
        )?;
        Ok(Box::pin(response))
    }
    pub(crate) fn wake_handle(&self) -> ManagedMailboxWake {
        ManagedMailboxWake(Arc::downgrade(&self.shared))
    }
    pub(crate) fn usage(&self) -> MailboxUsage {
        *self
            .shared
            .budget
            .usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// One consumer. The manager retains a Busy/Limit job before polling another.
    pub(crate) fn poll_next(&self, cx: &mut Context<'_>) -> Poll<Option<ManagedMailboxJob>> {
        let next_waker = cx.waker().clone();
        let (result, old_waker) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let old_waker = state.waker.replace(next_waker);
            let result = if let Some(job) = state.queue.pop_front() {
                Poll::Ready(Some(job))
            } else if state.closed {
                Poll::Ready(None)
            } else {
                Poll::Pending
            };
            (result, old_waker)
        };
        drop(old_waker);
        result
    }
    /// Reject observers, but never take already-dequeued manager settlement custody.
    pub(crate) fn close(&self) {
        let (queue, replies, waker) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            (
                std::mem::take(&mut state.queue),
                std::mem::take(&mut state.replies),
                state.waker.take(),
            )
        };
        for reply in replies.into_iter().filter_map(|reply| reply.upgrade()) {
            reply.finish(Err(Error::Unavailable));
        }
        drop(queue);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
impl Drop for ManagedMailbox {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Clone)]
pub(crate) struct ManagedMailboxRequester {
    shared: Weak<Shared>,
    principals: NativePrincipalRequester,
}
impl ManagedSubagentAuthority for ManagedMailboxRequester {
    fn execute(
        &self,
        invocation: ManagedSubagentInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ManagedSubagentResult, Error>> {
        let requester = self.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let shared = requester.shared.upgrade().ok_or(Error::Unavailable)?;
            let lease = requester
                .principals
                .claim(&invocation)
                .map_err(|_| Error::Unavailable)?;
            let response = shared.submit(invocation, lease, cancellation)?;
            drop(shared);
            response.await
        })
    }
}
impl Shared {
    fn submit(
        self: &Arc<Self>,
        invocation: ManagedSubagentInvocation,
        lease: NativeManagedCallLease,
        cancellation: CancellationToken,
    ) -> Result<Response, Error> {
        let reservation = self.budget.reserve()?;
        self.submit_admitted(
            JobRequest::Model(Box::new(invocation)),
            ManagedCommandActor::Model(lease),
            cancellation,
            reservation,
        )
    }

    fn submit_admitted(
        self: &Arc<Self>,
        request: JobRequest,
        actor: ManagedCommandActor,
        cancellation: CancellationToken,
        reservation: Arc<Reservation>,
    ) -> Result<Response, Error> {
        let reply = Arc::new(Reply::new(
            reservation.clone(),
            ManagedMailboxWake(Arc::downgrade(self)),
        ));
        let job = ManagedMailboxJob {
            request: Some(request),
            actor: Some(actor),
            cancellation,
            reply: reply.clone(),
            reservation,
        };
        let waker = {
            let mut state = self.state.lock().map_err(|_| Error::Unavailable)?;
            if state.closed {
                return Err(Error::Unavailable);
            }
            state.replies.retain(|reply| reply.strong_count() != 0);
            state.replies.push(Arc::downgrade(&reply));
            state.queue.push_back(job);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(Response::new(reply))
    }
}

#[derive(Clone)]
pub(crate) struct ManagedMailboxWake(Weak<Shared>);
impl ManagedMailboxWake {
    pub(crate) fn wake(&self) {
        let waker = self.0.upgrade().and_then(|shared| {
            shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .waker
                .take()
        });
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// Admitted request custody, not journal acceptance or permission to execute later.
pub(crate) struct ManagedMailboxJob {
    request: Option<JobRequest>,
    actor: Option<ManagedCommandActor>,
    cancellation: CancellationToken,
    reply: Arc<Reply>,
    reservation: Arc<Reservation>,
}
enum JobRequest {
    Model(Box<ManagedSubagentInvocation>),
    Human(Box<ManagedSubagentCommand>),
}
impl ManagedMailboxJob {
    pub(crate) fn command(&self) -> &ManagedSubagentCommand {
        match self.request.as_ref().expect("live job") {
            JobRequest::Model(invocation) => invocation.command(),
            JobRequest::Human(command) => command,
        }
    }
    pub(crate) fn context(&self) -> Option<&ToolContext> {
        match self.request.as_ref().expect("live job") {
            JobRequest::Model(invocation) => Some(invocation.context()),
            JobRequest::Human(_) => None,
        }
    }
    pub(crate) fn lease(&self) -> &ManagedCommandActor {
        self.actor.as_ref().expect("live job")
    }
    pub(crate) fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
    pub(crate) fn observer_gone(&self) -> bool {
        self.reply.observer_gone()
    }
    /// Result ownership transfers to the existing bounded core codec on poll Ready.
    pub(crate) fn complete(mut self, result: Result<ManagedSubagentResult, Error>) {
        let result = response::bounded_result(result);
        // The reservation covers both payloads while result normalization runs.
        drop(self.request.take());
        drop(self.actor.take());
        self.reply.finish(result);
    }
}
impl Drop for ManagedMailboxJob {
    fn drop(&mut self) {
        // A discarded job is rejected, not implicitly executed or retried.
        self.reply.finish(Err(Error::Unavailable));
        let _ = &self.reservation;
    }
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(impl fmt::Debug for $ty {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
redacted_debug!(
    ManagedMailbox,
    ManagedMailboxRequester,
    ManagedMailboxWake,
    ManagedMailboxJob
);
