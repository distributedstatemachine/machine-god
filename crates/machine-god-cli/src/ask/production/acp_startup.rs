//! Explicit launch capture shared by ordinary CLI and ephemeral ACP composition.

mod factory;
pub(super) use factory::AcpHostFactory;

#[cfg(test)]
mod tests;

use machine_god_core::CancellationToken;
use machine_god_native::{
    AiGatewayCredentialEnvironment, NativePermissionContexts, NativeReferenceHostMcpOptions,
    NativeReferenceHostTerminalOptions, NativeRootSelection, PreparedNativeRoots, TerminalShell,
    TokioWebSearchRuntime,
    mcp::{
        context::NativeMcpContexts, interaction::McpElicitationPresenter,
        management::NativeMcpManagementService,
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
    Ephemeral,
}
impl McpSelection {
    pub(super) fn prepare_management(
        self,
        directory: Option<&Path>,
    ) -> Result<Option<Arc<NativeMcpManagementService>>, ()> {
        match self {
            Self::Profile => super::mcp_startup::prepare(directory),
            Self::Ephemeral => Ok(None),
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
            Self::Ephemeral => {
                if management.is_some() {
                    return Err(());
                }
                let options = NativeReferenceHostMcpOptions::capture_ephemeral_startup(
                    roots,
                    terminal,
                    Arc::new(NativeMcpContexts::new()),
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
