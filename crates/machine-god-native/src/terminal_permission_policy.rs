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
    workspace_contexts: Option<Arc<crate::NativeWorkspaceContexts>>,
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
            workspace_contexts: None,
        })
    }

    /// Binds launch root selection to the exact live turn's captured scope.
    /// This builder performs no I/O and does not capture or duplicate roots.
    #[must_use]
    pub fn with_workspace_contexts(
        mut self,
        contexts: Arc<crate::NativeWorkspaceContexts>,
    ) -> Self {
        self.workspace_contexts = Some(contexts);
        self
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
        check_capture(deadline, cancellation)?;
        let scope = self
            .workspace_contexts
            .as_ref()
            .map(|contexts| {
                contexts
                    .snapshot_for_tool(context)
                    .map(Arc::new)
                    .map_err(|_| NativeSandboxError::Unavailable)
            })
            .transpose()?;
        self.capture_with_shared_workspace_scope(context, scope, deadline, cancellation)
    }

    /// The actual host passes its acceptance-time scope; do not rediscover a
    /// potentially replaced registration between cwd and sandbox root selection.
    pub(crate) fn capture_with_shared_workspace_scope(
        &self,
        context: &ToolContext,
        scope: Option<Arc<crate::NativeWorkspaceTurnScope>>,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Arc<NativeSandboxLaunch>, NativeSandboxError> {
        check_capture(deadline, cancellation)?;
        let controller = self
            .controller
            .get()
            .and_then(Weak::upgrade)
            .ok_or(NativeSandboxError::Unavailable)?;
        let policy = controller
            .policy_for_execution(context)
            .map_err(|_| NativeSandboxError::Unavailable)?;
        if (self.workspace_contexts.is_some() && scope.is_none())
            || scope.as_ref().is_some_and(|scope| !scope.is_live())
        {
            return Err(NativeSandboxError::Unavailable);
        }
        let roots = match &scope {
            Some(scope)
                if policy.effective_sandbox_mode() == NativeSandboxMode::Os
                    && cfg!(target_os = "macos") =>
            {
                workspace::roots(scope, deadline, cancellation)?
            }
            Some(_) => Vec::new(),
            None => self.roots.clone(),
        };
        let executable = if policy.effective_sandbox_mode() == NativeSandboxMode::Os {
            self.executable
                .as_ref()
                .map(File::try_clone)
                .transpose()
                .map_err(|_| NativeSandboxError::Unavailable)?
        } else {
            None
        };
        let launch = NativeSandboxLaunch::capture(
            policy.sandbox_mode(),
            policy.mode(),
            roots,
            executable,
            false,
            deadline,
            cancellation,
        )?;
        let launch = match scope {
            Some(scope) => launch.with_shared_workspace_scope(scope, deadline, cancellation)?,
            None => launch,
        };
        Ok(Arc::new(launch))
    }
}

fn check_capture(
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), NativeSandboxError> {
    if cancellation.is_cancelled() {
        Err(NativeSandboxError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(NativeSandboxError::Timeout)
    } else {
        Ok(())
    }
}

mod workspace;

#[cfg(test)]
pub(crate) mod tests;
