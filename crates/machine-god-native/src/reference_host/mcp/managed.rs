//! Inert parent selection stays with the outer host, never the shared engine.
use super::{
    Arc, CancellationToken, NativeMcpContexts, NativeMcpController, NativeMcpEphemeralOwner,
    NativeMcpManagementService, NativeMcpRuntime, NativeMcpRuntimeClock,
    NativeReferenceHostBuildError, NativeReferenceHostMcpOptions, NativeToolResultArchiveAdapter,
    ToolName, controller, error,
};
use crate::reference_host::permissions::SharedPermissionPreparation;
#[cfg(test)]
mod tests;

struct Inputs {
    management: Option<Arc<NativeMcpManagementService>>,
    archive: Arc<NativeToolResultArchiveAdapter>,
    url_launcher: Option<crate::mcp::browser_launcher::NativeMcpBrowserLauncher>,
}
struct Seed {
    options: NativeReferenceHostMcpOptions,
    inputs: Arc<Inputs>,
}
/// Configured-server authority only, with no parent's ephemeral selection.
pub(in crate::reference_host) struct ManagedMcpSeed(Seed);
/// Explicit parent-only selection, retained outside shared host services.
pub(in crate::reference_host) struct ManagedParentMcpSeed(Seed);

pub(in crate::reference_host) struct ManagedMcpInstance {
    pub runtime: Arc<NativeMcpRuntime>,
    pub controller: Option<Arc<NativeMcpController>>,
    pub ephemeral: Option<Arc<NativeMcpEphemeralOwner>>,
    pub contexts: Arc<NativeMcpContexts>,
    pub permissions: crate::managed::mcp::NativePrincipalMcpPermissions,
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
}

pub(in crate::reference_host) fn seeds(
    options: NativeReferenceHostMcpOptions,
    management: Option<Arc<NativeMcpManagementService>>,
    archive: Arc<NativeToolResultArchiveAdapter>,
    url_launcher: Option<crate::mcp::browser_launcher::NativeMcpBrowserLauncher>,
) -> Result<(Arc<ManagedMcpSeed>, ManagedParentMcpSeed), NativeReferenceHostBuildError> {
    options.limits.validate().map_err(|_| error())?;
    let inputs = Arc::new(Inputs {
        management,
        archive,
        url_launcher,
    });
    let mut child = options.clone();
    child.contexts = Arc::new(NativeMcpContexts::new());
    child.ephemeral = None;
    if let Some(startup) = &mut child.startup {
        startup.owner_cancellation = CancellationToken::new();
    }
    Ok((
        Arc::new(ManagedMcpSeed(Seed {
            options: child,
            inputs: inputs.clone(),
        })),
        ManagedParentMcpSeed(Seed { options, inputs }),
    ))
}

impl Seed {
    fn compose(
        &self,
        workers: &crate::NativeOwnedWorkerScope,
        reserved_tool_names: &[ToolName],
        permissions: &SharedPermissionPreparation,
    ) -> Result<ManagedMcpInstance, NativeReferenceHostBuildError> {
        let contexts = Arc::new(NativeMcpContexts::new());
        let mut options = self.options.clone();
        options.contexts = contexts.clone();
        if let Some(startup) = &mut options.startup {
            startup.owner_cancellation = CancellationToken::new();
        }
        if let Some(startup) = &mut options.ephemeral {
            startup.owner_cancellation = CancellationToken::new();
        }
        let mut composition = options.compose(
            self.inputs.archive.clone(),
            self.inputs.url_launcher.clone(),
        )?;
        composition.management.clone_from(&self.inputs.management);
        let clock = composition.clock.clone();
        let paired = crate::managed::mcp::NativePrincipalMcpPermissions::new(
            &composition.runtime,
            permissions.mcp_inputs(contexts.clone()),
        )
        .map_err(|_| error())?;
        let (runtime, controller, ephemeral) =
            controller(Some(composition), Some(workers), reserved_tool_names)?;
        Ok(ManagedMcpInstance {
            runtime: runtime.ok_or_else(error)?,
            controller,
            ephemeral,
            contexts,
            permissions: paired,
            clock,
        })
    }
}

impl ManagedMcpSeed {
    pub(in crate::reference_host) fn compose(
        &self,
        workers: &crate::NativeOwnedWorkerScope,
        reserved_tool_names: &[ToolName],
        permissions: &SharedPermissionPreparation,
    ) -> Result<ManagedMcpInstance, NativeReferenceHostBuildError> {
        self.0.compose(workers, reserved_tool_names, permissions)
    }
}
impl ManagedParentMcpSeed {
    pub(in crate::reference_host) fn is_ephemeral(&self) -> bool {
        self.0.options.ephemeral.is_some()
    }

    /// Reuse the captured workspace/helper/environment and host services while
    /// selecting explicitly supplied transport authority for a new parent.
    /// Never mutate the original parent or the configured-only child seed.
    pub(in crate::reference_host) fn select_ephemeral_network(
        &self,
        #[cfg(feature = "mcp-http")] network: Option<Arc<crate::mcp::network::NativeMcpNetwork>>,
    ) -> Result<Arc<Self>, NativeReferenceHostBuildError> {
        self.0
            .options
            .validate_controller(self.0.inputs.management.is_some())?;
        let mut options = self.0.options.clone();
        let startup = options.ephemeral.as_mut().ok_or_else(error)?;
        startup.owner_cancellation = CancellationToken::new();
        #[cfg(feature = "mcp-http")]
        {
            startup.network = network;
        }
        Ok(Arc::new(Self(Seed {
            options,
            inputs: self.0.inputs.clone(),
        })))
    }

    pub(in crate::reference_host) fn compose(
        &self,
        workers: &crate::NativeOwnedWorkerScope,
        reserved_tool_names: &[ToolName],
        permissions: &SharedPermissionPreparation,
    ) -> Result<ManagedMcpInstance, NativeReferenceHostBuildError> {
        self.0.compose(workers, reserved_tool_names, permissions)
    }
}
