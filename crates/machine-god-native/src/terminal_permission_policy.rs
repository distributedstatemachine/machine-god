//! Explicit terminal authority bound weakly to one permission controller.

use std::fs::File;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Instant;

use machine_god_core::{CancellationToken, ToolContext};

use crate::{
    MAX_NATIVE_SANDBOX_ROOTS, NativePermissionController, NativeSandboxError, NativeSandboxLaunch,
    NativeSandboxMode, NativeSandboxRoot,
};

/// Inert authority for terminal launches. The weak controller link avoids a
/// controller → prepared tool → terminal executor → controller ownership cycle.
/// Capturing policy is not permission to execute an effect.
pub struct NativeTerminalPermissionPolicy {
    roots: Vec<NativeSandboxRoot>,
    executable: Option<File>,
    controller: OnceLock<Weak<NativePermissionController>>,
}

impl std::fmt::Debug for NativeTerminalPermissionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTerminalPermissionPolicy")
            .finish_non_exhaustive()
    }
}

impl NativeTerminalPermissionPolicy {
    /// Retains supplied authority without inspecting files or discovering roots.
    /// # Errors
    /// Rejects more than the pinned seventeen roots. Missing OS authority is
    /// rejected at capture when the taken job actually selects OS isolation.
    pub fn new(
        roots: Vec<NativeSandboxRoot>,
        executable: Option<File>,
    ) -> Result<Self, NativeSandboxError> {
        if roots.len() > MAX_NATIVE_SANDBOX_ROOTS {
            return Err(NativeSandboxError::Invalid);
        }
        Ok(Self {
            roots,
            executable,
            controller: OnceLock::new(),
        })
    }

    /// Binds once, without extending the controller's lifetime or doing I/O.
    /// # Errors
    /// Rebinding, including rebinding an expired controller, fails closed.
    pub fn bind_controller(
        &self,
        controller: &Arc<NativePermissionController>,
    ) -> Result<(), NativeSandboxError> {
        self.controller
            .set(Arc::downgrade(controller))
            .map_err(|_| NativeSandboxError::Unavailable)
    }

    /// Capture only on the caller's owned effect worker, under the original
    /// deadline. Uses the exact live turn's taken policy, never current settings.
    /// # Errors
    /// Rejects cancellation, expiry, missing/closed controller routes and invalid
    /// or unavailable OS authority. There is no unsandboxed fallback.
    pub fn capture_on_worker(
        &self,
        context: &ToolContext,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Arc<NativeSandboxLaunch>, NativeSandboxError> {
        if cancellation.is_cancelled() {
            return Err(NativeSandboxError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(NativeSandboxError::Timeout);
        }
        let controller = self
            .controller
            .get()
            .and_then(Weak::upgrade)
            .ok_or(NativeSandboxError::Unavailable)?;
        let policy = controller
            .policy_for_execution(context)
            .map_err(|_| NativeSandboxError::Unavailable)?;
        let executable = if policy.effective_sandbox_mode() == NativeSandboxMode::Os {
            self.executable
                .as_ref()
                .map(File::try_clone)
                .transpose()
                .map_err(|_| NativeSandboxError::Unavailable)?
        } else {
            None
        };
        NativeSandboxLaunch::capture(
            policy.sandbox_mode(),
            policy.mode(),
            self.roots.clone(),
            executable,
            false,
            deadline,
            cancellation,
        )
        .map(Arc::new)
    }
}

#[cfg(test)]
pub(crate) mod tests;
