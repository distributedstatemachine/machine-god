//! Inert binding to the shared, bounded native worker collector.

use crate::background_supervisor::worker_ownership_registry;
use machine_god_core::BoxFuture;
use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// Fixed, redacted failure to admit, start or complete an owned native worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeOwnedWorkerSpawnError;

impl fmt::Display for NativeOwnedWorkerSpawnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("owned native worker unavailable")
    }
}
impl std::error::Error for NativeOwnedWorkerSpawnError {}

/// Starts workers whose handles belong to the production collector before work
/// is released. This zero-state binding does not create a collector or thread
/// until `spawn` is called, and dropping it does not detach any running worker.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeOwnedWorkerSpawner;

impl NativeOwnedWorkerSpawner {
    /// Constructs the binding without threads, native effects or reservations.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Admits one worker against the shared process-wide capacity. The job must
    /// own its shutdown/cleanup protocol; an abandoned response cannot transfer
    /// that responsibility to its caller or the collector.
    ///
    /// # Errors
    /// Returns a fixed failure if collection, capacity or thread admission
    /// fails. In that case the job has not executed; any failed-registration
    /// thread is collected before returning.
    pub fn spawn(
        &self,
        operation: impl FnOnce() + Send + 'static,
    ) -> Result<(), NativeOwnedWorkerSpawnError> {
        let registry = worker_ownership_registry().map_err(|()| NativeOwnedWorkerSpawnError)?;
        let reservation = registry
            .reserve_partitioned(&[1])
            .map_err(|()| NativeOwnedWorkerSpawnError)?
            .pop()
            .ok_or(NativeOwnedWorkerSpawnError)?;
        reservation
            .spawn_one(registry, "machine-god-owned-worker", operation)
            .map_err(|()| NativeOwnedWorkerSpawnError)
    }

    /// Returns an inert receipt for a single owned worker operation. Admission
    /// happens only on first poll. Once admitted, dropping the receipt does not
    /// cancel the operation or detach its worker from the production collector.
    /// The operation must own any cleanup required by its native effects.
    ///
    /// Operation panics become the fixed error. Panicking waker callbacks are
    /// contained outside the response lock; a broken waker cannot guarantee
    /// executor notification, but a later poll can still observe completion.
    pub fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> T + Send + 'static,
    ) -> BoxFuture<'static, Result<T, NativeOwnedWorkerSpawnError>> {
        Box::pin(OwnedWorkerFuture::new(operation, |job| {
            Self::new().spawn(job)
        }))
    }
}

type OwnedJob = Box<dyn FnOnce() + Send>;
type Admission = Box<dyn FnOnce(OwnedJob) -> Result<(), NativeOwnedWorkerSpawnError> + Send>;

struct Response<T> {
    value: Option<Result<T, NativeOwnedWorkerSpawnError>>,
    waker: Option<Waker>,
    abandoned: bool,
}

struct OwnedWorkerFuture<T> {
    operation: Option<Box<dyn FnOnce() -> T + Send>>,
    admission: Option<Admission>,
    response: Arc<Mutex<Response<T>>>,
}

/// Panic payloads are opaque user values: even their destructors may panic.
fn contain<T>(operation: impl FnOnce() -> T) -> Result<T, NativeOwnedWorkerSpawnError> {
    catch_unwind(AssertUnwindSafe(operation)).map_err(|payload| {
        std::mem::forget(payload);
        NativeOwnedWorkerSpawnError
    })
}

impl<T> OwnedWorkerFuture<T> {
    fn new(
        operation: impl FnOnce() -> T + Send + 'static,
        admission: impl FnOnce(OwnedJob) -> Result<(), NativeOwnedWorkerSpawnError> + Send + 'static,
    ) -> Self {
        Self {
            operation: Some(Box::new(operation)),
            admission: Some(Box::new(admission)),
            response: Arc::new(Mutex::new(Response {
                value: None,
                waker: None,
                abandoned: false,
            })),
        }
    }

    fn abandon(&mut self) {
        let (value, waker) = {
            let mut response = self
                .response
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            response.abandoned = true;
            (response.value.take(), response.waker.take())
        };
        let _ = contain(|| drop(waker));
        let _ = contain(|| drop(value));
        let _ = contain(|| drop(self.operation.take()));
        let _ = contain(|| drop(self.admission.take()));
    }
}

impl<T: Send + 'static> Future for OwnedWorkerFuture<T> {
    type Output = Result<T, NativeOwnedWorkerSpawnError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        // Raw wakers may reenter or panic on clone, drop and wake. Never invoke
        // any of those callbacks while the response mutex is held.
        let Ok(waker) = contain(|| context.waker().clone()) else {
            this.abandon();
            return Poll::Ready(Err(NativeOwnedWorkerSpawnError));
        };
        let (value, old_waker, unused_waker, abandoned) = {
            let mut response = this
                .response
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let value = response.value.take();
            if value.is_some() || response.abandoned {
                (
                    value,
                    response.waker.take(),
                    Some(waker),
                    response.abandoned,
                )
            } else {
                (None, response.waker.replace(waker), None, false)
            }
        };
        let _ = contain(|| drop(old_waker));
        let _ = contain(|| drop(unused_waker));
        if let Some(value) = value {
            this.abandon();
            return Poll::Ready(value);
        }
        if abandoned {
            return Poll::Ready(Err(NativeOwnedWorkerSpawnError));
        }
        if let Some(operation) = this.operation.take() {
            let response = Arc::clone(&this.response);
            let job = Box::new(move || {
                let mut value = Some(contain(operation));
                let waker = {
                    let mut response = response
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if !response.abandoned {
                        response.value = value.take();
                    }
                    response.waker.take()
                };
                // An abandoned result is destroyed by this owned worker, not
                // left in a detached task or an unbounded response queue.
                let _ = contain(|| drop(value));
                if let Some(waker) = waker {
                    let _ = contain(|| waker.wake());
                }
            });
            let Some(admit) = this.admission.take() else {
                this.abandon();
                let _ = contain(|| drop(job));
                return Poll::Ready(Err(NativeOwnedWorkerSpawnError));
            };
            if contain(|| admit(job))
                .and_then(std::convert::identity)
                .is_err()
            {
                this.abandon();
                return Poll::Ready(Err(NativeOwnedWorkerSpawnError));
            }
        }
        Poll::Pending
    }
}

impl<T> Drop for OwnedWorkerFuture<T> {
    fn drop(&mut self) {
        self.abandon();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_reentrant_waker_test::{Callback, new as reentrant_waker};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    #[test]
    fn worker_uses_collector_and_outlives_submitter_scope() {
        const SPAWNER: NativeOwnedWorkerSpawner = NativeOwnedWorkerSpawner::new();
        let caller = std::thread::current().id();
        let (started_tx, started_rx) = sync_channel(1);
        let (release_tx, release_rx) = sync_channel(1);
        let (finished_tx, finished_rx) = sync_channel(1);
        {
            let spawner = SPAWNER;
            spawner
                .spawn(move || {
                    started_tx.send(std::thread::current().id()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    finished_tx.send(()).unwrap();
                })
                .unwrap();
        }
        assert_ne!(
            started_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            caller
        );
        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    }

    fn poll<T: Send + 'static>(
        future: &mut OwnedWorkerFuture<T>,
    ) -> Poll<Result<T, NativeOwnedWorkerSpawnError>> {
        Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
    }

    fn queued(
        operation: impl FnOnce() -> usize + Send + 'static,
    ) -> (
        OwnedWorkerFuture<usize>,
        std::sync::mpsc::Receiver<OwnedJob>,
    ) {
        let (tx, rx) = sync_channel(1);
        (
            OwnedWorkerFuture::new(operation, move |job| {
                tx.send(job).unwrap();
                Ok(())
            }),
            rx,
        )
    }

    #[test]
    fn construction_and_unpolled_drop_do_not_admit_or_run() {
        let admissions = Arc::new(AtomicUsize::new(0));
        let operations = Arc::new(AtomicUsize::new(0));
        let a = Arc::clone(&admissions);
        let o = Arc::clone(&operations);
        let future = OwnedWorkerFuture::new(
            move || o.fetch_add(1, Ordering::SeqCst),
            move |_| {
                a.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        );
        assert_eq!(admissions.load(Ordering::SeqCst), 0);
        drop(future);
        assert_eq!(admissions.load(Ordering::SeqCst), 0);
        assert_eq!(operations.load(Ordering::SeqCst), 0);
        let o = Arc::clone(&operations);
        drop(NativeOwnedWorkerSpawner::new().run(move || o.fetch_add(1, Ordering::SeqCst)));
        assert_eq!(operations.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn repeated_pending_polls_submit_once_and_deliver_once() {
        let (mut future, jobs) = queued(|| 42);
        assert!(poll(&mut future).is_pending());
        assert!(poll(&mut future).is_pending());
        let job = jobs.try_recv().unwrap();
        assert!(jobs.try_recv().is_err());
        job();
        assert_eq!(poll(&mut future), Poll::Ready(Ok(42)));
        assert_eq!(
            poll(&mut future),
            Poll::Ready(Err(NativeOwnedWorkerSpawnError))
        );
    }

    #[test]
    fn all_waker_callbacks_reenter_outside_response_lock() {
        for callback in [Callback::Clone, Callback::Drop, Callback::Wake] {
            let (mut future, jobs) = queued(|| 42);
            let response = Arc::clone(&future.response);
            let (waker, handle) = reentrant_waker(callback, move || {
                assert!(
                    response.try_lock().is_ok(),
                    "waker callback held response lock"
                );
            });
            assert!(
                Pin::new(&mut future)
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            if callback == Callback::Drop {
                assert!(poll(&mut future).is_pending());
            }
            jobs.try_recv().unwrap()();
            assert_eq!(poll(&mut future), Poll::Ready(Ok(42)));
            assert!(handle.calls() > 0);
            drop(waker);
        }
    }

    #[test]
    fn panicking_replaced_waker_does_not_lose_job_or_completion() {
        let (mut future, jobs) = queued(|| 42);
        let (waker, _) = reentrant_waker(Callback::Drop, || panic!("waker drop"));
        assert!(
            Pin::new(&mut future)
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert!(poll(&mut future).is_pending());
        jobs.try_recv().unwrap()();
        assert_eq!(poll(&mut future), Poll::Ready(Ok(42)));
        assert!(contain(|| drop(waker)).is_err());
    }

    #[test]
    fn panicking_wake_keeps_completion_available() {
        let (mut future, jobs) = queued(|| 42);
        let (waker, _) = reentrant_waker(Callback::Wake, || panic!("waker wake"));
        assert!(
            Pin::new(&mut future)
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        jobs.try_recv().unwrap()();
        assert_eq!(poll(&mut future), Poll::Ready(Ok(42)));
    }

    #[test]
    fn panicking_clone_before_admission_returns_fixed_error() {
        let (mut future, jobs) = queued(|| panic!("must not execute"));
        let (waker, _) = reentrant_waker(Callback::Clone, || panic!("waker clone"));
        assert_eq!(
            Pin::new(&mut future).poll(&mut Context::from_waker(&waker)),
            Poll::Ready(Err(NativeOwnedWorkerSpawnError))
        );
        assert!(jobs.try_recv().is_err());
    }

    #[test]
    fn panicking_clone_after_admission_does_not_cancel_owned_job() {
        let executed = Arc::new(AtomicBool::new(false));
        let executed_by_job = Arc::clone(&executed);
        let (mut future, jobs) = queued(move || {
            executed_by_job.store(true, Ordering::SeqCst);
            42
        });
        assert!(poll(&mut future).is_pending());
        let (waker, _) = reentrant_waker(Callback::Clone, || panic!("waker clone"));
        assert_eq!(
            Pin::new(&mut future).poll(&mut Context::from_waker(&waker)),
            Poll::Ready(Err(NativeOwnedWorkerSpawnError))
        );
        drop(future);
        jobs.try_recv().unwrap()();
        assert!(executed.load(Ordering::SeqCst));
    }

    #[test]
    fn operation_panic_returns_fixed_error_without_dropping_opaque_payload() {
        struct HostilePayload(Arc<AtomicBool>);
        impl Drop for HostilePayload {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
                panic!("opaque payload destructor");
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let payload = HostilePayload(Arc::clone(&dropped));
        let (mut future, jobs) = queued(move || std::panic::panic_any(payload));
        assert!(poll(&mut future).is_pending());
        jobs.try_recv().unwrap()();
        assert_eq!(
            poll(&mut future),
            Poll::Ready(Err(NativeOwnedWorkerSpawnError))
        );
        assert!(!dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn admission_failure_does_not_execute_operation() {
        let executed = Arc::new(AtomicBool::new(false));
        let executed_by_job = Arc::clone(&executed);
        let mut future = OwnedWorkerFuture::new(
            move || executed_by_job.store(true, Ordering::SeqCst),
            |_| Err(NativeOwnedWorkerSpawnError),
        );
        assert_eq!(
            poll(&mut future),
            Poll::Ready(Err(NativeOwnedWorkerSpawnError))
        );
        assert!(!executed.load(Ordering::SeqCst));
    }

    #[test]
    fn production_receipt_wakes_executor_and_accepts_non_unpin_result() {
        struct PinnedValue {
            value: usize,
            _pin: std::marker::PhantomPinned,
        }
        let result =
            futures_executor::block_on(NativeOwnedWorkerSpawner::new().run(|| PinnedValue {
                value: 42,
                _pin: std::marker::PhantomPinned,
            }))
            .unwrap();
        assert_eq!(result.value, 42);
        assert_eq!(
            futures_executor::block_on(
                NativeOwnedWorkerSpawner::new().run(|| panic!("job failed"))
            ),
            Err::<(), _>(NativeOwnedWorkerSpawnError)
        );
    }

    #[test]
    fn abandoned_response_drops_result_outside_lock_even_if_destructor_panics() {
        struct HostileResult {
            reenter: Box<dyn Fn() + Send>,
        }
        impl Drop for HostileResult {
            fn drop(&mut self) {
                (self.reenter)();
                panic!("result destructor");
            }
        }
        let response_slot = Arc::new(Mutex::new(None::<Arc<Mutex<Response<HostileResult>>>>));
        let response_by_job = Arc::clone(&response_slot);
        let (tx, rx) = sync_channel(1);
        let mut future = OwnedWorkerFuture::new(
            move || HostileResult {
                reenter: Box::new(move || {
                    let slot = response_by_job.lock().unwrap();
                    assert!(slot.as_ref().unwrap().try_lock().is_ok());
                }),
            },
            move |job| {
                tx.send(job).unwrap();
                Ok(())
            },
        );
        *response_slot.lock().unwrap() = Some(Arc::clone(&future.response));
        assert!(poll(&mut future).is_pending());
        drop(future);
        rx.try_recv().unwrap()();
        assert!(
            response_slot
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .value
                .is_none()
        );
    }

    #[test]
    fn production_run_executes_off_poll_thread_and_cleans_abandoned_result() {
        struct Receipt(std::sync::mpsc::SyncSender<std::thread::ThreadId>);
        impl Drop for Receipt {
            fn drop(&mut self) {
                self.0.send(std::thread::current().id()).unwrap();
            }
        }
        let caller = std::thread::current().id();
        let (started_tx, started_rx) = sync_channel(1);
        let (release_tx, release_rx) = sync_channel(1);
        let (dropped_tx, dropped_rx) = sync_channel(1);
        let mut future = NativeOwnedWorkerSpawner::new().run(move || {
            started_tx.send(std::thread::current().id()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            Receipt(dropped_tx)
        });
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let worker = started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_ne!(worker, caller);
        drop(future);
        release_tx.send(()).unwrap();
        assert_eq!(
            dropped_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            worker
        );
    }

    #[test]
    fn panicking_wake_does_not_strand_production_worker_cleanup() {
        struct ThreadExit(std::sync::mpsc::SyncSender<()>);
        impl Drop for ThreadExit {
            fn drop(&mut self) {
                self.0.send(()).unwrap();
            }
        }
        thread_local! {
            static EXIT: std::cell::RefCell<Option<ThreadExit>> = const { std::cell::RefCell::new(None) };
        }
        let (exit_tx, exit_rx) = sync_channel(1);
        let (release_tx, release_rx) = sync_channel(1);
        let mut future = NativeOwnedWorkerSpawner::new().run(move || {
            EXIT.with(|slot| *slot.borrow_mut() = Some(ThreadExit(exit_tx)));
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            42
        });
        let (waker, _) = reentrant_waker(Callback::Wake, || panic!("worker wake"));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        release_tx.send(()).unwrap();
        exit_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(Ok(42))
        );
    }
}
