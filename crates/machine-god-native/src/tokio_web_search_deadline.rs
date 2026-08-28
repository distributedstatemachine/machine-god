//! Tokio-backed deadline authority for native web search.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use machine_god_core::BoxFuture;
use tokio::runtime::{Handle, Id, Runtime};

use crate::{WebSearchDeadline, WebSearchTransportError, WebSearchTransportErrorKind};

/// Owned current-thread Tokio runtime paired with a web-search deadline.
///
/// The wrapper exposes `block_on` instead of its underlying runtime. Code
/// running inside `block_on` may clone the current handle, so the paired
/// deadline also checks private liveness state before using Tokio time.
pub struct TokioWebSearchRuntime {
    runtime: Runtime,
    runtime_live: Arc<AtomicBool>,
}

impl TokioWebSearchRuntime {
    /// Runs a future on the paired time-and-I/O-enabled runtime.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

impl fmt::Debug for TokioWebSearchRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokioWebSearchRuntime")
    }
}

impl Drop for TokioWebSearchRuntime {
    fn drop(&mut self) {
        self.runtime_live.store(false, Ordering::Release);
    }
}

/// Deadline authority paired with one time-and-I/O-enabled Tokio runtime.
///
/// The private binding prevents a host from constructing this adapter for a
/// driverless runtime. The retained handle keeps the paired runtime identity
/// alive, while private shared state rejects use after the runtime owner drops.
pub struct TokioWebSearchDeadline {
    handle: Handle,
    runtime_id: Id,
    runtime_live: Arc<AtomicBool>,
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
    pub fn build_runtime_pair() -> Result<(TokioWebSearchRuntime, Self), WebSearchTransportError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| runtime_required())?;
        let handle = runtime.handle().clone();
        let runtime_id = handle.id();
        let runtime_live = Arc::new(AtomicBool::new(true));
        let deadline = Self {
            handle,
            runtime_id,
            runtime_live: Arc::clone(&runtime_live),
        };
        let runtime = TokioWebSearchRuntime {
            runtime,
            runtime_live,
        };
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
        Box::pin(async move {
            if !self.runtime_live.load(Ordering::Acquire) {
                return Err(runtime_required());
            }
            let current = Handle::try_current().map_err(|_| runtime_required())?;
            if current.id() != self.runtime_id || self.handle.id() != self.runtime_id {
                return Err(runtime_required());
            }
            if !self.runtime_live.load(Ordering::Acquire) {
                return Err(runtime_required());
            }

            let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
            sleep.await;
            Ok(())
        })
    }
}

const fn runtime_required() -> WebSearchTransportError {
    WebSearchTransportError::new(WebSearchTransportErrorKind::RuntimeRequired)
}

#[cfg(test)]
mod tests {
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
        let paired_handle = runtime.runtime.handle().clone();
        drop(runtime);

        let _entered = paired_handle.enter();
        let error = futures_executor::block_on(deadline.wait_until(Instant::now())).unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
    }
}
