//! One inert, host-owned resolver capture shared by hostname invocations.

use super::{SocketAddr, WebFetchTransportError, WebFetchTransportErrorKind, transport_error};
use crate::NativeOwnedWorkerScope;
use futures_util::{FutureExt, future::Shared};
use machine_god_core::BoxFuture;

pub(super) struct DeferredNameserver {
    // This keeper is never consumed by a request. Cancellation/drop releases
    // only that request's clone, so another request cannot restart discovery.
    capture: Shared<BoxFuture<'static, Result<SocketAddr, WebFetchTransportError>>>,
}

impl DeferredNameserver {
    pub(super) fn new(
        workers: &NativeOwnedWorkerScope,
        capture: impl FnOnce() -> Result<SocketAddr, WebFetchTransportError> + Send + 'static,
    ) -> Self {
        let capture = workers.run(capture);
        let capture: BoxFuture<'static, Result<SocketAddr, WebFetchTransportError>> =
            Box::pin(async move {
                capture
                    .await
                    .map_err(|_| transport_error(WebFetchTransportErrorKind::Unavailable))?
            });
        Self {
            capture: capture.shared(),
        }
    }

    pub(super) async fn snapshot(&self) -> Result<SocketAddr, WebFetchTransportError> {
        self.capture.clone().await
    }
}
