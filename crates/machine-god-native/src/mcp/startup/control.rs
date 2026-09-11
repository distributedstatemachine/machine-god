use super::{NativeMcpRuntimeClock, NativeMcpStartupError as Error, Result};
use futures_util::future::{Either, select, select_all};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{future::Future, sync::Arc, time::Instant};

pub(super) struct Clock(pub Arc<dyn NativeMcpRuntimeClock>);
impl Clock {
    pub fn now(&self) -> Instant {
        self.0.now()
    }
    #[cfg(feature = "mcp-http")]
    pub fn deadline(&self, milliseconds: u32, outer: Instant) -> Result<Instant> {
        self.now()
            .checked_add(std::time::Duration::from_millis(u64::from(milliseconds)))
            .map(|value| value.min(outer))
            .ok_or(Error::Limit)
    }
}
impl crate::mcp::peer::McpPeerTimer for Clock {
    fn now(&self) -> Instant {
        self.0.now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        self.0.sleep_until(deadline)
    }
}
#[cfg(feature = "mcp-http")]
impl crate::mcp::http::McpHttpClock for Clock {
    fn now(&self) -> Instant {
        self.0.now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        self.0.sleep_until(deadline)
    }
}

pub(super) fn check(clock: &Clock, guards: &[CancellationToken], deadline: Instant) -> Result<()> {
    if guards.iter().any(CancellationToken::is_cancelled) {
        Err(Error::Cancelled)
    } else if clock.now() >= deadline {
        Err(Error::Deadline)
    } else {
        Ok(())
    }
}

pub(super) async fn bounded<F: Future>(
    future: F,
    clock: &Clock,
    guards: &[CancellationToken],
    deadline: Instant,
) -> Result<F::Output> {
    check(clock, guards, deadline)?;
    let stopped = async {
        let cancelled = async {
            if guards.is_empty() {
                std::future::pending::<()>().await;
            } else {
                select_all(
                    guards
                        .iter()
                        .map(|token| Box::pin(token.cancelled()))
                        .collect::<Vec<_>>(),
                )
                .await;
            }
        };
        select(Box::pin(cancelled), clock.0.sleep_until(deadline)).await;
    };
    let result = match select(Box::pin(future), Box::pin(stopped)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => {
            return Err(if guards.iter().any(CancellationToken::is_cancelled) {
                Error::Cancelled
            } else {
                Error::Deadline
            });
        }
    };
    check(clock, guards, deadline)?;
    Ok(result)
}
