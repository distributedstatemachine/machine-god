//! Read-only terminal observations under the actual selected runtime's lease.
use super::{NativeInteractiveSession, NativeManagedNavigationError as Error};
use crate::{NativeObservedManagedAgent, NativeTerminalBackgroundSnapshot};
use machine_god_core::{BackgroundOutputOwner, BoxFuture, CancellationToken};

pub(super) type Snapshot = BoxFuture<'static, Result<NativeTerminalBackgroundSnapshot, Error>>;

pub(super) fn snapshot(
    owner: &NativeInteractiveSession,
    observed: Option<&NativeObservedManagedAgent>,
    cancellation: CancellationToken,
) -> Result<(BackgroundOutputOwner, Snapshot), Error> {
    let runtime = match observed {
        Some(observed) => owner
            .managed
            .as_ref()
            .and_then(|managed| managed.agents.observed_runtime(observed))
            .ok_or(Error::NoSelection)?,
        None => &owner.current,
    };
    let requester = owner
        .host
        .terminal_background_requester()
        .ok_or(Error::Unavailable)?;
    let (principal, future) = requester
        .observe_runtime(runtime, cancellation)
        .map_err(|_| Error::Unavailable)?;
    Ok((
        principal,
        Box::pin(async move { future.await.map_err(|_| Error::Unavailable) }),
    ))
}
