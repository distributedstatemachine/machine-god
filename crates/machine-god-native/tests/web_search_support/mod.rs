use std::future;
use std::sync::Arc;
use std::time::Instant;

use machine_god_core::{BoxFuture, NetworkTarget};
use machine_god_native::{WebSearchDeadline, WebSearchTransportError};

pub struct NeverDeadline;

impl WebSearchDeadline for NeverDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(future::pending())
    }
}

pub fn never_deadline() -> Arc<dyn WebSearchDeadline> {
    Arc::new(NeverDeadline)
}

pub fn production_gateway_target() -> NetworkTarget {
    NetworkTarget {
        scheme: "https".to_owned(),
        host: "ai-gateway.vercel.sh".to_owned(),
        port: None,
    }
}
