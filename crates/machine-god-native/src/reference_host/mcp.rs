//! Concrete MCP composition using the reference host's existing native owners.

use super::{
    NativeReferenceHost, NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind,
};
use crate::{
    NativeToolResultArchiveAdapter,
    mcp::{
        context::NativeMcpContexts,
        execution::NativeMcpArchivedToolExecutor,
        runtime::{
            NativeMcpPeerCompletion, NativeMcpRuntime, NativeMcpRuntimeClock,
            NativeMcpRuntimeError, NativeMcpRuntimeLimits,
        },
    },
    terminal_host::NativeTerminalHostResource,
};
use machine_god_core::{CancellationToken, ToolName};
use std::{fmt, sync::Arc, time::Instant};

/// Inert exact-context and clock selection for native MCP composition.
/// The archive, worker scope, builtin authority and reviewer are supplied only
/// by the actual composed reference host, never inferred or duplicated here.
#[derive(Clone)]
pub struct NativeReferenceHostMcpOptions {
    pub(super) contexts: Arc<NativeMcpContexts>,
    clock: Arc<dyn NativeMcpRuntimeClock>,
    limits: NativeMcpRuntimeLimits,
    form_responder: Option<Arc<dyn crate::mcp::interaction::McpElicitationPresenter>>,
}
impl NativeReferenceHostMcpOptions {
    #[must_use]
    pub fn new(contexts: Arc<NativeMcpContexts>, clock: Arc<dyn NativeMcpRuntimeClock>) -> Self {
        Self {
            contexts,
            clock,
            limits: NativeMcpRuntimeLimits::default(),
            form_responder: None,
        }
    }

    /// Selects finite ownership bounds, validated by actual runtime composition.
    #[must_use]
    pub fn with_runtime_limits(mut self, limits: NativeMcpRuntimeLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Selects the actual host-owned form endpoint, without prompting or I/O.
    #[must_use]
    pub fn with_form_responder(
        mut self,
        presenter: Arc<dyn crate::mcp::interaction::McpElicitationPresenter>,
    ) -> Self {
        self.form_responder = Some(presenter);
        self
    }

    pub(super) fn compose(
        self,
        archive: Arc<NativeToolResultArchiveAdapter>,
    ) -> Result<Composition, NativeReferenceHostBuildError> {
        let executor = NativeMcpArchivedToolExecutor::new(archive).map_err(|_| error())?;
        let executor = Arc::new(match self.form_responder {
            Some(presenter) => executor.with_form_responder(presenter),
            None => executor,
        });
        let policy = executor.execution_policy();
        let runtime = NativeMcpRuntime::new(
            self.contexts.clone(),
            self.clock,
            executor,
            policy,
            self.limits,
        )
        .map(Arc::new)
        .map_err(|_| error())?;
        Ok(Composition {
            runtime,
            contexts: self.contexts,
        })
    }
}

pub(super) struct Composition {
    pub runtime: Arc<NativeMcpRuntime>,
    pub contexts: Arc<NativeMcpContexts>,
}

pub(super) fn select(
    options: Option<NativeReferenceHostMcpOptions>,
    terminal: &super::SelectedTerminalComposition,
    permissions: Option<&super::PermissionComposition>,
    catalog: Arc<dyn crate::McpToolCatalog>,
) -> Result<(Option<Composition>, Arc<dyn crate::McpToolCatalog>), NativeReferenceHostBuildError> {
    let Some(options) = options else {
        return Ok((None, catalog));
    };
    if terminal.resource.is_none() || permissions.is_none() {
        return Err(error());
    }
    let composition = options.compose(terminal.archive.clone().ok_or_else(error)?)?;
    let catalog = composition.runtime.clone();
    Ok((Some(composition), catalog))
}
impl fmt::Debug for NativeReferenceHostMcpOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeReferenceHostMcpOptions { <redacted> }")
    }
}

pub(super) fn error() -> NativeReferenceHostBuildError {
    NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::McpConfig)
}

/// Retained only by the engine's actual host-resource lease, never its tools.
/// Field destruction follows Drop: MCP invalidation precedes terminal shutdown.
pub(super) struct HostResource {
    pub mcp: Arc<NativeMcpRuntime>,
    pub _terminal: NativeTerminalHostResource,
}
impl Drop for HostResource {
    fn drop(&mut self) {
        self.mcp.close();
    }
}

impl NativeReferenceHost {
    /// Returns the exact native runtime, without connecting or publishing peers.
    #[must_use]
    pub fn mcp_runtime(&self) -> Option<Arc<NativeMcpRuntime>> {
        self.mcp_runtime.clone()
    }

    /// Exact fixed engine registrations reserved from dynamic MCP naming.
    /// Reading these names does not clone schemas or query an extension catalog.
    #[must_use]
    pub fn reserved_tool_names(&self) -> &[ToolName] {
        &self.reserved_tool_names
    }

    /// Irreversibly invalidates native MCP before host worker shutdown.
    /// This is not a socket-close, child-reap or remote DELETE completion receipt.
    pub fn close_mcp(&self) {
        if let Some(runtime) = &self.mcp_runtime {
            runtime.close();
        }
    }

    /// Drives local retired-peer closure while this host still owns its workers.
    /// Construction/unpolled futures are inert; returned observations must still
    /// be checked for actual worker/reap completion. This never issues DELETE.
    /// # Errors
    /// Preserves the runtime's cancellation/deadline/ownership error and retains
    /// undrained peers for a later explicitly owned cleanup attempt.
    pub async fn drain_mcp(
        &self,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<Vec<NativeMcpPeerCompletion>, NativeMcpRuntimeError> {
        match &self.mcp_runtime {
            Some(runtime) => runtime.drain_retired(deadline, cancellation).await,
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests;
