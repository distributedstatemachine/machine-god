//! Continuous terminal pumping on an explicitly owned blocking worker.
//! Construction and unpolled requests are inert; this module spawns no threads.

use crate::terminal_monitor::{MAX_MONITOR_FEED_BYTES, TerminalWaitOutcome};
use crate::terminal_profile::TerminalProfileBudget;
use crate::terminal_profile_store::TerminalProfileStore;
use crate::terminal_registry::{
    MAX_RESIDENT_TERMINALS, TerminalRegistry, TerminalRegistryError, TerminalRegistryFailure,
    TerminalRegistryStep,
};
use crate::terminal_session::TerminalSessionBackend;
use crate::terminal_wait::{TerminalWaitCompletion, TerminalWaitCoordinator};
use crate::terminal_write_completion::TerminalWriteCoordinator;
use machine_god_core::{CancellationToken, Cancelled, TerminalClosePolicy, TerminalLifecycle};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

const MAX_REQUESTS: usize = 32;
const PUMP_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalOwnerError {
    Busy,
    Closed,
    Cancelled,
    Panicked,
    ProfileRequired,
    WaitAttention,
    Registry(TerminalRegistryError),
}
type Result<T> = std::result::Result<T, TerminalOwnerError>;

fn catch_callback<T>(callback: impl FnOnce() -> T) -> std::result::Result<T, ()> {
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(value) => Ok(value),
        Err(payload) => {
            // A caught opaque payload can itself have a panicking destructor.
            // Suppression must not execute that arbitrary callback during cleanup.
            // This exceptional path deliberately retains the opaque allocation.
            std::mem::forget(payload);
            Err(())
        }
    }
}

trait Job<B: TerminalSessionBackend, S>: Send {
    /// False means an operation panicked and this owner must stop.
    fn execute(
        self: Box<Self>,
        registry: &mut TerminalRegistry<B>,
        waits: &mut TerminalWaitCoordinator,
        writes: &mut TerminalWriteCoordinator,
        now_ms: i64,
        profile: Option<(&TerminalProfileStore, &TerminalProfileBudget)>,
        state: &mut S,
    ) -> bool;
}
enum Message<B: TerminalSessionBackend, S> {
    Job(Box<dyn Job<B, S>>),
    Wake,
}
struct Shared<B: TerminalSessionBackend, S> {
    sender: SyncSender<Message<B, S>>,
    closing: AtomicBool,
    clients: AtomicUsize,
    requests: Arc<AtomicUsize>,
    callback_panicked: Arc<AtomicBool>,
}
impl<B: TerminalSessionBackend, S> Shared<B, S> {
    fn close(&self) {
        if !self.closing.swap(true, Ordering::AcqRel) {
            let _ = self.sender.try_send(Message::Wake);
        }
    }
}
struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Reply<T> {
    result: Option<Result<T>>,
    executed: bool,
    waker: Option<Waker>,
}
fn complete<T>(reply: &Mutex<Reply<T>>, value: Result<T>, executed: bool) -> bool {
    let wake = {
        let mut reply = reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reply.result = Some(value);
        reply.executed = executed;
        reply.waker.take()
    };
    // Never invoke user-controlled wakers while holding a synchronization lock.
    if let Some(waker) = wake {
        return catch_callback(|| waker.wake()).is_ok();
    }
    true
}
type Operation<B, T, S> = Box<
    dyn FnOnce(
            &mut TerminalRegistry<B>,
            &mut TerminalWaitCoordinator,
            &mut TerminalWriteCoordinator,
            i64,
            &CancellationToken,
            Option<(&TerminalProfileStore, &TerminalProfileBudget)>,
            &mut S,
        ) -> Result<T>
        + Send,
>;
struct Request<B: TerminalSessionBackend, T, S> {
    operation: Option<Operation<B, T, S>>,
    reply: Arc<Mutex<Reply<T>>>,
    caller: CancellationToken,
    cancellation: CancellationToken,
    _permit: Arc<Permit>,
    callback_panicked: Arc<AtomicBool>,
    completed: bool,
}
impl<B: TerminalSessionBackend, T: Send, S> Job<B, S> for Request<B, T, S> {
    fn execute(
        mut self: Box<Self>,
        registry: &mut TerminalRegistry<B>,
        waits: &mut TerminalWaitCoordinator,
        writes: &mut TerminalWriteCoordinator,
        now_ms: i64,
        profile: Option<(&TerminalProfileStore, &TerminalProfileBudget)>,
        state: &mut S,
    ) -> bool {
        if self.caller.is_cancelled() {
            self.cancellation.cancel();
        }
        let executed = !self.cancellation.is_cancelled();
        let result = if executed {
            let operation = self.operation.take().expect("request executed once");
            catch_callback(|| {
                operation(
                    registry,
                    waits,
                    writes,
                    now_ms,
                    &self.cancellation,
                    profile,
                    state,
                )
            })
            .unwrap_or(Err(TerminalOwnerError::Panicked))
        } else {
            Err(TerminalOwnerError::Cancelled)
        };
        let keep_running = !matches!(&result, Err(TerminalOwnerError::Panicked));
        self.completed = true;
        let woke = complete(&self.reply, result, executed);
        if !woke {
            self.callback_panicked.store(true, Ordering::Release);
        }
        keep_running && woke
    }
}
impl<B: TerminalSessionBackend, T, S> Drop for Request<B, T, S> {
    fn drop(&mut self) {
        if !self.completed && !complete(&self.reply, Err(TerminalOwnerError::Closed), false) {
            // Contain wake separately from captured-value Drop: if both panic,
            // unwinding one through the other would abort before any outer catch.
            self.callback_panicked.store(true, Ordering::Release);
        }
    }
}

/// One short, typed dispatch borrow. State is not stored in request futures;
/// only the owning loop supplies it. Native authority must not be returned in
/// reply values, and profile guards must end before the callback returns.
pub(crate) struct TerminalOwnerContext<'a, B: TerminalSessionBackend, S> {
    pub(crate) registry: &'a mut TerminalRegistry<B>,
    pub(crate) store: &'a TerminalProfileStore,
    pub(crate) budget: &'a TerminalProfileBudget,
    pub(crate) waits: &'a mut TerminalWaitCoordinator,
    pub(crate) writes: &'a mut TerminalWriteCoordinator,
    pub(crate) state: &'a mut S,
    pub(crate) now_ms: i64,
    pub(crate) cancellation: &'a CancellationToken,
}

/// The host owns this handle, not an individual tool future. Last-handle drop
/// requests shutdown; it never waits for process cleanup on the polling thread.
pub(crate) struct TerminalOwnerHandle<B: TerminalSessionBackend, S = ()> {
    shared: Arc<Shared<B, S>>,
    owns_lifetime: bool,
}
impl<B: TerminalSessionBackend, S> Clone for TerminalOwnerHandle<B, S> {
    fn clone(&self) -> Self {
        if self.owns_lifetime {
            self.shared.clients.fetch_add(1, Ordering::Relaxed);
        }
        Self {
            shared: Arc::clone(&self.shared),
            owns_lifetime: self.owns_lifetime,
        }
    }
}
impl<B: TerminalSessionBackend, S> Drop for TerminalOwnerHandle<B, S> {
    fn drop(&mut self) {
        if self.owns_lifetime && self.shared.clients.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.close();
        }
    }
}
impl<B: TerminalSessionBackend + 'static, S: 'static> TerminalOwnerHandle<B, S> {
    /// Request authority without a host lifetime vote, for owned effect workers.
    /// Closing the last owning handle still rejects every subsequent request.
    pub(crate) fn requester(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            owns_lifetime: false,
        }
    }
    pub(crate) fn request_with_context<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(TerminalOwnerContext<'_, B, S>) -> T + Send + 'static,
    ) -> TerminalOwnerFuture<B, T, S> {
        self.request_inner(
            caller,
            Box::new(
                move |registry, waits, writes, now_ms, cancellation, profile, state| {
                    let (store, budget) = profile.ok_or(TerminalOwnerError::ProfileRequired)?;
                    Ok(operation(TerminalOwnerContext {
                        registry,
                        store,
                        budget,
                        waits,
                        writes,
                        state,
                        now_ms,
                        cancellation,
                    }))
                },
            ),
        )
    }

    /// Operations must be bounded, authorized host commands. No tool-supplied
    /// closure, process backend, or unbounded external probe belongs here.
    #[cfg(test)]
    pub(crate) fn request<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(&mut TerminalRegistry<B>, i64, &CancellationToken) -> T + Send + 'static,
    ) -> TerminalOwnerFuture<B, T, S> {
        self.request_inner(
            caller,
            Box::new(move |registry, _, _, now_ms, cancellation, _, _| {
                Ok(operation(registry, now_ms, cancellation))
            }),
        )
    }

    /// Supplies this worker's exact profile authority to a bounded authorized
    /// mutation. The operation acquires its own short transaction; neither
    /// borrowed authority nor its transaction can escape through the result.
    /// Completion wakes the caller only after the operation returns and drops
    /// all transaction guards, including when the operation panics.
    #[cfg(test)]
    pub(crate) fn request_with_profile<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(
            &mut TerminalRegistry<B>,
            &TerminalProfileStore,
            &TerminalProfileBudget,
            i64,
            &CancellationToken,
        ) -> T
        + Send
        + 'static,
    ) -> TerminalOwnerFuture<B, T, S> {
        self.request_inner(
            caller,
            Box::new(move |registry, _, _, now_ms, cancellation, profile, _| {
                let (store, budget) = profile.ok_or(TerminalOwnerError::ProfileRequired)?;
                Ok(operation(registry, store, budget, now_ms, cancellation))
            }),
        )
    }

    /// Register or cancel attention waits in a short owner request. The
    /// returned wait future is independent of this request's bounded reply;
    /// never block this callback waiting for a terminal condition.
    #[cfg(test)]
    pub(crate) fn request_with_waits<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(
            &mut TerminalRegistry<B>,
            &TerminalProfileStore,
            &TerminalProfileBudget,
            &mut TerminalWaitCoordinator,
            i64,
            &CancellationToken,
        ) -> T
        + Send
        + 'static,
    ) -> TerminalOwnerFuture<B, T, S> {
        self.request_inner(
            caller,
            Box::new(
                move |registry, waits, _, now_ms, cancellation, profile, _| {
                    let (store, budget) = profile.ok_or(TerminalOwnerError::ProfileRequired)?;
                    Ok(operation(
                        registry,
                        store,
                        budget,
                        waits,
                        now_ms,
                        cancellation,
                    ))
                },
            ),
        )
    }

    /// Submit one authorized write after reserving completion capacity. The
    /// returned future observes owned input; this callback never waits for it.
    #[cfg(test)]
    pub(crate) fn request_with_writes<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(
            &mut TerminalRegistry<B>,
            &TerminalProfileStore,
            &TerminalProfileBudget,
            &mut TerminalWriteCoordinator,
            i64,
            &CancellationToken,
        ) -> T
        + Send
        + 'static,
    ) -> TerminalOwnerFuture<B, T, S> {
        self.request_inner(
            caller,
            Box::new(
                move |registry, _, writes, now_ms, cancellation, profile, _| {
                    let (store, budget) = profile.ok_or(TerminalOwnerError::ProfileRequired)?;
                    Ok(operation(
                        registry,
                        store,
                        budget,
                        writes,
                        now_ms,
                        cancellation,
                    ))
                },
            ),
        )
    }

    fn request_inner<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: Operation<B, T, S>,
    ) -> TerminalOwnerFuture<B, T, S> {
        TerminalOwnerFuture {
            shared: Arc::clone(&self.shared),
            operation: Some(operation),
            reply: Arc::new(Mutex::new(Reply {
                result: None,
                executed: false,
                waker: None,
            })),
            caller_wait: Some(caller.cancelled()),
            caller,
            cancellation: CancellationToken::new(),
            permit: None,
            submitted: false,
            finished: false,
        }
    }
    pub(crate) fn shutdown(&self) {
        self.shared.close();
    }
}
pub(crate) struct TerminalOwnerFuture<B: TerminalSessionBackend, T, S = ()> {
    shared: Arc<Shared<B, S>>,
    operation: Option<Operation<B, T, S>>,
    reply: Arc<Mutex<Reply<T>>>,
    caller_wait: Option<Cancelled>,
    caller: CancellationToken,
    cancellation: CancellationToken,
    permit: Option<Arc<Permit>>,
    submitted: bool,
    finished: bool,
}
impl<B: TerminalSessionBackend, T, S> TerminalOwnerFuture<B, T, S> {
    pub(crate) fn operation_executed(&self) -> bool {
        self.reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .executed
    }

    /// Observe an executed operation without admitting work or invoking a waker.
    /// Queue rejection and pre-execution cancellation are not operation receipts.
    pub(crate) fn take_executed_reply(&mut self) -> Option<Result<T>> {
        let (result, old) = {
            let mut reply = self
                .reply
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !reply.executed {
                return None;
            }
            (reply.result.take(), reply.waker.take())
        };
        if result.is_some() {
            self.finished = true;
            self.caller_wait = None;
            self.permit = None;
        }
        // User-controlled waker destruction must remain outside the reply lock.
        drop(old);
        result
    }
}
impl<B: TerminalSessionBackend + 'static, T: Send + 'static, S: 'static> Future
    for TerminalOwnerFuture<B, T, S>
{
    type Output = Result<T>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(!this.finished, "completed terminal request polled again");
        if this
            .caller_wait
            .as_mut()
            .is_some_and(|wait| Pin::new(wait).poll(cx).is_ready())
        {
            this.cancellation.cancel();
            this.caller_wait = None;
        }
        if !this.submitted {
            let error = if this.cancellation.is_cancelled() {
                Some(TerminalOwnerError::Cancelled)
            } else if this.shared.closing.load(Ordering::Acquire) {
                Some(TerminalOwnerError::Closed)
            } else if this
                .shared
                .requests
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < MAX_REQUESTS).then_some(n + 1)
                })
                .is_err()
            {
                Some(TerminalOwnerError::Busy)
            } else {
                None
            };
            if let Some(error) = error {
                this.finished = true;
                this.caller_wait = None;
                this.operation = None;
                return Poll::Ready(Err(error));
            }
            let permit = Arc::new(Permit(Arc::clone(&this.shared.requests)));
            this.permit = Some(Arc::clone(&permit));
            let job = Request {
                operation: this.operation.take(),
                reply: Arc::clone(&this.reply),
                caller: this.caller.clone(),
                cancellation: this.cancellation.clone(),
                _permit: permit,
                callback_panicked: Arc::clone(&this.shared.callback_panicked),
                completed: false,
            };
            this.submitted = true;
            if let Err(error) = this.shared.sender.try_send(Message::Job(Box::new(job))) {
                // Dropping the rejected request resolves its reply, outside any lock.
                drop(error);
            }
        }
        let incoming = cx.waker().clone();
        let (result, old) = {
            let mut reply = this
                .reply
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = reply.result.take();
            let old = if result.is_some() {
                Some(incoming)
            } else {
                reply.waker.replace(incoming)
            };
            (result, old)
        };
        drop(old);
        match result {
            Some(result) => {
                this.finished = true;
                this.caller_wait = None;
                this.permit = None;
                Poll::Ready(result)
            }
            None => Poll::Pending,
        }
    }
}
impl<B: TerminalSessionBackend, T, S> Drop for TerminalOwnerFuture<B, T, S> {
    fn drop(&mut self) {
        if !self.finished {
            self.cancellation.cancel();
        }
        let old = self
            .reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waker
            .take();
        drop(old);
    }
}

pub(crate) struct TerminalOwnerLoop<B: TerminalSessionBackend, S = ()> {
    shared: Arc<Shared<B, S>>,
    receiver: Receiver<Message<B, S>>,
}
pub(crate) struct TerminalOwnerExit {
    pub(crate) error: Option<TerminalOwnerError>,
    pub(crate) shutdown: std::result::Result<Vec<TerminalRegistryFailure>, TerminalRegistryError>,
}

#[derive(Clone, Copy)]
enum Persistence<'a> {
    Profile(&'a TerminalProfileStore, &'a TerminalProfileBudget),
    #[cfg(test)]
    Unmetered,
}

impl Persistence<'_> {
    #[cfg_attr(
        not(test),
        allow(
            clippy::unnecessary_wraps,
            reason = "shared owner supports unmetered test authority"
        )
    )]
    fn authority(&self) -> Option<(&TerminalProfileStore, &TerminalProfileBudget)> {
        match self {
            Self::Profile(store, budget) => Some((store, budget)),
            #[cfg(test)]
            Self::Unmetered => None,
        }
    }

    fn pump<B: TerminalSessionBackend>(
        &self,
        registry: &mut TerminalRegistry<B>,
        now_ms: i64,
    ) -> std::result::Result<Vec<TerminalRegistryStep>, TerminalRegistryError> {
        match self {
            Self::Profile(store, budget) => {
                registry.pump_with_profile(store, budget, now_ms, MAX_RESIDENT_TERMINALS)
            }
            #[cfg(test)]
            Self::Unmetered => registry.pump(now_ms, MAX_RESIDENT_TERMINALS),
        }
    }
    fn shutdown<B: TerminalSessionBackend>(
        &self,
        registry: &mut TerminalRegistry<B>,
    ) -> std::result::Result<Vec<TerminalRegistryFailure>, TerminalRegistryError> {
        let now_ms = registry.minimum_time_ms();
        match self {
            Self::Profile(store, budget) => {
                registry.shutdown_with_profile(store, budget, now_ms, TerminalClosePolicy::Force)
            }
            #[cfg(test)]
            Self::Unmetered => registry.shutdown(now_ms, TerminalClosePolicy::Force),
        }
    }

    /// Registry delegation owns the short transaction and drops it before
    /// returning. The caller publishes no reply while persistence is held.
    fn finish_wait<B: TerminalSessionBackend>(
        self,
        registry: &mut TerminalRegistry<B>,
        completion: &TerminalWaitCompletion,
    ) -> bool {
        match self {
            Self::Profile(store, budget) => registry
                .finish_attention_with(
                    store,
                    budget,
                    &completion.identity.owner,
                    &completion.identity.session,
                    completion.identity.actor,
                    completion.identity.writer,
                    registry.minimum_time_ms(),
                    completion.outcome == TerminalWaitOutcome::Cancelled,
                )
                .is_ok(),
            #[cfg(test)]
            Self::Unmetered => false,
        }
    }
}

/// One bounded durable page per pending wait per scheduler turn. The registry
/// pump can commit extra drain chunks beyond `step.output`; reading from each
/// saved cursor observes those bytes exactly once, including inactive records.
fn observe_waits<B: TerminalSessionBackend>(
    registry: &TerminalRegistry<B>,
    waits: &mut TerminalWaitCoordinator,
) -> bool {
    let mut catching_up = false;
    for observation in waits.observations() {
        let owner = &observation.identity.owner;
        let session = &observation.identity.session;
        let Ok((mut context, last_output_ms, process)) = registry.wait_observation(owner, session)
        else {
            waits.lose(owner, session);
            continue;
        };
        context.now_ms = registry.minimum_time_ms();
        // Known termination/loss outranks literal/quiet conditions, regardless
        // of remaining tail bytes. No matcher result is inferred from skipping.
        let terminal = process.is_some() || context.lifecycle == TerminalLifecycle::Lost;
        let result = if terminal || observation.cursor == context.cursor {
            waits.advance_one(&observation, &[], &context, last_output_ms, process, true)
        } else {
            match registry.read(owner, session, &observation.cursor, MAX_MONITOR_FEED_BYTES) {
                Ok(page)
                    if page.gap.is_none()
                        && page.next > observation.cursor
                        && page.next <= context.cursor =>
                {
                    let caught_up = page.next == context.cursor;
                    catching_up |= !caught_up;
                    context.cursor = page.next;
                    waits.advance_one(
                        &observation,
                        &page.bytes,
                        &context,
                        last_output_ms,
                        process,
                        caught_up,
                    )
                }
                _ => {
                    waits.lose(owner, session);
                    continue;
                }
            }
        };
        if result.is_err() {
            waits.lose(owner, session);
        }
    }
    catching_up
}

/// Returns (waker panicked, final attention failed). Failed normal publication
/// drops its retry token without invoking callbacks or discarding the outcome.
fn publish_waits<B: TerminalSessionBackend>(
    registry: &mut TerminalRegistry<B>,
    waits: &mut TerminalWaitCoordinator,
    persistence: Persistence<'_>,
    final_attempt: bool,
) -> (bool, bool) {
    let mut panicked = false;
    let mut failed = false;
    for completion in waits.take_ready() {
        let finished = if waits.has_pending_attention(&completion.identity, completion.id) {
            true
        } else if let Ok(finished) =
            catch_callback(|| persistence.finish_wait(registry, &completion))
        {
            finished
        } else {
            panicked = true;
            false
        };
        if finished {
            panicked |= !completion.publish();
        } else if final_attempt {
            failed = true;
            panicked |= !completion.publish_failed();
        }
    }
    (panicked, failed)
}
impl<B: TerminalSessionBackend, S> TerminalOwnerLoop<B, S> {
    /// A rejected request can run user waker or captured-value destructors.
    /// Contain each one independently so every remaining reply is resolved and
    /// neither normal shutdown nor unwinding Drop can skip native cleanup.
    fn reject_pending(&self) -> bool {
        let mut panicked = false;
        while let Ok(message) = self.receiver.try_recv() {
            panicked |= catch_callback(|| drop(message)).is_err();
        }
        panicked
    }
    pub(crate) fn new() -> (Self, TerminalOwnerHandle<B, S>) {
        let (sender, receiver) = sync_channel(MAX_REQUESTS);
        let shared = Arc::new(Shared {
            sender,
            closing: AtomicBool::new(false),
            clients: AtomicUsize::new(1),
            requests: Arc::new(AtomicUsize::new(0)),
            callback_panicked: Arc::new(AtomicBool::new(false)),
        });
        (
            Self {
                shared: Arc::clone(&shared),
                receiver,
            },
            TerminalOwnerHandle {
                shared,
                owns_lifetime: true,
            },
        )
    }
}

impl<B: TerminalSessionBackend> TerminalOwnerLoop<B> {
    /// Runs only on the host's owned blocking worker. It borrows the registry so
    /// the same worker can retain failed histories after inspecting the exit.
    /// The observer consumes bounded output/probe descriptions synchronously;
    /// it must not execute unbounded probes or enqueue unbounded output.
    #[cfg(test)]
    pub(crate) fn run(
        self,
        registry: &mut TerminalRegistry<B>,
        clock: impl FnMut() -> i64,
        mut observer: impl FnMut(Vec<TerminalRegistryStep>),
    ) -> TerminalOwnerExit {
        self.run_inner(
            registry,
            Persistence::Unmetered,
            &mut (),
            clock,
            |(), steps| observer(steps),
        )
    }

    /// Profile transactions exist only inside dispatch, never across observers,
    /// job callbacks/reply wakes, or the blocking request wait.
    #[cfg(test)]
    pub(crate) fn run_with_profile(
        self,
        registry: &mut TerminalRegistry<B>,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        clock: impl FnMut() -> i64,
        mut observer: impl FnMut(Vec<TerminalRegistryStep>),
    ) -> TerminalOwnerExit {
        self.run_with_profile_and_state(registry, store, budget, &mut (), clock, |(), steps| {
            observer(steps);
        })
    }
}

impl<B: TerminalSessionBackend, S> TerminalOwnerLoop<B, S> {
    /// The state borrow remains on this worker across requests and observers.
    /// The caller retains state until registry destruction and cleanup retries.
    pub(crate) fn run_with_profile_and_state(
        self,
        registry: &mut TerminalRegistry<B>,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        state: &mut S,
        clock: impl FnMut() -> i64,
        observer: impl FnMut(&mut S, Vec<TerminalRegistryStep>),
    ) -> TerminalOwnerExit {
        self.run_inner(
            registry,
            Persistence::Profile(store, budget),
            state,
            clock,
            observer,
        )
    }

    fn run_inner(
        self,
        registry: &mut TerminalRegistry<B>,
        persistence: Persistence<'_>,
        state: &mut S,
        mut clock: impl FnMut() -> i64,
        mut observer: impl FnMut(&mut S, Vec<TerminalRegistryStep>),
    ) -> TerminalOwnerExit {
        let mut error = None;
        let mut waits = TerminalWaitCoordinator::new();
        let mut writes = TerminalWriteCoordinator::new();
        let mut deadline = Instant::now();
        let execution = catch_callback(|| {
            while !self.shared.closing.load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    let output_ready = match persistence.pump(registry, clock()) {
                        Ok(steps) => {
                            let output_ready = steps.iter().any(|step| {
                                step.result
                                    .as_ref()
                                    .is_ok_and(|step| !step.output.is_empty())
                            });
                            let catching_up = observe_waits(registry, &mut waits);
                            let (panicked, _) =
                                publish_waits(registry, &mut waits, persistence, false);
                            let writes_woke = writes.observe(registry);
                            if panicked || !writes_woke {
                                error = Some(TerminalOwnerError::Panicked);
                                break;
                            }
                            observer(state, steps);
                            output_ready || catching_up
                        }
                        Err(failure) => {
                            error = Some(TerminalOwnerError::Registry(failure));
                            break;
                        }
                    };
                    deadline = Instant::now()
                        + if output_ready {
                            Duration::ZERO
                        } else {
                            waits.next_deadline().map_or(PUMP_INTERVAL, |next| {
                                let milliseconds = next.saturating_sub(registry.minimum_time_ms());
                                Duration::from_millis(u64::try_from(milliseconds).unwrap_or(0))
                                    .min(PUMP_INTERVAL)
                            })
                        };
                }
                match self
                    .receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(Message::Job(job)) => {
                        if self.shared.closing.load(Ordering::Acquire) {
                            drop(job);
                            break;
                        }
                        // Commands use the last validated registry time, preventing a
                        // second unvalidated clock read from preceding native effects.
                        if !job.execute(
                            registry,
                            &mut waits,
                            &mut writes,
                            registry.minimum_time_ms(),
                            persistence.authority(),
                            state,
                        ) {
                            error = Some(TerminalOwnerError::Panicked);
                            break;
                        }
                        // A new wait, cancellation or committed mutation must
                        // be observed promptly without blocking registry pumps.
                        deadline = Instant::now();
                    }
                    Ok(Message::Wake) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        if execution.is_err() {
            error = Some(TerminalOwnerError::Panicked);
        }
        self.finish(registry, persistence, &mut waits, &mut writes, error)
    }

    fn finish(
        &self,
        registry: &mut TerminalRegistry<B>,
        persistence: Persistence<'_>,
        waits: &mut TerminalWaitCoordinator,
        writes: &mut TerminalWriteCoordinator,
        mut error: Option<TerminalOwnerError>,
    ) -> TerminalOwnerExit {
        self.shared.close();
        // Resolve queued requests before native shutdown; no rejected operation runs.
        let rejection_panicked = self.reject_pending();
        if rejection_panicked || self.shared.callback_panicked.load(Ordering::Acquire) {
            error = Some(TerminalOwnerError::Panicked);
        }
        let shutdown = catch_callback(|| persistence.shutdown(registry)).unwrap_or_else(|()| {
            error = Some(TerminalOwnerError::Panicked);
            Err(TerminalRegistryError::Invalid)
        });
        observe_waits(registry, waits);
        waits.close();
        let (wait_panicked, attention_failed) = publish_waits(registry, waits, persistence, true);
        let writes_woke = writes.observe(registry);
        let writes_closed = writes.close();
        if wait_panicked || !writes_woke || !writes_closed {
            error = Some(TerminalOwnerError::Panicked);
        } else if attention_failed && error.is_none() {
            error = Some(TerminalOwnerError::WaitAttention);
        }
        TerminalOwnerExit { error, shutdown }
    }
}
impl<B: TerminalSessionBackend, S> Drop for TerminalOwnerLoop<B, S> {
    fn drop(&mut self) {
        self.shared.close();
        self.reject_pending();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_input::{BackgroundInputReceipt, BackgroundInputStatus};
    use crate::terminal_catalog::owner_name;
    use crate::terminal_history::TerminalHistory;
    use crate::terminal_input::TerminalWriterId;
    use crate::terminal_journal::TerminalJournalLimits;
    use crate::terminal_profile::{TerminalProfileLimits, TerminalProfileMutationContext};
    use crate::terminal_pty::{TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
    use crate::terminal_session::TerminalSession;
    use crate::terminal_session_record::test_metadata;
    use crate::terminal_wait::{
        TerminalWaitFuture, TerminalWaitHistory, TerminalWaitIdentity, TerminalWaitReceipt,
    };
    use machine_god_core::{
        BackgroundOutputOwner, SessionId, SessionIncarnationId, TerminalActorRole,
        TerminalDimensions, TerminalReturnCondition, TerminalSessionId, TerminalSignal,
        TerminalWaitRequest,
    };
    use std::collections::VecDeque;
    use std::num::NonZeroU64;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    struct ReentrantWake {
        reply: std::sync::Weak<Mutex<Reply<()>>>,
        woke: AtomicBool,
    }
    impl std::task::Wake for ReentrantWake {
        fn wake(self: Arc<Self>) {
            let reply = self.reply.upgrade().unwrap();
            assert!(reply.try_lock().unwrap().result.is_some());
            self.woke.store(true, Ordering::Release);
        }
    }
    #[test]
    fn completion_wakes_outside_reply_lock() {
        let reply = Arc::new(Mutex::new(Reply {
            result: None,
            executed: false,
            waker: None,
        }));
        let wake = Arc::new(ReentrantWake {
            reply: Arc::downgrade(&reply),
            woke: AtomicBool::new(false),
        });
        reply.lock().unwrap().waker = Some(Waker::from(Arc::clone(&wake)));
        complete(&reply, Ok(()), true);
        assert!(wake.woke.load(Ordering::Acquire));
    }

    #[test]
    fn executed_reply_inspection_preserves_errors_without_admitting_requests() {
        let (_owner, handle) = TerminalOwnerLoop::<Backend>::new();
        let mut future = handle.request(CancellationToken::new(), |_, _, _| ());
        assert_eq!(future.take_executed_reply(), None);
        assert!(!future.submitted);
        assert_eq!(handle.shared.requests.load(Ordering::Acquire), 0);
        assert!(complete(
            &future.reply,
            Err(TerminalOwnerError::Closed),
            false
        ));
        assert_eq!(future.take_executed_reply(), None);
        assert!(!future.finished);
        assert!(complete(
            &future.reply,
            Err(TerminalOwnerError::ProfileRequired),
            true
        ));
        assert_eq!(
            future.take_executed_reply(),
            Some(Err(TerminalOwnerError::ProfileRequired))
        );
        assert!(future.finished);
        assert!(future.caller_wait.is_none());
        assert!(future.permit.is_none());
    }

    #[derive(Default)]
    struct NativeState {
        output: VecDeque<Vec<u8>>,
        closes: usize,
        exited: bool,
        write_limit: Option<usize>,
        written: Vec<u8>,
    }
    struct Backend(Arc<Mutex<NativeState>>);
    impl TerminalSessionBackend for Backend {
        fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            let bytes = self
                .0
                .lock()
                .unwrap()
                .output
                .pop_front()
                .unwrap_or_default();
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok(TerminalPtyRead {
                bytes_read: bytes.len(),
                closed: false,
            })
        }
        fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            let mut state = self.0.lock().unwrap();
            let accepted = bytes.len().min(state.write_limit.unwrap_or(usize::MAX));
            state.written.extend_from_slice(&bytes[..accepted]);
            Ok(BackgroundInputReceipt::new(
                accepted,
                false,
                if accepted == bytes.len() {
                    BackgroundInputStatus::Written
                } else {
                    BackgroundInputStatus::Backpressure
                },
            ))
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            Ok(if self.0.lock().unwrap().exited {
                TerminalPtyStatus::Exited(7)
            } else {
                TerminalPtyStatus::Running
            })
        }
        fn resize(&mut self, _: &TerminalDimensions) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal(&mut self, _: TerminalSignal) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal_may_discard_output(&self) -> bool {
            false
        }
        fn close(
            &mut self,
            _: bool,
            output: &mut dyn FnMut(&[u8]),
        ) -> std::result::Result<TerminalPtyClose, ()> {
            let mut state = self.0.lock().unwrap();
            state.closes += 1;
            for bytes in state.output.drain(..) {
                output(&bytes);
            }
            Ok(TerminalPtyClose {
                status: TerminalPtyStatus::Exited(7),
                output_incomplete: false,
            })
        }
    }
    struct Fixture {
        path: PathBuf,
        store: Arc<TerminalProfileStore>,
        budget: TerminalProfileBudget,
        state: Arc<Mutex<NativeState>>,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-owner-wait-{:032x}",
                u128::from_le_bytes(random)
            ));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            let fd = rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            )
            .unwrap();
            Self {
                path,
                store: Arc::new(TerminalProfileStore::prepare(fd).unwrap()),
                budget: TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap(),
                state: Arc::new(Mutex::new(NativeState::default())),
            }
        }
        fn registry(&self, who: &TerminalWaitIdentity) -> TerminalRegistry<Backend> {
            self.registry_with_limits(who, TerminalJournalLimits::default())
        }
        fn registry_with_limits(
            &self,
            who: &TerminalWaitIdentity,
            limits: TerminalJournalLimits,
        ) -> TerminalRegistry<Backend> {
            let mut transaction = self.store.transaction().unwrap();
            let mut catalog = transaction
                .prepare_catalog("/workspace".into(), who.owner.clone())
                .unwrap();
            drop(
                transaction
                    .create_session(&mut catalog, &who.session)
                    .unwrap(),
            );
            let completion = self
                .budget
                .create_journal(
                    &mut transaction,
                    catalog.namespace_key(),
                    &who.session,
                    limits,
                )
                .unwrap();
            completion.accounting.unwrap();
            let journal = completion.operation.unwrap();
            let mut persistence = TerminalProfileMutationContext::new(
                &mut transaction,
                self.budget,
                catalog.namespace_key(),
            );
            let history = TerminalHistory::create_with(
                &mut persistence,
                journal,
                &TerminalDimensions::new(3, 20).unwrap(),
            )
            .unwrap();
            let mut session = TerminalSession::new_with(
                &mut persistence,
                Backend(Arc::clone(&self.state)),
                history,
                who.owner.clone(),
                who.session.clone(),
                test_metadata(),
                0,
            )
            .unwrap();
            session.shell_ready_with(&mut persistence, 0).unwrap();
            let mut registry = TerminalRegistry::new("/workspace".into()).unwrap();
            registry
                .start(who.owner.clone(), who.session.clone(), || Ok(session))
                .unwrap();
            registry
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn identity() -> TerminalWaitIdentity {
        TerminalWaitIdentity {
            owner: BackgroundOutputOwner::new(
                SessionId::new("owner").unwrap(),
                SessionIncarnationId::new("incarnation").unwrap(),
            ),
            session: TerminalSessionId::new("terminal").unwrap(),
            actor: TerminalActorRole::Agent,
            writer: TerminalWriterId::new(NonZeroU64::new(1).unwrap()),
        }
    }
    fn poll_request<T: Send + 'static>(
        request: &mut TerminalOwnerFuture<Backend, T>,
    ) -> Poll<Result<T>> {
        Pin::new(request).poll(&mut Context::from_waker(Waker::noop()))
    }
    fn poll_wait(wait: &mut TerminalWaitFuture) -> Poll<TerminalWaitReceipt> {
        Pin::new(wait).poll(&mut Context::from_waker(Waker::noop()))
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit owner callback fixture inputs"
    )]
    fn admit(
        registry: &mut TerminalRegistry<Backend>,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        waits: &mut TerminalWaitCoordinator,
        who: TerminalWaitIdentity,
        condition: TerminalReturnCondition,
        now_ms: i64,
        cancellation: CancellationToken,
    ) -> TerminalWaitFuture {
        let (context, last_output, _) =
            registry.wait_observation(&who.owner, &who.session).unwrap();
        let namespace = owner_name("/workspace", &who.owner);
        let mut transaction = store.transaction().unwrap();
        let mut persistence =
            TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
        registry
            .live_mut(&who.owner, &who.session)
            .unwrap()
            .begin_attention_with(&mut persistence, &who.owner, who.actor, who.writer, now_ms)
            .unwrap();
        waits
            .register(
                who,
                TerminalWaitRequest {
                    condition,
                    safety_ceiling_ms: 100,
                },
                &context,
                last_output,
                TerminalWaitHistory::default(),
                cancellation,
            )
            .unwrap()
            .1
    }

    struct ProfileWake {
        store: Arc<TerminalProfileStore>,
        calls: AtomicUsize,
    }
    impl std::task::Wake for ProfileWake {
        fn wake(self: Arc<Self>) {
            assert!(
                self.store.transaction().is_ok(),
                "wait woke before profile transaction ended"
            );
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn write_fixture(
        registry: &mut TerminalRegistry<Backend>,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        who: &TerminalWaitIdentity,
    ) -> std::result::Result<
        crate::terminal_write_completion::TerminalWriteReceipt,
        TerminalRegistryError,
    > {
        use machine_god_core::{
            TerminalWriteLeaseIntent, TerminalWritePayload, TerminalWriteRequest,
        };
        registry
            .mutate_with_profile(
                store,
                budget,
                &who.owner,
                &who.session,
                |session, persistence| {
                    session.write_with(
                        persistence,
                        &who.owner,
                        who.actor,
                        who.writer,
                        &TerminalWriteRequest {
                            lease: TerminalWriteLeaseIntent::Acquire,
                            payload: None,
                        },
                        false,
                    )?;
                    session.write_completion_with(
                        persistence,
                        &who.owner,
                        who.actor,
                        who.writer,
                        &TerminalWriteRequest {
                            lease: TerminalWriteLeaseIntent::Use,
                            payload: Some(TerminalWritePayload::Text {
                                text: "abcdefgh".into(),
                            }),
                        },
                        false,
                    )
                },
            )
            .map(|(input, publication_error)| {
                crate::terminal_write_completion::TerminalWriteReceipt {
                    input,
                    publication_error: publication_error.map(TerminalRegistryError::Session),
                }
            })
    }

    #[test]
    fn owner_writes_complete_once_after_cancellation_or_shutdown() {
        use crate::terminal_input::TerminalInputProgress;
        use crate::terminal_write_completion::TerminalWriteIdentity;
        for close in [false, true] {
            let fixture = Fixture::new();
            fixture.state.lock().unwrap().write_limit = Some(2);
            let who = identity();
            let mut registry = fixture.registry(&who);
            let (worker, handle) = TerminalOwnerLoop::new();
            let caller = CancellationToken::new();
            let mut request = handle.request_with_writes(
                caller.clone(),
                move |registry, store, budget, writes, _, cancellation| {
                    let write_identity = TerminalWriteIdentity {
                        owner: who.owner.clone(),
                        session: who.session.clone(),
                        actor: who.actor,
                        writer: who.writer,
                    };
                    writes.submit(write_identity, cancellation, || {
                        write_fixture(registry, store, budget, &who)
                    })
                },
            );
            assert!(poll_request(&mut request).is_pending());
            let wake = Arc::new(ProfileWake {
                store: Arc::clone(&fixture.store),
                calls: AtomicUsize::new(0),
            });
            let waker = Waker::from(Arc::clone(&wake));
            let mut waiting = None;
            let mut receipt = None;
            let mut ticks = 0;
            let exit = worker.run_with_profile(
                &mut registry,
                &fixture.store,
                &fixture.budget,
                || 1,
                |_| {
                    ticks += 1;
                    assert!(ticks < 12);
                    if waiting.is_none()
                        && let Poll::Ready(result) = poll_request(&mut request)
                    {
                        waiting = Some(result.unwrap().unwrap());
                        caller.cancel();
                    }
                    if let Some(wait) = &mut waiting
                        && let Poll::Ready(result) =
                            Pin::new(wait).poll(&mut Context::from_waker(&waker))
                    {
                        receipt = Some(result.unwrap());
                        handle.shutdown();
                    } else if close && waiting.is_some() {
                        handle.shutdown();
                    }
                },
            );
            assert_eq!(exit.error, None);
            assert!(exit.shutdown.unwrap().is_empty());
            let receipt = receipt.unwrap_or_else(|| {
                let Poll::Ready(result) =
                    Pin::new(waiting.as_mut().unwrap()).poll(&mut Context::from_waker(&waker))
                else {
                    panic!("shutdown left pending input reply")
                };
                result.unwrap()
            });
            let state = fixture.state.lock().unwrap();
            assert_eq!(state.written, b"abcdefgh"[..receipt.input.accepted_bytes]);
            assert_eq!(
                receipt.input.progress,
                if close {
                    TerminalInputProgress::Closed
                } else {
                    TerminalInputProgress::Complete
                }
            );
            assert_eq!(receipt.input.accepted_bytes, if close { 4 } else { 8 });
            assert_eq!(state.closes, 1);
            assert_eq!(wake.calls.load(Ordering::Acquire), 1);
        }
    }

    #[test]
    fn write_receipt_authorizes_owner_actor_and_writer_after_shutdown() {
        use machine_god_core::{
            TerminalWriteLeaseIntent, TerminalWritePayload, TerminalWriteRequest,
        };
        let fixture = Fixture::new();
        let who = identity();
        let mut registry = fixture.registry(&who);
        let input = registry
            .mutate_with_profile(
                &fixture.store,
                &fixture.budget,
                &who.owner,
                &who.session,
                |session, persistence| {
                    session.write_with(
                        persistence,
                        &who.owner,
                        who.actor,
                        who.writer,
                        &TerminalWriteRequest {
                            lease: TerminalWriteLeaseIntent::Acquire,
                            payload: None,
                        },
                        false,
                    )?;
                    session.write_with(
                        persistence,
                        &who.owner,
                        who.actor,
                        who.writer,
                        &TerminalWriteRequest {
                            lease: TerminalWriteLeaseIntent::Use,
                            payload: Some(TerminalWritePayload::Text {
                                text: "once".into(),
                            }),
                        },
                        false,
                    )
                },
            )
            .unwrap();
        registry
            .shutdown_with_profile(
                &fixture.store,
                &fixture.budget,
                1,
                TerminalClosePolicy::Force,
            )
            .unwrap();
        let operation = input.operation_id.unwrap();
        assert_eq!(
            registry
                .write_receipt(&who.owner, &who.session, who.actor, who.writer, operation)
                .unwrap(),
            input
        );
        let foreign = BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new("other").unwrap(),
        );
        assert_eq!(
            registry.write_receipt(&foreign, &who.session, who.actor, who.writer, operation),
            Err(TerminalRegistryError::NotFound)
        );
        assert!(
            registry
                .write_receipt(
                    &who.owner,
                    &who.session,
                    TerminalActorRole::Human,
                    who.writer,
                    operation
                )
                .is_err()
        );
        assert!(
            registry
                .write_receipt(
                    &who.owner,
                    &who.session,
                    who.actor,
                    TerminalWriterId::new(NonZeroU64::new(2).unwrap()),
                    operation
                )
                .is_err()
        );
        assert_eq!(fixture.state.lock().unwrap().written, b"once");
    }

    #[test]
    fn profile_owner_wait_matches_committed_chunks_and_wakes_after_attention_commit() {
        let fixture = Fixture::new();
        let who = identity();
        let mut registry = fixture.registry(&who);
        let (worker, handle) = TerminalOwnerLoop::new();
        let state = Arc::clone(&fixture.state);
        let mut request = handle.request_with_waits(
            CancellationToken::new(),
            move |registry, store, budget, waits, now_ms, cancellation| {
                let wait = admit(
                    registry,
                    store,
                    budget,
                    waits,
                    who,
                    TerminalReturnCondition::Match {
                        pattern: "hello".into(),
                    },
                    now_ms,
                    cancellation.clone(),
                );
                state
                    .lock()
                    .unwrap()
                    .output
                    .extend([b"he".to_vec(), b"llo".to_vec()]);
                wait
            },
        );
        assert!(poll_request(&mut request).is_pending());
        let wake = Arc::new(ProfileWake {
            store: Arc::clone(&fixture.store),
            calls: AtomicUsize::new(0),
        });
        let waker = Waker::from(Arc::clone(&wake));
        let mut waiting = None;
        let mut receipt = None;
        let mut ticks = 0;
        let exit = worker.run_with_profile(
            &mut registry,
            &fixture.store,
            &fixture.budget,
            || 1,
            |steps| {
                for step in steps {
                    assert!(step.result.is_ok(), "pump failure: {:?}", step.result.err());
                }
                ticks += 1;
                assert!(ticks < 20);
                if waiting.is_none()
                    && let Poll::Ready(result) = poll_request(&mut request)
                {
                    waiting = Some(result.unwrap());
                }
                if let Some(wait) = &mut waiting
                    && let Poll::Ready(result) =
                        Pin::new(wait).poll(&mut Context::from_waker(&waker))
                {
                    receipt = Some(result);
                    assert_eq!(fixture.state.lock().unwrap().closes, 0);
                    handle.shutdown();
                }
            },
        );
        assert!(exit.error.is_none());
        assert!(exit.shutdown.unwrap().is_empty());
        assert_eq!(
            receipt.unwrap(),
            TerminalWaitReceipt {
                outcome: TerminalWaitOutcome::ConditionMet,
                attention_error: None
            }
        );
        assert_eq!(wake.calls.load(Ordering::Relaxed), 1);
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
    }

    #[test]
    fn dropped_attention_wait_does_not_stop_owner_or_other_wait() {
        let fixture = Fixture::new();
        let who = identity();
        let mut registry = fixture.registry(&who);
        let (worker, handle) = TerminalOwnerLoop::new();
        let state = Arc::clone(&fixture.state);
        let mut request = handle.request_with_waits(
            CancellationToken::new(),
            move |registry, store, budget, waits, now_ms, _| {
                let abandoned = admit(
                    registry,
                    store,
                    budget,
                    waits,
                    who.clone(),
                    TerminalReturnCondition::Exit,
                    now_ms,
                    CancellationToken::new(),
                );
                let survivor = admit(
                    registry,
                    store,
                    budget,
                    waits,
                    who,
                    TerminalReturnCondition::Match {
                        pattern: "alive".into(),
                    },
                    now_ms,
                    CancellationToken::new(),
                );
                drop(abandoned);
                state.lock().unwrap().output.push_back(b"alive".to_vec());
                survivor
            },
        );
        assert!(poll_request(&mut request).is_pending());
        let mut waiting = None;
        let mut receipt = None;
        let mut ticks = 0;
        let exit = worker.run_with_profile(
            &mut registry,
            &fixture.store,
            &fixture.budget,
            || 1,
            |_| {
                ticks += 1;
                assert!(ticks < 20);
                if waiting.is_none()
                    && let Poll::Ready(result) = poll_request(&mut request)
                {
                    waiting = Some(result.unwrap());
                }
                if let Some(wait) = &mut waiting
                    && let Poll::Ready(result) = poll_wait(wait)
                {
                    receipt = Some(result);
                    assert_eq!(fixture.state.lock().unwrap().closes, 0);
                    handle.shutdown();
                }
            },
        );
        assert!(exit.error.is_none());
        assert!(exit.shutdown.unwrap().is_empty());
        assert_eq!(receipt.unwrap().outcome, TerminalWaitOutcome::ConditionMet);
    }

    #[test]
    fn failed_attention_publication_retries_and_shutdown_reports_unavailable() {
        for fail_shutdown in [false, true] {
            let fixture = Fixture::new();
            let who = identity();
            let mut registry = fixture.registry(&who);
            let (worker, handle) = TerminalOwnerLoop::new();
            let mut request = handle.request_with_waits(
                CancellationToken::new(),
                move |registry, store, budget, waits, now_ms, _| {
                    admit(
                        registry,
                        store,
                        budget,
                        waits,
                        who,
                        TerminalReturnCondition::Quiet { duration_ms: 5 },
                        now_ms,
                        CancellationToken::new(),
                    )
                },
            );
            assert!(poll_request(&mut request).is_pending());
            let mut waiting = None;
            let mut receipt = None;
            let mut held_transaction = None;
            let mut ticks = 0;
            let mut now_ms = 0;
            let exit = worker.run_with_profile(
                &mut registry,
                &fixture.store,
                &fixture.budget,
                || {
                    now_ms += 1;
                    now_ms
                },
                |_| {
                    ticks += 1;
                    assert!(ticks < 20);
                    if waiting.is_none()
                        && let Poll::Ready(result) = poll_request(&mut request)
                    {
                        waiting = Some(result.unwrap());
                        held_transaction = Some(fixture.store.transaction().unwrap());
                    }
                    if let Some(wait) = &mut waiting {
                        if ticks <= 8 {
                            assert!(
                                poll_wait(wait).is_pending(),
                                "attention transition has not committed"
                            );
                        } else if let Poll::Ready(result) = poll_wait(wait) {
                            receipt = Some(result);
                            handle.shutdown();
                        }
                    }
                    if ticks == 8 {
                        if fail_shutdown {
                            handle.shutdown();
                        } else {
                            held_transaction = None;
                        }
                    }
                },
            );
            drop(held_transaction);
            if fail_shutdown {
                assert_eq!(exit.error, Some(TerminalOwnerError::WaitAttention));
                assert!(!exit.shutdown.unwrap().is_empty());
                let Poll::Ready(result) = poll_wait(waiting.as_mut().unwrap()) else {
                    panic!("shutdown left a wait pending")
                };
                assert_eq!(result.outcome, TerminalWaitOutcome::ConditionMet);
                assert_eq!(
                    result.attention_error,
                    Some(crate::terminal_wait::TerminalWaitAttentionError::Unavailable)
                );
            } else {
                assert!(exit.error.is_none());
                assert!(exit.shutdown.unwrap().is_empty());
                assert_eq!(
                    receipt.unwrap(),
                    TerminalWaitReceipt {
                        outcome: TerminalWaitOutcome::ConditionMet,
                        attention_error: None
                    }
                );
            }
        }
    }

    #[test]
    fn owner_waits_observe_inactive_and_recovered_records_without_native_pumps() {
        for recover in [false, true] {
            let fixture = Fixture::new();
            let who = identity();
            let mut registry = fixture.registry(&who);
            let (worker, handle) = TerminalOwnerLoop::new();
            let mut request = handle.request_with_waits(
                CancellationToken::new(),
                move |registry, store, budget, waits, now_ms, _| {
                    let namespace = owner_name("/workspace", &who.owner);
                    let mut transaction = store.transaction().unwrap();
                    let mut persistence =
                        TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                    registry
                        .live_mut(&who.owner, &who.session)
                        .unwrap()
                        .close_with(
                            &mut persistence,
                            &who.owner,
                            TerminalClosePolicy::Force,
                            now_ms,
                        )
                        .unwrap();
                    if recover {
                        registry.release(&who.owner, &who.session).unwrap();
                        let journal = crate::terminal_journal::TerminalJournal::open_existing(
                            transaction.open_session(&namespace, &who.session).unwrap(),
                            &who.session,
                            TerminalJournalLimits::default(),
                        )
                        .unwrap();
                        let history = TerminalHistory::recover(journal).unwrap();
                        let mut persistence = TerminalProfileMutationContext::new(
                            &mut transaction,
                            *budget,
                            &namespace,
                        );
                        let recovered =
                            crate::terminal_session::TerminalRecoveredSession::recover_with(
                                &mut persistence,
                                history,
                                &who.owner,
                                now_ms,
                            )
                            .unwrap();
                        registry
                            .recover(who.owner.clone(), who.session.clone(), || Ok(recovered))
                            .unwrap();
                    }
                    let (context, last_output, _) =
                        registry.wait_observation(&who.owner, &who.session).unwrap();
                    waits
                        .register(
                            who,
                            TerminalWaitRequest {
                                condition: TerminalReturnCondition::Exit,
                                safety_ceiling_ms: 100,
                            },
                            &context,
                            last_output,
                            TerminalWaitHistory::default(),
                            CancellationToken::new(),
                        )
                        .unwrap()
                        .1
                },
            );
            assert!(poll_request(&mut request).is_pending());
            let mut waiting = None;
            let mut receipt = None;
            let mut ticks = 0;
            let exit = worker.run_with_profile(
                &mut registry,
                &fixture.store,
                &fixture.budget,
                || 1,
                |_| {
                    ticks += 1;
                    assert!(ticks < 10);
                    if waiting.is_none()
                        && let Poll::Ready(result) = poll_request(&mut request)
                    {
                        waiting = Some(result.unwrap());
                    }
                    if let Some(wait) = &mut waiting
                        && let Poll::Ready(result) = poll_wait(wait)
                    {
                        receipt = Some(result);
                        handle.shutdown();
                    }
                },
            );
            assert!(exit.error.is_none());
            assert!(exit.shutdown.unwrap().is_empty());
            let receipt = receipt.unwrap();
            assert_eq!(receipt.outcome, TerminalWaitOutcome::Exited(7));
            assert!(receipt.attention_error.is_none());
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
        }
    }

    #[test]
    fn owner_catches_up_committed_pages_without_losing_segment_rollovers() {
        for rollover in [false, true] {
            let fixture = Fixture::new();
            let who = identity();
            let limits = if rollover {
                TerminalJournalLimits {
                    segment_bytes: 8 * MAX_MONITOR_FEED_BYTES,
                    session_bytes: 16 * 1024 * 1024,
                }
            } else {
                TerminalJournalLimits::default()
            };
            let mut registry = fixture.registry_with_limits(&who, limits);
            let (worker, handle) = TerminalOwnerLoop::new();
            let state = Arc::clone(&fixture.state);
            let mut request = handle.request_with_waits(
                CancellationToken::new(),
                move |registry, store, budget, waits, now_ms, _| {
                    let wait = admit(
                        registry,
                        store,
                        budget,
                        waits,
                        who,
                        TerminalReturnCondition::Match {
                            pattern: "hello".into(),
                        },
                        now_ms,
                        CancellationToken::new(),
                    );
                    let chunks = if rollover { 8 } else { 1 };
                    for _ in 1..chunks {
                        state
                            .lock()
                            .unwrap()
                            .output
                            .push_back(vec![b'x'; MAX_MONITOR_FEED_BYTES]);
                    }
                    let mut first = vec![b'x'; MAX_MONITOR_FEED_BYTES];
                    first[MAX_MONITOR_FEED_BYTES - 2..].copy_from_slice(b"he");
                    state
                        .lock()
                        .unwrap()
                        .output
                        .extend([first, b"llo".to_vec()]);
                    for _ in 0..=chunks {
                        let steps = registry
                            .pump_with_profile(store, budget, now_ms, MAX_RESIDENT_TERMINALS)
                            .unwrap();
                        assert!(steps.iter().all(|step| step.result.is_ok()));
                    }
                    wait
                },
            );
            assert!(poll_request(&mut request).is_pending());
            let mut waiting = None;
            let mut receipt = None;
            let mut ticks = 0;
            let exit = worker.run_with_profile(
                &mut registry,
                &fixture.store,
                &fixture.budget,
                || 1,
                |_| {
                    ticks += 1;
                    assert!(ticks < 20);
                    if waiting.is_none()
                        && let Poll::Ready(result) = poll_request(&mut request)
                    {
                        waiting = Some(result.unwrap());
                    }
                    if let Some(wait) = &mut waiting
                        && let Poll::Ready(result) = poll_wait(wait)
                    {
                        receipt = Some(result);
                        handle.shutdown();
                    }
                },
            );
            assert!(
                ticks >= 3,
                "two pages must not be consumed in one unbounded turn"
            );
            assert!(exit.error.is_none());
            assert!(exit.shutdown.unwrap().is_empty());
            assert_eq!(
                receipt.unwrap(),
                TerminalWaitReceipt {
                    outcome: TerminalWaitOutcome::ConditionMet,
                    attention_error: None
                }
            );
        }
    }
}
