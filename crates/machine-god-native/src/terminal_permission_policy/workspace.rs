//! Root acquisition belongs to the already-owned launch worker, not preparation.

use crate::{
    MAX_NATIVE_SANDBOX_ROOTS, NativeSandboxError, NativeSandboxRoot, NativeWorkspaceTurnScope,
};
use machine_god_core::CancellationToken;
use std::time::Instant;

pub(super) fn roots(
    scope: &NativeWorkspaceTurnScope,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<Vec<NativeSandboxRoot>, NativeSandboxError> {
    let snapshot = scope
        .snapshot()
        .map_err(|_| NativeSandboxError::Unavailable)?;
    let mut roots: Vec<NativeSandboxRoot> = Vec::with_capacity(MAX_NATIVE_SANDBOX_ROOTS);
    for identity in std::iter::once(snapshot.primary_identity()).chain(
        snapshot
            .entries()
            .iter()
            .filter(|entry| entry.active())
            .map(|entry| entry.source().identity()),
    ) {
        check(scope, deadline, cancellation)?;
        let route = snapshot
            .route(identity)
            .map_err(|_| NativeSandboxError::Unavailable)?;
        if roots
            .iter()
            .any(|root| root.canonical_path() == route.root_identity())
        {
            continue;
        }
        if roots.len() == MAX_NATIVE_SANDBOX_ROOTS {
            return Err(NativeSandboxError::Invalid);
        }
        let descriptor = route
            .root_descriptor()
            .try_clone()
            .map_err(|_| NativeSandboxError::Unavailable)?;
        check(scope, deadline, cancellation)?;
        roots.push(NativeSandboxRoot::new(
            descriptor.into(),
            route.root_identity().to_owned(),
        )?);
    }
    check(scope, deadline, cancellation)?;
    Ok(roots)
}

fn check(
    scope: &NativeWorkspaceTurnScope,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), NativeSandboxError> {
    if cancellation.is_cancelled() {
        Err(NativeSandboxError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(NativeSandboxError::Timeout)
    } else if !scope.is_live() {
        Err(NativeSandboxError::Unavailable)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
