//! Optional startup cap and separately finite housekeeping stages.

use super::{Failure, NativeMcpControllerError, NativeMcpControllerOptions};
use futures_util::future::{Either, select};
use std::{
    future::Future,
    time::{Duration, Instant},
};

pub(super) async fn elapsed(options: &NativeMcpControllerOptions, deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => options.startup.clock.sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

pub(super) fn housekeeping_deadline(
    options: &NativeMcpControllerOptions,
    outer: Option<Instant>,
) -> Result<Instant, Failure> {
    options
        .startup
        .clock
        .now()
        .checked_add(Duration::from_secs(30))
        .map(|deadline| outer.map_or(deadline, |outer| deadline.min(outer)))
        .ok_or_else(|| NativeMcpControllerError::Limit.into())
}

/// The owned worker closure retains its generation reservation even when the
/// caller stops awaiting this stage. No detached observer or retry is created.
pub(super) async fn housekeeping<T>(
    options: &NativeMcpControllerOptions,
    outer: Option<Instant>,
    operation: impl Future<Output = T>,
) -> Result<T, Failure> {
    let deadline = housekeeping_deadline(options, outer)?;
    if options.startup.clock.now() >= deadline {
        return Err(NativeMcpControllerError::Deadline.into());
    }
    let result = match select(
        Box::pin(operation),
        options.startup.clock.sleep_until(deadline),
    )
    .await
    {
        Either::Left((result, _)) => result,
        Either::Right(_) => return Err(NativeMcpControllerError::Deadline.into()),
    };
    if options.startup.clock.now() >= deadline {
        return Err(NativeMcpControllerError::Deadline.into());
    }
    Ok(result)
}
