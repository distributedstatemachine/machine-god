//! Explicit launch capture shared by ordinary CLI and ephemeral ACP composition.

mod factory;
mod network;
pub(super) use factory::AcpHostFactory;
pub(super) use network::GatewayNetwork;

#[cfg(test)]
mod tests;

use machine_god_core::CancellationToken;
use machine_god_native::{
    AiGatewayCredentialEnvironment, NativePermissionContexts, NativeReferenceHostMcpOptions,
    NativeReferenceHostTerminalOptions, NativeRootSelection, PreparedNativeRoots, TerminalShell,
    TokioWebSearchRuntime,
    mcp::{
        context::NativeMcpContexts, ephemeral::NativeMcpNetworkRequirement,
        interaction::McpElicitationPresenter, management::NativeMcpManagementService,
    },
};
use std::{
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) trait HostRuntime {
    fn block_on<F: Future>(&self, future: F) -> F::Output;
}
impl HostRuntime for TokioWebSearchRuntime {
    fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.block_on(future)
    }
}
impl HostRuntime for tokio::runtime::Handle {
    fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.block_on(future)
    }
}

#[derive(Clone)]
pub(super) struct TerminalCapture {
    helper: PathBuf,
    shell: PathBuf,
}
impl TerminalCapture {
    pub(super) fn capture() -> Result<Self, ()> {
        let shell = TerminalShell::for_current_user(None, None).map_err(|_| ())?;
        Ok(Self {
            helper: std::env::current_exe().map_err(|_| ())?,
            shell: shell.program().to_owned(),
        })
    }
    pub(super) fn configure(
        &self,
        workspace: &Path,
        environment: Vec<(OsString, OsString)>,
    ) -> Result<NativeReferenceHostTerminalOptions, ()> {
        super::terminal_options_from_capture(
            workspace,
            environment,
            self.helper.clone(),
            self.shell.clone(),
        )
    }
}

#[derive(Clone, Copy)]
pub(super) enum McpSelection {
    Profile,
    Ephemeral(NativeMcpNetworkRequirement),
}
impl McpSelection {
    pub(super) fn capture_workspace_identity(
        self,
        roots: &PreparedNativeRoots,
        authority: &machine_god_native::NativeWorkspaceAuthority,
    ) -> Result<Option<machine_god_native::acp::selection::NativeAcpWorkspaceIdentity>, ()> {
        match self {
            Self::Ephemeral(_) => {
                machine_god_native::acp::selection::NativeAcpWorkspaceIdentity::capture(
                    roots, authority,
                )
                .map(Some)
                .map_err(|_| ())
            }
            Self::Profile => Ok(None),
        }
    }

    pub(super) fn prepare_management(
        self,
        directory: Option<&Path>,
    ) -> Result<Option<Arc<NativeMcpManagementService>>, ()> {
        match self {
            Self::Profile => super::mcp_startup::prepare(directory),
            Self::Ephemeral(_) => Ok(None),
        }
    }
    pub(super) fn prepare_runtime(
        self,
        roots: &PreparedNativeRoots,
        terminal: &NativeReferenceHostTerminalOptions,
        management: Option<&NativeMcpManagementService>,
        presenter: Option<Arc<dyn McpElicitationPresenter>>,
    ) -> Result<Option<NativeReferenceHostMcpOptions>, ()> {
        match self {
            Self::Profile => {
                super::mcp_startup::prepare_runtime(roots, terminal, management, presenter)
            }
            Self::Ephemeral(network) => {
                if management.is_some() {
                    return Err(());
                }
                let options = NativeReferenceHostMcpOptions::capture_ephemeral_startup(
                    roots,
                    terminal,
                    Arc::new(NativeMcpContexts::new()),
                    network,
                )
                .map_err(|_| ())?;
                Ok(Some(match presenter {
                    Some(presenter) => options.with_form_responder(presenter),
                    None => options,
                }))
            }
        }
    }
}

pub(super) struct CapturedHostInputs {
    pub(super) environment: Vec<(OsString, OsString)>,
    pub(super) roots: NativeRootSelection,
    pub(super) terminal: TerminalCapture,
    pub(super) mcp: McpSelection,
    pub(super) permission_contexts: Arc<NativePermissionContexts>,
    pub(super) cancellation: CancellationToken,
    pub(super) network: GatewayNetwork,
}

/// Bounded launch capture shared by the production entry and owned-I/O tests.
/// This contains authority inputs, never a prepared host or selected session.
pub(super) struct CapturedAcpLaunch {
    environment: Arc<[(OsString, OsString)]>,
    terminal: TerminalCapture,
    network: GatewayNetwork,
}
impl CapturedAcpLaunch {
    pub(super) fn capture() -> Result<Self, ()> {
        let environment = Self::environment(std::env::vars_os())?;
        Ok(Self {
            environment,
            terminal: TerminalCapture::capture()?,
            network: GatewayNetwork::default(),
        })
    }

    fn environment(
        values: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Arc<[(OsString, OsString)]>, ()> {
        let mut environment = Vec::new();
        let mut bytes = 0usize;
        for (name, value) in values {
            bytes = bytes
                .checked_add(name.len())
                .and_then(|n| n.checked_add(value.len()))
                .ok_or(())?;
            if bytes > 1024 * 1024 || environment.len() == 4096 {
                return Err(());
            }
            environment.push((name, value));
        }
        Ok(environment.into())
    }

    #[cfg(test)]
    pub(super) fn loopback(
        environment: Vec<(OsString, OsString)>,
        helper: PathBuf,
        shell: PathBuf,
        gateway: std::net::SocketAddr,
    ) -> Result<Self, ()> {
        Ok(Self {
            environment: Self::environment(environment)?,
            terminal: TerminalCapture { helper, shell },
            network: GatewayNetwork::loopback(gateway)?,
        })
    }
}
pub(super) fn check_cancelled(cancel: &CancellationToken) -> Result<(), ()> {
    if cancel.is_cancelled() {
        Err(())
    } else {
        Ok(())
    }
}
pub(super) fn credential_environment(
    values: &[(OsString, OsString)],
) -> AiGatewayCredentialEnvironment {
    let selected = |name: &str| {
        values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    AiGatewayCredentialEnvironment::new(
        selected("VERCEL_OIDC_TOKEN"),
        selected("AI_GATEWAY_API_KEY"),
    )
}
