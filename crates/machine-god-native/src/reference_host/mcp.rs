//! Concrete MCP composition using the reference host's existing native owners.

#[cfg(feature = "mcp-http")]
mod authentication;
mod ephemeral;
pub use ephemeral::NativeReferenceHostMcpEphemeralStartupOptions;
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
        ephemeral::{NativeMcpEphemeralError, NativeMcpEphemeralOwner},
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
    ephemeral: Option<NativeReferenceHostMcpEphemeralStartupOptions>,
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
            ephemeral: None,
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
        if let Some(ephemeral) = &self.ephemeral {
            if has_management
                || self.startup.is_some()
                || !Arc::ptr_eq(&ephemeral.clock, &self.clock)
            {
                return Err(error());
            }
            #[cfg(feature = "mcp-http")]
            if self.authentication.is_some() {
                return Err(error());
            }
        }
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
        url_launcher: Option<crate::mcp::browser_launcher::NativeMcpBrowserLauncher>,
    ) -> Result<Composition, NativeReferenceHostBuildError> {
        let executor = NativeMcpArchivedToolExecutor::new(archive.clone()).map_err(|_| error())?;
        let executor = match self.form_responder.clone() {
            Some(presenter) => executor.with_form_responder(presenter),
            None => executor,
        };
        let executor = Arc::new(match url_launcher.clone() {
            Some(launcher) => executor.with_url_launcher(launcher),
            None => executor,
        });
        let policy = executor.execution_policy();
        let runtime = NativeMcpRuntime::new(
            self.contexts.clone(),
            self.clock.clone(),
            executor,
            policy,
            self.limits,
        )
        .map(|runtime| Arc::new(runtime.with_feature_input(self.form_responder, url_launcher)))
        .map_err(|_| error())?;
        let features = Arc::new(crate::NativeMcpFeaturesTool::new(
            Arc::downgrade(&runtime),
            archive,
        ));
        Ok(Composition {
            clock: self.clock,
            runtime,
            features,
            contexts: self.contexts,
            startup: self.startup,
            ephemeral: self.ephemeral,
            #[cfg(feature = "mcp-http")]
            authentication: self.authentication,
            management: None,
        })
    }
}

pub(super) struct Composition {
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
    pub runtime: Arc<NativeMcpRuntime>,
    pub features: Arc<crate::NativeMcpFeaturesTool>,
    pub contexts: Arc<NativeMcpContexts>,
    startup: Option<NativeMcpControllerStartupOptions>,
    ephemeral: Option<NativeReferenceHostMcpEphemeralStartupOptions>,
    #[cfg(feature = "mcp-http")]
    authentication: Option<authentication::Options>,
    management: Option<Arc<NativeMcpManagementService>>,
}

#[derive(Default)]
pub(super) struct Selection {
    pub options: Option<NativeReferenceHostMcpOptions>,
    pub management: Option<Arc<NativeMcpManagementService>>,
}

/// Explicit configuration seed only. It never retains a parent's live runtime,
/// contexts, connection, ephemeral owner or permission bundle.
pub(super) struct ManagedMcpSeed {
    options: NativeReferenceHostMcpOptions,
    management: Option<Arc<NativeMcpManagementService>>,
    archive: Arc<NativeToolResultArchiveAdapter>,
    url_launcher: Option<crate::mcp::browser_launcher::NativeMcpBrowserLauncher>,
}

pub(super) struct ManagedMcpInstance {
    pub runtime: Arc<NativeMcpRuntime>,
    pub controller: Option<Arc<NativeMcpController>>,
    pub contexts: Arc<NativeMcpContexts>,
    pub permissions: crate::managed::mcp::NativePrincipalMcpPermissions,
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
}

impl ManagedMcpSeed {
    pub(super) fn compose(
        &self,
        workers: &crate::NativeOwnedWorkerScope,
        reserved_tool_names: &[ToolName],
        permissions: &super::permissions::SharedPermissionPreparation,
    ) -> Result<ManagedMcpInstance, NativeReferenceHostBuildError> {
        let contexts = Arc::new(NativeMcpContexts::new());
        let mut options = self.options.clone();
        options.contexts = contexts.clone();
        if let Some(startup) = &mut options.startup {
            startup.owner_cancellation = CancellationToken::new();
        }
        // Request-scoped server authority belongs to the original principal.
        // It is not a configured-server seed for descendants.
        options.ephemeral = None;
        let mut composition = options.compose(self.archive.clone(), self.url_launcher.clone())?;
        composition.management.clone_from(&self.management);
        let clock = composition.clock.clone();
        let paired = crate::managed::mcp::NativePrincipalMcpPermissions::new(
            &composition.runtime,
            permissions.mcp_inputs(contexts.clone()),
        )
        .map_err(|_| error())?;
        let (runtime, controller, ephemeral) =
            controller(Some(composition), Some(workers), reserved_tool_names)?;
        debug_assert!(ephemeral.is_none());
        Ok(ManagedMcpInstance {
            runtime: runtime.ok_or_else(error)?,
            controller,
            contexts,
            permissions: paired,
            clock,
        })
    }
}

pub(super) type OwnedRuntime = (
    Option<Arc<NativeMcpRuntime>>,
    Option<Arc<NativeMcpController>>,
    Option<Arc<NativeMcpEphemeralOwner>>,
);

pub(super) fn controller(
    composition: Option<Composition>,
    workers: Option<&crate::NativeOwnedWorkerScope>,
    reserved_tool_names: &[ToolName],
) -> Result<OwnedRuntime, NativeReferenceHostBuildError> {
    let Some(composition) = composition else {
        return Ok((None, None, None));
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
    let ephemeral = composition
        .ephemeral
        .map(|startup| {
            startup.compose(
                composition.runtime.clone(),
                workers.ok_or_else(error)?,
                reserved_tool_names,
            )
        })
        .transpose()?;
    Ok((Some(composition.runtime), controller, ephemeral))
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

pub(super) struct Selected {
    pub composition: Option<Composition>,
    pub catalog: Arc<dyn crate::McpToolCatalog>,
    pub managed_seed: Option<Arc<ManagedMcpSeed>>,
}

pub(super) fn select(
    selection: Selection,
    terminal: &super::SelectedTerminalComposition,
    permissions: Option<&super::PermissionComposition>,
    catalog: Arc<dyn crate::McpToolCatalog>,
    url_launcher: Option<crate::mcp::browser_launcher::NativeMcpBrowserLauncher>,
) -> Result<Selected, NativeReferenceHostBuildError> {
    let Some(options) = selection.options else {
        return Ok(Selected {
            composition: None,
            catalog,
            managed_seed: None,
        });
    };
    if terminal.resource.is_none() || permissions.is_none() {
        return Err(error());
    }
    options.validate_controller(selection.management.is_some())?;
    let archive = terminal.archive.clone().ok_or_else(error)?;
    let mut child_options = options.clone();
    // Retain no parent context allocation, even before the first child exists.
    child_options.contexts = Arc::new(NativeMcpContexts::new());
    child_options.ephemeral = None;
    if let Some(startup) = &mut child_options.startup {
        startup.owner_cancellation = CancellationToken::new();
    }
    let seed = Arc::new(ManagedMcpSeed {
        options: child_options,
        management: selection.management.clone(),
        archive: archive.clone(),
        url_launcher: url_launcher.clone(),
    });
    let mut composition = options.compose(archive, url_launcher)?;
    composition.management = selection.management;
    let catalog = composition.runtime.clone();
    Ok(Selected {
        composition: Some(composition),
        catalog,
        managed_seed: Some(seed),
    })
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
    pub ephemeral: Option<Arc<NativeMcpEphemeralOwner>>,
    pub _terminal: NativeTerminalHostResource,
}
impl Drop for HostResource {
    fn drop(&mut self) {
        if let Some(controller) = &self.controller {
            controller.close();
        }
        if let Some(owner) = &self.ephemeral {
            owner.close();
        }
        self.mcp.close();
    }
}

impl NativeReferenceHost {
    /// Observes this host's explicitly selected MCP clock. This accessor is
    /// effectful (unlike host construction), and never substitutes ambient time.
    /// # Errors
    /// Missing MCP selection or an unrepresentable deadline.
    pub fn mcp_deadline_after(
        &self,
        duration: std::time::Duration,
    ) -> Result<Instant, NativeMcpEphemeralError> {
        self.mcp_clock
            .as_ref()
            .ok_or(NativeMcpEphemeralError::Unavailable)?
            .now()
            .checked_add(duration)
            .ok_or(NativeMcpEphemeralError::Limit)
    }

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
    /// Returns the exact session-owned ephemeral selection, never a profile
    /// controller. Retaining it cannot prevent engine-drop invalidation.
    #[must_use]
    pub fn mcp_ephemeral_owner(&self) -> Option<Arc<NativeMcpEphemeralOwner>> {
        self.mcp_ephemeral.clone()
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
        if let Some(owner) = &self.mcp_ephemeral {
            owner.close();
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

    /// Settles all ephemeral startup and publication custody before the caller
    /// shuts down this host's worker scope. The independent cleanup token must
    /// not be the cancelled session-owner token. An unpolled future is inert.
    /// # Errors
    /// Retains custody on cancellation, timeout or overlapping settlement.
    pub async fn settle_mcp_ephemeral(
        &self,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<(), NativeMcpEphemeralError> {
        match &self.mcp_ephemeral {
            Some(owner) => owner.settle(cancellation, deadline).await,
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
