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
    let permit = runtime
        .acquire_file_control()
        .map_err(|_| Error::Unavailable)?;
    let principal = BackgroundOutputOwner::new(runtime.id(), runtime.incarnation_id());
    let future = requester.snapshot(principal.clone(), cancellation);
    Ok((
        principal,
        Box::pin(async move {
            let result = future.await.map_err(|_| Error::Unavailable);
            // The original lifecycle lease survives worker settlement. Neither
            // frame replacement nor closing the view drops an admitted operation.
            drop(permit);
            result
        }),
    ))
}
