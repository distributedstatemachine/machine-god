//! Tokio-backed deadline authority for native web search.

use std::time::Instant;

use machine_god_core::BoxFuture;

use crate::{WebSearchDeadline, WebSearchTransportError, WebSearchTransportErrorKind};

/// Inert adapter from an absolute standard-library deadline to Tokio time.
///
/// Constructing the adapter or a wait future performs no runtime or timer
/// work. Polling a wait outside a Tokio runtime returns the fixed
/// [`WebSearchTransportErrorKind::RuntimeRequired`] failure instead of
/// invoking a Tokio timer API that would panic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokioWebSearchDeadline;

impl TokioWebSearchDeadline {
    /// Constructs an inert Tokio deadline adapter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl WebSearchDeadline for TokioWebSearchDeadline {
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(async move {
            if tokio::runtime::Handle::try_current().is_err() {
                return Err(WebSearchTransportError::new(
                    WebSearchTransportErrorKind::RuntimeRequired,
                ));
            }
            let sleep = std::panic::catch_unwind(|| {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline))
            })
            .map_err(|_| {
                WebSearchTransportError::new(WebSearchTransportErrorKind::RuntimeRequired)
            })?;
            sleep.await;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn construction_and_unpolled_wait_are_runtime_independent() {
        let deadline = TokioWebSearchDeadline::new();
        drop(deadline.wait_until(Instant::now()));
        assert_eq!(deadline, TokioWebSearchDeadline);
    }

    #[test]
    fn polling_without_a_runtime_returns_fixed_runtime_required() {
        let error =
            futures_executor::block_on(TokioWebSearchDeadline::new().wait_until(Instant::now()))
                .unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
        assert_eq!(
            format!("{error:?} {error}"),
            "WebSearchTransportError { kind: RuntimeRequired } web-search transport failed"
        );
    }

    #[test]
    fn active_time_enabled_runtime_drives_the_absolute_deadline() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let deadline = Instant::now() + Duration::from_millis(1);
            TokioWebSearchDeadline::new()
                .wait_until(deadline)
                .await
                .unwrap();
            assert!(Instant::now() >= deadline);
        });
    }

    #[test]
    fn active_runtime_without_time_returns_fixed_runtime_required() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let error = runtime
            .block_on(
                TokioWebSearchDeadline::new().wait_until(Instant::now() + Duration::from_millis(1)),
            )
            .unwrap_err();
        assert_eq!(error.kind(), WebSearchTransportErrorKind::RuntimeRequired);
    }
}
