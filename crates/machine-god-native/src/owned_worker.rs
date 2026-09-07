//! Inert binding to the shared, bounded native worker collector.

use crate::background_supervisor::worker_ownership_registry;
use machine_god_core::BoxFuture;
use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, Weak};
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

/// Explicit enrollment for one host's native workers and transferred child reap
/// obligations. Clones carry no host-lifetime vote. Drop does not close the scope:
/// the actual host resource must call [`Self::close`] before requesting cleanup.
#[derive(Clone, Default)]
pub struct NativeOwnedWorkerScope {
    state: Arc<ScopeState>,
}

/// Observation only: retaining this value cannot admit work or retain a host,
/// operation, response value, process handle or runtime owner.
#[derive(Clone)]
pub struct NativeOwnedWorkerCompletion {
    state: Arc<ScopeState>,
}

/// Metadata-only continuation of already enrolled cleanup. Native adapters may
/// retain it alongside transferred reap authority. It grants no admission,
/// native capability or host-lifetime vote; dropping it discharges only this
/// reference to the existing completion obligation.
#[derive(Clone)]
pub struct NativeOwnedWorkerCleanup {
    #[cfg_attr(
        not(any(test, target_os = "linux", target_os = "macos")),
        allow(
            dead_code,
            reason = "retains completion metadata without platform cleanup effects"
        )
    )]
    ticket: NativeOwnedWorkerTicket,
}
impl NativeOwnedWorkerCleanup {
    /// Attributes nested cleanup obligations to this original scope while a
    /// shared cleanup worker services it. Restore the previous attribution on
    /// return or unwind; unlike a dedicated worker, this thread serves other
    /// scopes afterward. Cloned cleanup tokens retain the existing ticket only.
    #[cfg(any(test, target_os = "linux", target_os = "macos"))]
    pub fn run_on_cleanup_worker<T>(&self, operation: impl FnOnce() -> T) -> T {
        struct RestoreTicket<'a> {
            slot: &'a RefCell<Option<Weak<ScopeTicket>>>,
            previous: Option<Weak<ScopeTicket>>,
        }
        impl Drop for RestoreTicket<'_> {
            fn drop(&mut self) {
                drop(self.slot.replace(self.previous.take()));
            }
        }

        WORKER_TICKET.with(|slot| {
            let _restore = RestoreTicket {
                slot,
                previous: slot.replace(Some(Arc::downgrade(&self.ticket.0))),
            };
            operation()
        })
    }
}
impl fmt::Debug for NativeOwnedWorkerCleanup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeOwnedWorkerCleanup")
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct ScopeState {
    status: Mutex<ScopeStatus>,
    wake: Condvar,
}

#[derive(Default)]
struct ScopeStatus {
    closed: bool,
    tickets: usize,
}

struct ScopeTicket {
    state: Arc<ScopeState>,
}

/// Metadata only. Clones extend an already admitted cleanup obligation; they
/// cannot create processes or authorize a new worker after scope closure.
#[derive(Clone)]
pub(crate) struct NativeOwnedWorkerTicket(Arc<ScopeTicket>);

thread_local! {
    static WORKER_TICKET: RefCell<Option<Weak<ScopeTicket>>> = const { RefCell::new(None) };
}

/// Used only by existing child-reap reservation while inside an explicitly
/// scoped worker. The ordinary zero-state spawner never installs this metadata.
pub(crate) fn current_worker_ticket() -> Option<NativeOwnedWorkerTicket> {
    WORKER_TICKET
        .try_with(|slot| slot.borrow().as_ref().and_then(Weak::upgrade))
        .ok()
        .flatten()
        .map(NativeOwnedWorkerTicket)
}

impl NativeOwnedWorkerTicket {
    pub(crate) fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        // Each collector job owns a dedicated thread. Keep weak metadata through
        // thread-local destruction; the collector retains the strong ticket
        // until join, so cleanup can still enroll and self-waits are rejected.
        WORKER_TICKET.with(|slot| *slot.borrow_mut() = Some(Arc::downgrade(&self.0)));
        operation()
    }
}

impl Drop for ScopeTicket {
    fn drop(&mut self) {
        let mut status = self
            .state
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        status.tickets -= 1;
        let complete = status.closed && status.tickets == 0;
        drop(status);
        if complete {
            self.state.wake.notify_all();
        }
    }
}

impl fmt::Debug for NativeOwnedWorkerScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeOwnedWorkerScope")
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for NativeOwnedWorkerCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeOwnedWorkerCompletion")
            .finish_non_exhaustive()
    }
}

impl NativeOwnedWorkerScope {
    /// Retains completion metadata only when called inside an explicitly scoped
    /// worker. An unscoped worker receives `None`; no ambient authority is added.
    /// Keep this token with existing cleanup ownership, never with response data.
    #[must_use]
    pub fn retain_current_cleanup() -> Option<NativeOwnedWorkerCleanup> {
        current_worker_ticket().map(|ticket| NativeOwnedWorkerCleanup { ticket })
    }

    /// Inert: no worker, collector, process or native reservation is created.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically prevents new admissions. Already admitted operations still own
    /// their cleanup; the host separately cancels them using its existing token.
    pub fn close(&self) {
        let mut status = self
            .state
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        status.closed = true;
        drop(status);
        self.state.wake.notify_all();
    }

    /// Returns a handle which observes settlement without retaining host life.
    #[must_use]
    pub fn completion(&self) -> NativeOwnedWorkerCompletion {
        NativeOwnedWorkerCompletion {
            state: Arc::clone(&self.state),
        }
    }

    fn admit(&self) -> Result<NativeOwnedWorkerTicket, NativeOwnedWorkerSpawnError> {
        let mut status = self
            .state
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if status.closed {
            return Err(NativeOwnedWorkerSpawnError);
        }
        status.tickets = status
            .tickets
            .checked_add(1)
            .ok_or(NativeOwnedWorkerSpawnError)?;
        Ok(NativeOwnedWorkerTicket(Arc::new(ScopeTicket {
            state: Arc::clone(&self.state),
        })))
    }

    /// Runs one job through the existing collector and enrolls its actual thread
    /// completion, including thread-local destructors and quarantined child reap.
    ///
    /// # Errors
    /// Rejects closed scopes, collector capacity and native thread admission.
    /// A rejected operation never executes and retains no completion ticket.
    pub fn spawn(
        &self,
        operation: impl FnOnce() + Send + 'static,
    ) -> Result<(), NativeOwnedWorkerSpawnError> {
        self.spawn_with(operation, |ticket, operation| {
            let registry = worker_ownership_registry().map_err(|()| NativeOwnedWorkerSpawnError)?;
            let reservation = registry
                .reserve_partitioned(&[1])
                .map_err(|()| NativeOwnedWorkerSpawnError)?
                .pop()
                .ok_or(NativeOwnedWorkerSpawnError)?;
            reservation
                .spawn_one_scoped(registry, "machine-god-owned-worker", ticket, operation)
                .map_err(|()| NativeOwnedWorkerSpawnError)
        })
    }

    fn spawn_with(
        &self,
        operation: impl FnOnce() + Send + 'static,
        spawn: impl FnOnce(NativeOwnedWorkerTicket, OwnedJob) -> Result<(), NativeOwnedWorkerSpawnError>,
    ) -> Result<(), NativeOwnedWorkerSpawnError> {
        let ticket = self.admit()?;
        spawn(ticket, Box::new(operation))
    }

    /// Inert until polled. Scope completion never waits for this response to be
    /// consumed: the collector owns the completion ticket, not the result tuple.
    pub fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> T + Send + 'static,
    ) -> BoxFuture<'static, Result<T, NativeOwnedWorkerSpawnError>> {
        let scope = self.clone();
        Box::pin(OwnedWorkerFuture::new(operation, move |job| {
            scope.spawn(job)
        }))
    }
}

impl NativeOwnedWorkerCompletion {
    /// True only after closure and settlement of every enrolled worker/reap
    /// obligation. An empty but still-open scope is not complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        let status = self
            .state
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        status.closed && status.tickets == 0
    }

    /// Blocks a dedicated caller worker until this scope closes and settles.
    /// This does not stop or drain unrelated workers. Never call on an async
    /// polling thread; no timeout converts incomplete cleanup into success.
    ///
    /// # Errors
    /// Rejects a wait from an enrolled worker in this same scope, which would
    /// otherwise wait for its own thread to be joined.
    pub fn wait_on_worker(&self) -> Result<(), NativeOwnedWorkerSpawnError> {
        if current_worker_ticket().is_some_and(|ticket| Arc::ptr_eq(&ticket.0.state, &self.state)) {
            return Err(NativeOwnedWorkerSpawnError);
        }
        let mut status = self
            .state
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !status.closed || status.tickets != 0 {
            status = self
                .state
                .wake
                .wait(status)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        Ok(())
    }
}

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
    fn cleanup_worker_restores_context_and_retains_nested_completion() {
        assert!(current_worker_ticket().is_none());
        let scope = NativeOwnedWorkerScope::new();
        let cleanup = NativeOwnedWorkerCleanup {
            ticket: scope.admit().unwrap(),
        };
        scope.close();
        let (value, nested) = cleanup.run_on_cleanup_worker(|| {
            let current = current_worker_ticket().unwrap();
            assert!(Arc::ptr_eq(&current.0.state, &scope.state));
            assert_eq!(
                scope.completion().wait_on_worker(),
                Err(NativeOwnedWorkerSpawnError)
            );
            (
                42,
                NativeOwnedWorkerScope::retain_current_cleanup().unwrap(),
            )
        });
        assert_eq!(value, 42);
        assert!(current_worker_ticket().is_none());
        assert_eq!(scope.state.status.lock().unwrap().tickets, 1);
        drop(cleanup);
        assert!(!scope.completion().is_complete());
        let last = nested.clone();
        drop(nested);
        assert!(!scope.completion().is_complete());
        drop(last);
        assert!(scope.completion().is_complete());
    }

    #[test]
    fn cleanup_worker_nested_scopes_restore_the_previous_ticket() {
        assert!(current_worker_ticket().is_none());
        let outer = NativeOwnedWorkerScope::new();
        let inner = NativeOwnedWorkerScope::new();
        let outer_cleanup = NativeOwnedWorkerCleanup {
            ticket: outer.admit().unwrap(),
        };
        let inner_cleanup = NativeOwnedWorkerCleanup {
            ticket: inner.admit().unwrap(),
        };
        outer.close();
        inner.close();
        outer_cleanup.run_on_cleanup_worker(|| {
            assert!(Arc::ptr_eq(
                &current_worker_ticket().unwrap().0.state,
                &outer.state
            ));
            inner_cleanup.run_on_cleanup_worker(|| {
                assert!(Arc::ptr_eq(
                    &current_worker_ticket().unwrap().0.state,
                    &inner.state
                ));
            });
            assert!(Arc::ptr_eq(
                &current_worker_ticket().unwrap().0.state,
                &outer.state
            ));
        });
        assert!(current_worker_ticket().is_none());
        drop(inner_cleanup);
        assert!(inner.completion().is_complete());
        assert!(!outer.completion().is_complete());
        drop(outer_cleanup);
        assert!(outer.completion().is_complete());
    }

    #[test]
    fn cleanup_worker_unwind_restores_nested_and_outer_contexts() {
        assert!(current_worker_ticket().is_none());
        let outer = NativeOwnedWorkerScope::new();
        let inner = NativeOwnedWorkerScope::new();
        let outer_cleanup = NativeOwnedWorkerCleanup {
            ticket: outer.admit().unwrap(),
        };
        let inner_cleanup = NativeOwnedWorkerCleanup {
            ticket: inner.admit().unwrap(),
        };
        assert!(
            contain(|| outer_cleanup.run_on_cleanup_worker(|| {
                assert!(
                    contain(|| inner_cleanup.run_on_cleanup_worker(|| {
                        panic!("nested cleanup unwind");
                    }))
                    .is_err()
                );
                assert!(Arc::ptr_eq(
                    &current_worker_ticket().unwrap().0.state,
                    &outer.state
                ));
                panic!("outer cleanup unwind");
            }))
            .is_err()
        );
        assert!(current_worker_ticket().is_none());
        outer.close();
        inner.close();
        assert!(!outer.completion().is_complete());
        assert!(!inner.completion().is_complete());
        drop((outer_cleanup, inner_cleanup));
        assert!(outer.completion().is_complete());
        assert!(inner.completion().is_complete());
    }

    #[test]
    fn cleanup_worker_sequential_jobs_do_not_attribute_unrelated_cleanup() {
        assert!(current_worker_ticket().is_none());
        for _ in 0..2 {
            let scope = NativeOwnedWorkerScope::new();
            let cleanup = NativeOwnedWorkerCleanup {
                ticket: scope.admit().unwrap(),
            };
            scope.close();
            cleanup.run_on_cleanup_worker(|| {
                assert!(Arc::ptr_eq(
                    &current_worker_ticket().unwrap().0.state,
                    &scope.state
                ));
            });
            assert!(NativeOwnedWorkerScope::retain_current_cleanup().is_none());
            drop(cleanup);
            assert!(scope.completion().is_complete());
        }
    }

    #[test]
    fn scoped_dormant_close_rejects_unpolled_and_later_admissions() {
        let scope = NativeOwnedWorkerScope::new();
        let completion = scope.completion();
        assert!(!completion.is_complete());
        let future = scope.run(|| panic!("closed scope must not execute"));
        assert_eq!(scope.state.status.lock().unwrap().tickets, 0);
        scope.close();
        assert!(completion.is_complete());
        assert_eq!(
            futures_executor::block_on(future),
            Err::<(), _>(NativeOwnedWorkerSpawnError)
        );
        assert_eq!(
            scope.spawn(|| panic!("closed spawn")),
            Err(NativeOwnedWorkerSpawnError)
        );
        completion.wait_on_worker().unwrap();
        assert!(completion.is_complete());
    }

    #[test]
    fn scoped_failed_spawn_discharges_admission_without_execution() {
        let scope = NativeOwnedWorkerScope::new();
        let result = scope.spawn_with(
            || panic!("failed native spawn"),
            |ticket, operation| {
                assert_eq!(scope.state.status.lock().unwrap().tickets, 1);
                drop(operation);
                drop(ticket);
                Err(NativeOwnedWorkerSpawnError)
            },
        );
        assert_eq!(result, Err(NativeOwnedWorkerSpawnError));
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert_eq!(scope.state.status.lock().unwrap().tickets, 0);
    }

    #[test]
    fn scoped_completion_ignores_unconsumed_response_and_unrelated_worker() {
        let unrelated = NativeOwnedWorkerScope::new();
        let (started, started_rx) = sync_channel(1);
        let (release, release_rx) = sync_channel(1);
        unrelated
            .spawn(move || {
                started.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        unrelated.close();
        let scope = NativeOwnedWorkerScope::new();
        let mut future = scope.run(|| 42);
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        assert!(!unrelated.completion().is_complete());
        assert_eq!(futures_executor::block_on(future), Ok(42));
        release.send(()).unwrap();
        unrelated.completion().wait_on_worker().unwrap();
    }

    #[test]
    fn scoped_completion_waits_for_collector_join_and_thread_local_cleanup() {
        struct TlsGate {
            reached: std::sync::mpsc::SyncSender<()>,
            release: std::sync::mpsc::Receiver<()>,
            completion: NativeOwnedWorkerCompletion,
        }
        impl Drop for TlsGate {
            fn drop(&mut self) {
                assert_eq!(
                    self.completion.wait_on_worker(),
                    Err(NativeOwnedWorkerSpawnError)
                );
                self.reached.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(10)).unwrap();
            }
        }
        thread_local! { static GATE: RefCell<Option<TlsGate>> = const { RefCell::new(None) }; }
        let scope = NativeOwnedWorkerScope::new();
        let (reached, reached_rx) = sync_channel(1);
        let (release, release_rx) = sync_channel(1);
        let completion = scope.completion();
        scope
            .spawn(move || {
                GATE.with(|slot| {
                    *slot.borrow_mut() = Some(TlsGate {
                        reached,
                        release: release_rx,
                        completion,
                    });
                });
            })
            .unwrap();
        scope.close();
        reached_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            !scope.completion().is_complete(),
            "job return is not thread completion"
        );
        release.send(()).unwrap();
        scope.completion().wait_on_worker().unwrap();
    }

    #[test]
    fn scoped_worker_cannot_wait_for_its_own_join_and_metadata_never_leaks() {
        let scope = NativeOwnedWorkerScope::new();
        let completion = scope.completion();
        let result = futures_executor::block_on(scope.run(move || {
            assert!(current_worker_ticket().is_some());
            completion.wait_on_worker()
        }))
        .unwrap();
        assert_eq!(result, Err(NativeOwnedWorkerSpawnError));
        assert!(current_worker_ticket().is_none());
        scope.close();
        scope.completion().wait_on_worker().unwrap();
        futures_executor::block_on(
            NativeOwnedWorkerSpawner::new().run(|| assert!(current_worker_ticket().is_none())),
        )
        .unwrap();
    }

    #[test]
    fn scoped_close_racing_admission_never_completes_before_admitted_work() {
        for _ in 0..32 {
            let scope = NativeOwnedWorkerScope::new();
            let other = scope.clone();
            let ran = Arc::new(AtomicBool::new(false));
            let worker_ran = Arc::clone(&ran);
            let result = std::thread::scope(|threads| {
                let admission = threads.spawn(move || {
                    other.spawn(move || {
                        worker_ran.store(true, Ordering::Release);
                    })
                });
                scope.close();
                admission.join().unwrap()
            });
            scope.completion().wait_on_worker().unwrap();
            assert_eq!(result.is_ok(), ran.load(Ordering::Acquire));
        }
    }

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
