//! Tokio-backed deadline authority for native web search.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use machine_god_core::BoxFuture;
use tokio::runtime::{Handle, Id, Runtime};
use tokio::task::JoinHandle;

use crate::{WebSearchDeadline, WebSearchTransportError, WebSearchTransportErrorKind};

/// Deadline authority paired with one time-and-I/O-enabled Tokio runtime.
///
/// The private binding prevents a host from constructing this adapter for a
/// driverless runtime. A wait schedules its timer onto the proven paired
/// runtime, so runtime shutdown cancels the timer instead of racing timer API
/// use on another executor.
pub struct TokioWebSearchDeadline {
    handle: Handle,
    runtime_id: Id,
}

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
    pub fn build_runtime_pair() -> Result<(Runtime, Self), WebSearchTransportError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| runtime_required())?;
        let handle = runtime.handle().clone();
        let runtime_id = handle.id();
        let deadline = Self { handle, runtime_id };
        Ok((runtime, deadline))
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
            task: None,
        })
    }
}

struct TokioWebSearchWait<'a> {
    authority: &'a TokioWebSearchDeadline,
    deadline: Instant,
    task: Option<JoinHandle<()>>,
}

impl Future for TokioWebSearchWait<'_> {
    type Output = Result<(), WebSearchTransportError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.task.is_none() {
            let Ok(current) = Handle::try_current() else {
                return Poll::Ready(Err(runtime_required()));
            };
            if current.id() != self.authority.runtime_id
                || self.authority.handle.id() != self.authority.runtime_id
            {
                return Poll::Ready(Err(runtime_required()));
            }

            let deadline = self.deadline;
            self.task = Some(self.authority.handle.spawn(async move {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            }));
        }

        let task_poll = match self.task.as_mut() {
            Some(task) => Pin::new(task).poll(context),
            None => return Poll::Ready(Err(runtime_required())),
        };
        match task_poll {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(())) => {
                self.task.take();
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(_)) => {
                self.task.take();
                Poll::Ready(Err(runtime_required()))
            }
        }
    }
}

impl Drop for TokioWebSearchWait<'_> {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

const fn runtime_required() -> WebSearchTransportError {
    WebSearchTransportError::new(WebSearchTransportErrorKind::RuntimeRequired)
}

#[cfg(test)]
mod tests {
    use std::future::poll_fn;
    use std::sync::{Arc, Barrier};
    use std::task::Poll;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

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
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        let paired_handle = runtime.handle().clone();
        drop(runtime);

        let _entered = paired_handle.enter();
        let error = futures_executor::block_on(deadline.wait_until(Instant::now())).unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
    }

    #[test]
    fn concurrent_runtime_shutdown_and_first_poll_return_fixed_error_without_panicking() {
        for _ in 0..32 {
            let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
            let paired_handle = runtime.handle().clone();
            let barrier = Arc::new(Barrier::new(2));

            let error = thread::scope(|scope| {
                let waiter_barrier = Arc::clone(&barrier);
                let waiter = scope.spawn(move || {
                    let _entered = paired_handle.enter();
                    waiter_barrier.wait();
                    futures_executor::block_on(
                        deadline.wait_until(Instant::now() + Duration::from_secs(60)),
                    )
                    .unwrap_err()
                });
                let shutdown = scope.spawn(move || {
                    barrier.wait();
                    drop(runtime);
                });

                let error = waiter.join().unwrap();
                shutdown.join().unwrap();
                error
            });
            assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
        }
    }

    #[test]
    fn dropping_wait_aborts_the_runtime_owned_timer_task() {
        let (runtime, deadline) = TokioWebSearchDeadline::build_runtime_pair().unwrap();
        runtime.block_on(async {
            let mut wait = deadline.wait_until(Instant::now() + Duration::from_secs(60));
            poll_fn(|context| {
                assert!(wait.as_mut().poll(context).is_pending());
                Poll::Ready(())
            })
            .await;
            assert_eq!(Handle::current().metrics().num_alive_tasks(), 1);

            drop(wait);
            tokio::task::yield_now().await;
            assert_eq!(Handle::current().metrics().num_alive_tasks(), 0);
        });
    }
}
