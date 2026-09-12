//! Concrete MCP composition using the reference host's existing native owners.

#[cfg(feature = "mcp-http")]
mod authentication;
#[cfg(feature = "mcp-http")]
mod startup;

use super::{
    NativeReferenceHost, NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind,
};
use crate::{
    NativeToolResultArchiveAdapter,
    mcp::{
        context::NativeMcpContexts,
        controller::{
            NativeMcpController, NativeMcpControllerOptions, NativeMcpControllerStartupOptions,
        },
        execution::NativeMcpArchivedToolExecutor,
        management::NativeMcpManagementService,
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
    startup: Option<NativeMcpControllerStartupOptions>,
    #[cfg(feature = "mcp-http")]
    authentication: Option<authentication::Options>,
}
impl NativeReferenceHostMcpOptions {
    #[must_use]
    pub fn new(contexts: Arc<NativeMcpContexts>, clock: Arc<dyn NativeMcpRuntimeClock>) -> Self {
        Self {
            contexts,
            clock,
            limits: NativeMcpRuntimeLimits::default(),
            form_responder: None,
            startup: None,
            #[cfg(feature = "mcp-http")]
            authentication: None,
        }
    }

    /// Selects finite ownership bounds, validated by actual runtime composition.
    #[must_use]
    pub fn with_runtime_limits(mut self, limits: NativeMcpRuntimeLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Selects caller-polled profile activation using this host's exact runtime,
    /// management service, fixed registrations and worker scope. No I/O occurs.
    /// Host composition requires management selection and the same clock Arc.
    #[must_use]
    pub fn with_controller_startup(mut self, startup: NativeMcpControllerStartupOptions) -> Self {
        self.startup = Some(startup);
        self
    }

    pub(super) fn validate_controller(
        &self,
        has_management: bool,
    ) -> Result<(), NativeReferenceHostBuildError> {
        if self
            .startup
            .as_ref()
            .is_some_and(|startup| !has_management || !Arc::ptr_eq(&startup.clock, &self.clock))
        {
            return Err(error());
        }
        Ok(())
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
        let executor = NativeMcpArchivedToolExecutor::new(archive.clone()).map_err(|_| error())?;
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
        let features = Arc::new(crate::NativeMcpFeaturesTool::new(
            Arc::downgrade(&runtime),
            archive,
        ));
        Ok(Composition {
            runtime,
            features,
            contexts: self.contexts,
            startup: self.startup,
            #[cfg(feature = "mcp-http")]
            authentication: self.authentication,
            management: None,
        })
    }
}

pub(super) struct Composition {
    pub runtime: Arc<NativeMcpRuntime>,
    pub features: Arc<crate::NativeMcpFeaturesTool>,
    pub contexts: Arc<NativeMcpContexts>,
    startup: Option<NativeMcpControllerStartupOptions>,
    #[cfg(feature = "mcp-http")]
    authentication: Option<authentication::Options>,
    management: Option<Arc<NativeMcpManagementService>>,
}

#[derive(Default)]
pub(super) struct Selection {
    pub options: Option<NativeReferenceHostMcpOptions>,
    pub management: Option<Arc<NativeMcpManagementService>>,
}

pub(super) type OwnedRuntime = (
    Option<Arc<NativeMcpRuntime>>,
    Option<Arc<NativeMcpController>>,
);

pub(super) fn controller(
    composition: Option<Composition>,
    workers: Option<&crate::NativeOwnedWorkerScope>,
    reserved_tool_names: &[ToolName],
) -> Result<OwnedRuntime, NativeReferenceHostBuildError> {
    let Some(composition) = composition else {
        return Ok((None, None));
    };
    let controller = composition
        .startup
        .map(|startup| {
            let management = composition.management.ok_or_else(error)?;
            let workers = workers.ok_or_else(error)?;
            #[cfg(feature = "mcp-http")]
            let stored_authentication = composition
                .authentication
                .map(|selected| selected.compose(&management, &startup, workers))
                .transpose()?;
            NativeMcpController::new(NativeMcpControllerOptions {
                runtime: composition.runtime.clone(),
                management,
                workers: workers.clone(),
                reserved_tool_names: reserved_tool_names.into(),
                startup,
                #[cfg(feature = "mcp-http")]
                stored_authentication,
                max_retained_generations: 4,
            })
            .map(Arc::new)
            .map_err(|_| error())
        })
        .transpose()?;
    if let Some(controller) = &controller {
        composition
            .runtime
            .bind_controller(controller)
            .map_err(|_| error())?;
    }
    Ok((Some(composition.runtime), controller))
}

pub(super) fn features(
    composition: Option<&Composition>,
    fallback: Arc<dyn crate::McpFeatureAuthority>,
) -> Arc<dyn machine_god_core::Tool> {
    match composition {
        Some(composition) => composition.features.clone(),
        None => Arc::new(crate::McpFeaturesTool::shared_authority(fallback)),
    }
}

pub(super) fn select(
    selection: Selection,
    terminal: &super::SelectedTerminalComposition,
    permissions: Option<&super::PermissionComposition>,
    catalog: Arc<dyn crate::McpToolCatalog>,
) -> Result<(Option<Composition>, Arc<dyn crate::McpToolCatalog>), NativeReferenceHostBuildError> {
    let Some(options) = selection.options else {
        return Ok((None, catalog));
    };
    if terminal.resource.is_none() || permissions.is_none() {
        return Err(error());
    }
    options.validate_controller(selection.management.is_some())?;
    let mut composition = options.compose(terminal.archive.clone().ok_or_else(error)?)?;
    composition.management = selection.management;
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
    pub controller: Option<Arc<NativeMcpController>>,
    pub _terminal: NativeTerminalHostResource,
}
impl Drop for HostResource {
    fn drop(&mut self) {
        if let Some(controller) = &self.controller {
            controller.close();
        }
        self.mcp.close();
    }
}

impl NativeReferenceHost {
    /// Returns the host's exact optional profile credential service. This is an
    /// inert accessor, not authentication, credential read or browser consent.
    #[cfg(feature = "mcp-http")]
    #[must_use]
    pub fn mcp_authentication(&self) -> Option<Arc<crate::mcp::auth::NativeMcpAuthService>> {
        self.mcp_controller.as_ref()?.authentication_service()
    }
    /// Returns this engine owner's exact optional controller without activation.
    /// Retaining this accessor does not prevent engine-drop invalidation.
    #[must_use]
    pub fn mcp_controller(&self) -> Option<Arc<NativeMcpController>> {
        self.mcp_controller.clone()
    }
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
        if let Some(controller) = &self.mcp_controller {
            controller.close();
        }
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
