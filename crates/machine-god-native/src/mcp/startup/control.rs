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
    pub fn deadline(&self, milliseconds: u32, outer: Option<Instant>) -> Result<Instant> {
        self.now()
            .checked_add(std::time::Duration::from_millis(u64::from(milliseconds)))
            .map(|value| outer.map_or(value, |outer| value.min(outer)))
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
    check_optional(clock, guards, Some(deadline))
}

pub(super) fn check_optional(
    clock: &Clock,
    guards: &[CancellationToken],
    deadline: Option<Instant>,
) -> Result<()> {
    if guards.iter().any(CancellationToken::is_cancelled) {
        Err(Error::Cancelled)
    } else if deadline.is_some_and(|deadline| clock.now() >= deadline) {
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
    bounded_optional(future, clock, guards, Some(deadline)).await
}

pub(super) fn housekeeping_deadline(clock: &Clock, outer: Option<Instant>) -> Result<Instant> {
    clock
        .now()
        .checked_add(std::time::Duration::from_secs(30))
        .map(|deadline| outer.map_or(deadline, |outer| deadline.min(outer)))
        .ok_or(Error::Limit)
}

pub(super) async fn bounded_optional<F: Future>(
    future: F,
    clock: &Clock,
    guards: &[CancellationToken],
    deadline: Option<Instant>,
) -> Result<F::Output> {
    check_optional(clock, guards, deadline)?;
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
        let elapsed = async {
            match deadline {
                Some(deadline) => clock.0.sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        };
        select(Box::pin(cancelled), Box::pin(elapsed)).await;
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
    check_optional(clock, guards, deadline)?;
    Ok(result)
}
