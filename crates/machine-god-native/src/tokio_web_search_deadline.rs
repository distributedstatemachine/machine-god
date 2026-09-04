//! Tokio-backed deadline authority for native web search.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::task::{Context, Poll};
use std::time::Instant;

use machine_god_core::BoxFuture;
use tokio::runtime::{Handle, Id, Runtime};
use tokio::time::Sleep;

use crate::{WebSearchDeadline, WebSearchTransportError, WebSearchTransportErrorKind};

/// Deadline authority paired with one time-and-I/O-enabled Tokio runtime.
///
/// The private binding prevents a host from constructing this adapter for a
/// driverless runtime. A wait owns its timer registration directly on the
/// proven paired runtime, so dropping the wait synchronously deregisters that
/// timer without leaving a spawned task tail.
pub struct TokioWebSearchDeadline {
    runtime_id: Id,
    lifecycle: Weak<TokioWebSearchRuntimeLifecycle>,
}

/// Tokio runtime whose destruction is paired with a deadline authority.
///
/// Each deadline poll takes a bounded admission before touching its timer.
/// Dropping this wrapper atomically closes admission, transfers the raw runtime
/// to shared cleanup ownership, and uses context-safe nonblocking shutdown as
/// soon as every already-admitted poll has returned.
pub struct TokioWebSearchRuntime {
    runtime: Option<Runtime>,
    handle: Handle,
    lifecycle: Arc<TokioWebSearchRuntimeLifecycle>,
}

struct TokioWebSearchRuntimeLifecycle {
    state: AtomicUsize,
    runtime: Mutex<Option<Runtime>>,
}

struct TokioWebSearchRuntimeAdmission {
    lifecycle: Arc<TokioWebSearchRuntimeLifecycle>,
}

const RUNTIME_CLOSED: usize = 1 << (usize::BITS - 1);
const RUNTIME_ACTIVE_MASK: usize = RUNTIME_CLOSED - 1;

impl TokioWebSearchDeadline {
    /// Builds a current-thread runtime and its non-forgeable deadline adapter.
    ///
    /// The runtime has both I/O and time enabled. The host must drive the
    /// returned runtime and keep it paired with the returned adapter. Runtime
    /// construction failure is reported through the fixed, data-free
    /// [`WebSearchTransportErrorKind::RuntimeRequired`] category.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeRequired` when Tokio cannot build the paired runtime.
    pub fn build_runtime_pair() -> Result<(TokioWebSearchRuntime, Self), WebSearchTransportError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| runtime_required())?;
        let handle = runtime.handle().clone();
        let runtime_id = handle.id();
        let lifecycle = Arc::new(TokioWebSearchRuntimeLifecycle {
            state: AtomicUsize::new(0),
            runtime: Mutex::new(None),
        });
        let deadline = Self {
            runtime_id,
            lifecycle: Arc::downgrade(&lifecycle),
        };
        let runtime = TokioWebSearchRuntime {
            runtime: Some(runtime),
            handle,
            lifecycle,
        };
        Ok((runtime, deadline))
    }
}

impl TokioWebSearchRuntime {
    /// Runs a future to completion on the paired current-thread runtime.
    ///
    /// # Panics
    ///
    /// Propagates the same nested-runtime and future panics as
    /// [`Runtime::block_on`].
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime
            .as_ref()
            .expect("paired runtime remains present until wrapper drop")
            .block_on(future)
    }

    /// Returns the paired Tokio runtime handle.
    #[must_use]
    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}

impl Drop for TokioWebSearchRuntime {
    fn drop(&mut self) {
        self.lifecycle.close();
        self.lifecycle.install_runtime(self.runtime.take());
        // If the last admission left between `close` and `install_runtime`, it
        // observed no transferred runtime to clean up. This mandatory retry
        // sees the now-idle closed state and performs the one-time shutdown.
        self.lifecycle.shutdown_if_closed_and_idle();
    }
}

impl TokioWebSearchRuntimeLifecycle {
    fn try_admit(self: &Arc<Self>) -> Option<TokioWebSearchRuntimeAdmission> {
        let mut observed = self.state.load(Ordering::Acquire);
        loop {
            if observed & RUNTIME_CLOSED != 0
                || observed & RUNTIME_ACTIVE_MASK == RUNTIME_ACTIVE_MASK
            {
                return None;
            }
            match self.state.compare_exchange_weak(
                observed,
                observed + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(TokioWebSearchRuntimeAdmission {
                        lifecycle: Arc::clone(self),
                    });
                }
                Err(current) => observed = current,
            }
        }
    }

    fn close(&self) {
        self.state.fetch_or(RUNTIME_CLOSED, Ordering::AcqRel);
    }

    fn is_closed(&self) -> bool {
        self.state.load(Ordering::Acquire) & RUNTIME_CLOSED != 0
    }

    fn install_runtime(&self, runtime: Option<Runtime>) {
        let Some(runtime) = runtime else {
            return;
        };
        let mut owned = self.runtime.lock().unwrap_or_else(PoisonError::into_inner);
        if owned.is_none() {
            *owned = Some(runtime);
            return;
        }
        drop(owned);
        runtime.shutdown_background();
    }

    fn shutdown_if_closed_and_idle(&self) {
        if self.state.load(Ordering::Acquire) != RUNTIME_CLOSED {
            return;
        }
        let runtime = {
            self.runtime
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
        };
        if let Some(runtime) = runtime {
            runtime.shutdown_background();
        }
    }
}

impl Drop for TokioWebSearchRuntimeAdmission {
    fn drop(&mut self) {
        let previous = self.lifecycle.state.fetch_sub(1, Ordering::AcqRel);
        debug_assert_ne!(previous & RUNTIME_ACTIVE_MASK, 0);
        if previous == RUNTIME_CLOSED | 1 {
            self.lifecycle.shutdown_if_closed_and_idle();
        }
    }
}

impl Drop for TokioWebSearchRuntimeLifecycle {
    fn drop(&mut self) {
        let runtime = self
            .runtime
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(runtime) = runtime {
            runtime.shutdown_background();
        }
    }
}

impl fmt::Debug for TokioWebSearchRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokioWebSearchRuntime")
    }
}

impl fmt::Debug for TokioWebSearchDeadline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokioWebSearchDeadline")
    }
}

impl WebSearchDeadline for TokioWebSearchDeadline {
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(TokioWebSearchWait {
            authority: self,
            deadline,
            sleep: None,
            complete: false,
        })
    }
}

struct TokioWebSearchWait<'a> {
    authority: &'a TokioWebSearchDeadline,
    deadline: Instant,
    sleep: Option<Pin<Box<Sleep>>>,
    complete: bool,
}

impl Future for TokioWebSearchWait<'_> {
    type Output = Result<(), WebSearchTransportError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.complete {
            return Poll::Ready(Ok(()));
        }
        let Some(lifecycle) = self.authority.lifecycle.upgrade() else {
            // The lifecycle can disappear only after the paired runtime has
            // completed its nonblocking shutdown. Tokio timer deregistration
            // is synchronous and remains valid after driver shutdown.
            self.sleep.take();
            return Poll::Ready(Err(runtime_required()));
        };
        let Some(_admission) = lifecycle.try_admit() else {
            // A closed lifecycle will never poll or construct another timer.
            // Any timer retained from an earlier poll may be deregistered
            // after shutdown; it cannot keep the runtime alive by itself.
            self.sleep.take();
            return Poll::Ready(Err(runtime_required()));
        };
        let Ok(current) = Handle::try_current() else {
            // Keep the admission live while dropping the paired timer: its
            // registered Waker may reentrantly close the runtime wrapper.
            self.sleep.take();
            return Poll::Ready(Err(runtime_required()));
        };
        if current.id() != self.authority.runtime_id {
            self.sleep.take();
            return Poll::Ready(Err(runtime_required()));
        }

        if self.sleep.is_none() {
            let deadline = self.deadline;
            let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
            self.sleep = Some(Box::pin(sleep));
        }

        let sleep_poll = match self.sleep.as_mut() {
            Some(sleep) => sleep.as_mut().poll(context),
            None => return Poll::Ready(Err(runtime_required())),
        };
        let observed_at = Instant::now();
        if lifecycle.is_closed() {
            self.sleep.take();
            return Poll::Ready(Err(runtime_required()));
        }
        match sleep_poll {
            Poll::Pending => Poll::Pending,
            Poll::Ready(()) => {
                self.sleep.take();
                if observed_at < self.deadline {
                    return Poll::Ready(Err(runtime_required()));
                }
                self.complete = true;
                Poll::Ready(Ok(()))
            }
        }
    }
}

const fn runtime_required() -> WebSearchTransportError {
    WebSearchTransportError::new(WebSearchTransportErrorKind::RuntimeRequired)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::poll_fn;
    use std::panic::{PanicHookInfo, take_hook};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex, MutexGuard, PoisonError};
    use std::task::{Poll, Wake, Waker};
    use std::thread;
    use std::time::{Duration, Instant};

    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};

    use super::*;

    type PanicHook = dyn for<'a> Fn(&PanicHookInfo<'a>) + Send + Sync + 'static;

    static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());
    static MARKED_PANICS: AtomicUsize = AtomicUsize::new(0);

    thread_local! {
        static COUNT_PANICS: Cell<bool> = const { Cell::new(false) };
    }

    struct PanicHookGuard<'a> {
        _lock: MutexGuard<'a, ()>,
        previous: Arc<PanicHook>,
    }

    impl PanicHookGuard<'_> {
        fn install() -> Self {
            let lock = PANIC_HOOK_LOCK
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            MARKED_PANICS.store(0, Ordering::SeqCst);
            let previous: Arc<PanicHook> = Arc::from(take_hook());
            let delegated = Arc::clone(&previous);
            std::panic::set_hook(Box::new(move |information| {
                if COUNT_PANICS.get() {
                    MARKED_PANICS.fetch_add(1, Ordering::SeqCst);
                } else {
                    delegated(information);
                }
            }));
            Self {
                _lock: lock,
                previous,
            }
        }

        fn count() -> usize {
            MARKED_PANICS.load(Ordering::SeqCst)
        }
    }

    impl Drop for PanicHookGuard<'_> {
        fn drop(&mut self) {
            drop(take_hook());
            let previous = Arc::clone(&self.previous);
            std::panic::set_hook(Box::new(move |information| previous(information)));
        }
    }

    struct MarkPanics;

    impl MarkPanics {
        fn on_this_thread() -> Self {
            COUNT_PANICS.set(true);
            Self
        }
    }

    impl Drop for MarkPanics {
        fn drop(&mut self) {
            COUNT_PANICS.set(false);
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    struct RuntimeDroppingWake {
        runtime: Mutex<Option<TokioWebSearchRuntime>>,
        drops: Arc<AtomicUsize>,
    }

    impl Wake for RuntimeDroppingWake {
        fn wake(self: Arc<Self>) {}
    }

    impl Drop for RuntimeDroppingWake {
        fn drop(&mut self) {
            let runtime = self
                .runtime
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            drop(runtime);
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn runtime_pair_is_current_thread_with_time_and_io_enabled() {
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        runtime.block_on(async {
            assert_eq!(
                Handle::current().runtime_flavor(),
                tokio::runtime::RuntimeFlavor::CurrentThread
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            drop(listener);

            let target = Instant::now() + Duration::from_millis(1);
            deadline.wait_until(target).await.unwrap();
            assert!(Instant::now() >= target);
        });
    }

    #[test]
    fn wait_construction_is_inert_outside_the_paired_runtime() {
        let (_runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        drop(deadline.wait_until(Instant::now()));
        assert_eq!(format!("{deadline:?}"), "TokioWebSearchDeadline");
    }

    #[test]
    fn polling_without_a_runtime_returns_fixed_runtime_required() {
        let (_runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let error = futures_executor::block_on(deadline.wait_until(Instant::now())).unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
        assert_eq!(
            format!("{error:?} {error}"),
            "WebSearchTransportError { kind: RuntimeRequired } web-search transport failed"
        );
    }

    #[test]
    fn polling_on_driverless_runtime_returns_fixed_error_without_panicking() {
        let (_runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let driverless = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        let error = driverless
            .block_on(deadline.wait_until(Instant::now() + Duration::from_millis(1)))
            .unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
    }

    #[test]
    fn polling_on_foreign_time_enabled_runtime_returns_fixed_error_without_panicking() {
        let (_runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let foreign = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();

        let error = foreign
            .block_on(deadline.wait_until(Instant::now() + Duration::from_millis(1)))
            .unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
    }

    #[test]
    fn polling_through_a_shutdown_paired_handle_returns_fixed_error_without_panicking() {
        let _panic_hook = PanicHookGuard::install();
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let paired_handle = runtime.handle().clone();
        let _mark_panics = MarkPanics::on_this_thread();
        drop(runtime);

        let _entered = paired_handle.enter();
        let error = futures_executor::block_on(deadline.wait_until(Instant::now())).unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
        assert_eq!(PanicHookGuard::count(), 0);
    }

    #[test]
    fn concurrent_runtime_shutdown_and_first_poll_return_fixed_error_without_panicking() {
        let _panic_hook = PanicHookGuard::install();
        for _ in 0..32 {
            let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
            let paired_handle = runtime.handle().clone();
            let barrier = Arc::new(Barrier::new(2));

            let error = thread::scope(|scope| {
                let waiter_barrier = Arc::clone(&barrier);
                let waiter = scope.spawn(move || {
                    let _mark_panics = MarkPanics::on_this_thread();
                    let _entered = paired_handle.enter();
                    waiter_barrier.wait();
                    futures_executor::block_on(
                        deadline.wait_until(Instant::now() + Duration::from_secs(60)),
                    )
                    .unwrap_err()
                });
                let shutdown = scope.spawn(move || {
                    let _mark_panics = MarkPanics::on_this_thread();
                    barrier.wait();
                    drop(runtime);
                });

                let error = waiter.join().unwrap();
                shutdown.join().unwrap();
                error
            });
            assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
        }
        assert_eq!(PanicHookGuard::count(), 0);
    }

    #[test]
    fn wrapper_drop_inside_foreign_async_context_uses_nonblocking_shutdown_without_panicking() {
        let _panic_hook = PanicHookGuard::install();
        let (runtime, _deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let foreign = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();

        foreign.block_on(async move {
            let _mark_panics = MarkPanics::on_this_thread();
            drop(runtime);
        });

        assert_eq!(PanicHookGuard::count(), 0);
    }

    #[test]
    fn reentrant_wrapper_drop_closes_an_admitted_poll_before_it_returns_pending() {
        let _panic_hook = PanicHookGuard::install();
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let paired_handle = runtime.handle().clone();
        let drops = Arc::new(AtomicUsize::new(0));
        let runtime_waker = Waker::from(Arc::new(RuntimeDroppingWake {
            runtime: Mutex::new(Some(runtime)),
            drops: Arc::clone(&drops),
        }));
        let mut wait = deadline.wait_until(Instant::now() + Duration::from_secs(60));
        let _mark_panics = MarkPanics::on_this_thread();

        {
            let _entered = paired_handle.enter();
            let mut context = Context::from_waker(&runtime_waker);
            assert!(wait.as_mut().poll(&mut context).is_pending());
        }
        drop(runtime_waker);

        let error = {
            let _entered = paired_handle.enter();
            let waker = Waker::from(Arc::new(NoopWake));
            let mut context = Context::from_waker(&waker);
            match wait.as_mut().poll(&mut context) {
                Poll::Ready(Err(error)) => error,
                Poll::Ready(Ok(())) => panic!("closed runtime published timer success"),
                Poll::Pending => panic!("closed runtime stranded an admitted wait"),
            }
        };
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
        assert_eq!(drops.load(Ordering::SeqCst), 1);

        let _entered = paired_handle.enter();
        let error = futures_executor::block_on(deadline.wait_until(Instant::now())).unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
        assert_eq!(PanicHookGuard::count(), 0);
    }

    #[test]
    fn pending_wait_rejects_later_foreign_and_driverless_polls() {
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();

        let mut foreign_wait = deadline.wait_until(Instant::now() + Duration::from_secs(60));
        runtime.block_on(poll_fn(|context| {
            assert!(foreign_wait.as_mut().poll(context).is_pending());
            Poll::Ready(())
        }));
        let foreign = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let error = foreign.block_on(foreign_wait).unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);

        let mut driverless_wait = deadline.wait_until(Instant::now() + Duration::from_secs(60));
        runtime.block_on(poll_fn(|context| {
            assert!(driverless_wait.as_mut().poll(context).is_pending());
            Poll::Ready(())
        }));
        let driverless = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let error = driverless.block_on(driverless_wait).unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
    }

    #[test]
    fn virtual_timer_ready_before_standard_deadline_is_rejected() {
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        runtime.block_on(async {
            tokio::time::pause();
            let requested = Instant::now() + Duration::from_secs(60);
            let mut wait = deadline.wait_until(requested);
            poll_fn(|context| {
                assert!(wait.as_mut().poll(context).is_pending());
                Poll::Ready(())
            })
            .await;
            tokio::time::advance(Duration::from_secs(60)).await;
            let error = wait.await.unwrap_err();
            assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
            assert!(Instant::now() < requested);
        });
    }

    #[test]
    fn early_ready_is_sampled_before_blocking_waker_teardown() {
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        runtime.block_on(async {
            tokio::time::pause();
            let requested = Instant::now() + Duration::from_millis(100);
            let (waker, waker_handle) = reentrant_waker(Callback::Drop, move || {
                thread::sleep(requested.saturating_duration_since(Instant::now()));
            });
            let mut wait = deadline.wait_until(requested);
            let mut context = Context::from_waker(&waker);
            assert!(wait.as_mut().poll(&mut context).is_pending());
            tokio::time::advance(Duration::from_secs(60)).await;
            let error = match wait.as_mut().poll(&mut context) {
                Poll::Ready(Err(error)) => error,
                Poll::Ready(Ok(())) => panic!("blocking teardown hid early timer readiness"),
                Poll::Pending => panic!("advanced timer remained pending"),
            };
            assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
            assert!(Instant::now() >= requested);
            assert!(waker_handle.calls() >= 1);
        });
    }

    #[test]
    fn rapid_poll_and_drop_leaves_no_runtime_task_tail() {
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        runtime.block_on(async {
            let alive = Handle::current().metrics().num_alive_tasks();
            let wake = Arc::new(NoopWake);
            let waker = Waker::from(Arc::clone(&wake));
            let baseline_waker_owners = Arc::strong_count(&wake);
            for _ in 0..1_024 {
                let mut wait = deadline.wait_until(Instant::now() + Duration::from_secs(60));
                let mut context = Context::from_waker(&waker);
                assert!(wait.as_mut().poll(&mut context).is_pending());
                assert_eq!(Handle::current().metrics().num_alive_tasks(), alive);
                assert_eq!(Arc::strong_count(&wake), baseline_waker_owners + 1);
                drop(wait);
                assert_eq!(Handle::current().metrics().num_alive_tasks(), alive);
                assert_eq!(Arc::strong_count(&wake), baseline_waker_owners);
            }
        });
    }
}
