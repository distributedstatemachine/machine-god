//! Continuous terminal pumping on an explicitly owned blocking worker.
//! Construction and unpolled requests are inert; this module spawns no threads.

use crate::terminal_registry::{
    MAX_RESIDENT_TERMINALS, TerminalRegistry, TerminalRegistryError, TerminalRegistryFailure,
    TerminalRegistryStep,
};
use crate::terminal_session::TerminalSessionBackend;
use machine_god_core::{CancellationToken, Cancelled, TerminalClosePolicy};
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

trait Job<B: TerminalSessionBackend>: Send {
    /// False means an operation panicked and this owner must stop.
    fn execute(self: Box<Self>, registry: &mut TerminalRegistry<B>, now_ms: i64) -> bool;
}
enum Message<B: TerminalSessionBackend> {
    Job(Box<dyn Job<B>>),
    Wake,
}
struct Shared<B: TerminalSessionBackend> {
    sender: SyncSender<Message<B>>,
    closing: AtomicBool,
    clients: AtomicUsize,
    requests: Arc<AtomicUsize>,
    callback_panicked: Arc<AtomicBool>,
}
impl<B: TerminalSessionBackend> Shared<B> {
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
    waker: Option<Waker>,
}
fn complete<T>(reply: &Mutex<Reply<T>>, value: Result<T>) -> bool {
    let wake = {
        let mut reply = reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reply.result = Some(value);
        reply.waker.take()
    };
    // Never invoke user-controlled wakers while holding a synchronization lock.
    if let Some(waker) = wake {
        return catch_callback(|| waker.wake()).is_ok();
    }
    true
}
type Operation<B, T> =
    Box<dyn FnOnce(&mut TerminalRegistry<B>, i64, &CancellationToken) -> T + Send>;
struct Request<B: TerminalSessionBackend, T> {
    operation: Option<Operation<B, T>>,
    reply: Arc<Mutex<Reply<T>>>,
    caller: CancellationToken,
    cancellation: CancellationToken,
    _permit: Arc<Permit>,
    callback_panicked: Arc<AtomicBool>,
    completed: bool,
}
impl<B: TerminalSessionBackend, T: Send> Job<B> for Request<B, T> {
    fn execute(mut self: Box<Self>, registry: &mut TerminalRegistry<B>, now_ms: i64) -> bool {
        if self.caller.is_cancelled() {
            self.cancellation.cancel();
        }
        let result = if self.cancellation.is_cancelled() {
            Err(TerminalOwnerError::Cancelled)
        } else {
            let operation = self.operation.take().expect("request executed once");
            catch_callback(|| operation(registry, now_ms, &self.cancellation))
                .map_err(|_| TerminalOwnerError::Panicked)
        };
        let keep_running = !matches!(&result, Err(TerminalOwnerError::Panicked));
        self.completed = true;
        let woke = complete(&self.reply, result);
        if !woke {
            self.callback_panicked.store(true, Ordering::Release);
        }
        keep_running && woke
    }
}
impl<B: TerminalSessionBackend, T> Drop for Request<B, T> {
    fn drop(&mut self) {
        if !self.completed && !complete(&self.reply, Err(TerminalOwnerError::Closed)) {
            // Contain wake separately from captured-value Drop: if both panic,
            // unwinding one through the other would abort before any outer catch.
            self.callback_panicked.store(true, Ordering::Release);
        }
    }
}

/// The host owns this handle, not an individual tool future. Last-handle drop
/// requests shutdown; it never waits for process cleanup on the polling thread.
pub(crate) struct TerminalOwnerHandle<B: TerminalSessionBackend> {
    shared: Arc<Shared<B>>,
}
impl<B: TerminalSessionBackend> Clone for TerminalOwnerHandle<B> {
    fn clone(&self) -> Self {
        self.shared.clients.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}
impl<B: TerminalSessionBackend> Drop for TerminalOwnerHandle<B> {
    fn drop(&mut self) {
        if self.shared.clients.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.close();
        }
    }
}
impl<B: TerminalSessionBackend + 'static> TerminalOwnerHandle<B> {
    /// Operations must be bounded, authorized host commands. No tool-supplied
    /// closure, process backend, or unbounded external probe belongs here.
    pub(crate) fn request<T: Send + 'static>(
        &self,
        caller: CancellationToken,
        operation: impl FnOnce(&mut TerminalRegistry<B>, i64, &CancellationToken) -> T + Send + 'static,
    ) -> TerminalOwnerFuture<B, T> {
        TerminalOwnerFuture {
            shared: Arc::clone(&self.shared),
            operation: Some(Box::new(operation)),
            reply: Arc::new(Mutex::new(Reply {
                result: None,
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
pub(crate) struct TerminalOwnerFuture<B: TerminalSessionBackend, T> {
    shared: Arc<Shared<B>>,
    operation: Option<Operation<B, T>>,
    reply: Arc<Mutex<Reply<T>>>,
    caller_wait: Option<Cancelled>,
    caller: CancellationToken,
    cancellation: CancellationToken,
    permit: Option<Arc<Permit>>,
    submitted: bool,
    finished: bool,
}
impl<B: TerminalSessionBackend + 'static, T: Send + 'static> Future for TerminalOwnerFuture<B, T> {
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
impl<B: TerminalSessionBackend, T> Drop for TerminalOwnerFuture<B, T> {
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

pub(crate) struct TerminalOwnerLoop<B: TerminalSessionBackend> {
    shared: Arc<Shared<B>>,
    receiver: Receiver<Message<B>>,
}
pub(crate) struct TerminalOwnerExit {
    pub(crate) error: Option<TerminalOwnerError>,
    pub(crate) shutdown: std::result::Result<Vec<TerminalRegistryFailure>, TerminalRegistryError>,
}
impl<B: TerminalSessionBackend> TerminalOwnerLoop<B> {
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
    pub(crate) fn new() -> (Self, TerminalOwnerHandle<B>) {
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
            TerminalOwnerHandle { shared },
        )
    }
    /// Runs only on the host's owned blocking worker. It borrows the registry so
    /// the same worker can retain failed histories after inspecting the exit.
    /// The observer consumes bounded output/probe descriptions synchronously;
    /// it must not execute unbounded probes or enqueue unbounded output.
    pub(crate) fn run(
        self,
        registry: &mut TerminalRegistry<B>,
        mut clock: impl FnMut() -> i64,
        mut observer: impl FnMut(Vec<TerminalRegistryStep>),
    ) -> TerminalOwnerExit {
        let mut error = None;
        let mut deadline = Instant::now();
        let execution = catch_callback(|| {
            while !self.shared.closing.load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    let output_ready = match registry.pump(clock(), MAX_RESIDENT_TERMINALS) {
                        Ok(steps) => {
                            let output_ready = steps.iter().any(|step| {
                                step.result
                                    .as_ref()
                                    .is_ok_and(|step| !step.output.is_empty())
                            });
                            observer(steps);
                            output_ready
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
                            PUMP_INTERVAL
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
                        if !job.execute(registry, registry.minimum_time_ms()) {
                            error = Some(TerminalOwnerError::Panicked);
                            break;
                        }
                    }
                    Ok(Message::Wake) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        if execution.is_err() {
            error = Some(TerminalOwnerError::Panicked);
        }
        self.shared.close();
        // Resolve queued requests before native shutdown; no rejected operation runs.
        let rejection_panicked = self.reject_pending();
        if rejection_panicked || self.shared.callback_panicked.load(Ordering::Acquire) {
            error = Some(TerminalOwnerError::Panicked);
        }
        let shutdown = registry.shutdown(registry.minimum_time_ms(), TerminalClosePolicy::Force);
        TerminalOwnerExit { error, shutdown }
    }
}
impl<B: TerminalSessionBackend> Drop for TerminalOwnerLoop<B> {
    fn drop(&mut self) {
        self.shared.close();
        self.reject_pending();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            waker: None,
        }));
        let wake = Arc::new(ReentrantWake {
            reply: Arc::downgrade(&reply),
            woke: AtomicBool::new(false),
        });
        reply.lock().unwrap().waker = Some(Waker::from(Arc::clone(&wake)));
        complete(&reply, Ok(()));
        assert!(wake.woke.load(Ordering::Acquire));
    }
}
