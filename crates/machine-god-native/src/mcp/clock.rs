//! Production MCP timing on the host's existing Tokio runtime.

use super::{http::McpHttpClock, runtime::NativeMcpRuntimeClock};
use machine_god_core::BoxFuture;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Explicit monotonic clock selection shared by native runtime/startup and HTTP.
/// Construction is inert. Timers use the existing host runtime only when polled;
/// no thread, runtime, detached task or independent cleanup owner is created.
#[derive(Clone, Copy, Debug, Default)]
pub struct TokioMcpClock;

impl NativeMcpRuntimeClock for TokioMcpClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move { tokio::time::sleep_until(deadline.into()).await })
    }
}

impl McpHttpClock for TokioMcpClock {
    fn now(&self) -> Instant {
        NativeMcpRuntimeClock::now(self)
    }

    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        NativeMcpRuntimeClock::sleep_until(self, deadline)
    }
}

impl super::auth::McpAuthClock for TokioMcpClock {
    fn unix_millis(&self) -> i64 {
        unix_millis(SystemTime::now())
    }
}

fn unix_millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(error) => {
            i64::try_from(error.duration().as_millis()).map_or(i64::MIN, i64::saturating_neg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn persisted_expiry_uses_signed_wall_time_without_changing_monotonic_timers() {
        assert_eq!(unix_millis(UNIX_EPOCH), 0);
        assert_eq!(
            unix_millis(UNIX_EPOCH.checked_add(Duration::from_millis(17)).unwrap()),
            17
        );
        assert_eq!(
            unix_millis(UNIX_EPOCH.checked_sub(Duration::from_millis(17)).unwrap()),
            -17
        );
    }

    #[test]
    fn clock_and_unpolled_deadlines_need_no_tokio_runtime() {
        let clock = TokioMcpClock;
        let before = Instant::now();
        assert!(NativeMcpRuntimeClock::now(&clock) >= before);
        assert!(McpHttpClock::now(&clock) >= before);
        let deadline = before.checked_add(Duration::from_secs(60)).unwrap();
        drop(NativeMcpRuntimeClock::sleep_until(&clock, deadline));
        drop(McpHttpClock::sleep_until(&clock, deadline));
    }

    #[test]
    fn both_deadlines_use_the_selected_runtime_and_can_be_dropped() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(async {
                let clock = TokioMcpClock;
                let deadline = Instant::now().checked_add(Duration::from_secs(60)).unwrap();
                let mut native = NativeMcpRuntimeClock::sleep_until(&clock, deadline);
                let mut http = McpHttpClock::sleep_until(&clock, deadline);
                let mut abandoned = McpHttpClock::sleep_until(&clock, deadline);
                assert!(futures_util::poll!(&mut native).is_pending());
                assert!(futures_util::poll!(&mut http).is_pending());
                assert!(futures_util::poll!(&mut abandoned).is_pending());
                drop(abandoned);
                tokio::time::advance(Duration::from_secs(61)).await;
                assert!(futures_util::poll!(&mut native).is_ready());
                assert!(futures_util::poll!(&mut http).is_ready());
            });
    }
}
