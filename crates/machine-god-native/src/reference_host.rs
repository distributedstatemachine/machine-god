use std::error::Error;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use machine_god_core::Engine;

use crate::workspace::WorkspaceRoot;
use crate::{
    AiGatewayCredentialEnvironment, AiGatewayCredentialSource, AiGatewayHttpTransport,
    AiGatewayProvider, AiGatewayTransport, AskPermissionHandler, FileSessionStore,
    LoadedNativeConfig, NativeProviderKind, NativeTransportKind, PermissionMode,
    PermissionPrompter, discover_ai_gateway_credential,
};

/// Stable stage at which native reference-host composition failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeReferenceHostBuildErrorKind {
    /// The loaded configuration selects a component this host cannot compose.
    UnsupportedSelection,
    /// The workspace root could not be safely retained for both native tools.
    WorkspaceRoot,
    /// The existing session-store root could not be safely retained.
    SessionStore,
    /// AI Gateway credential discovery failed.
    Credential,
    /// The production AI Gateway HTTP transport could not be constructed.
    HttpTransport,
    /// The selected provider could not be constructed.
    Provider,
    /// The provider-neutral engine could not be constructed.
    Engine,
}

/// Fixed, redacted native reference-host composition failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeReferenceHostBuildError {
    kind: NativeReferenceHostBuildErrorKind,
}

impl NativeReferenceHostBuildError {
    /// Returns the stable composition stage that failed.
    #[must_use]
    pub const fn kind(&self) -> NativeReferenceHostBuildErrorKind {
        self.kind
    }

    const fn new(kind: NativeReferenceHostBuildErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeReferenceHostBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeReferenceHostBuildError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeReferenceHostBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeReferenceHostBuildErrorKind::UnsupportedSelection => {
                "native reference-host selection is unsupported"
            }
            NativeReferenceHostBuildErrorKind::WorkspaceRoot => {
                "native reference-host workspace root is unavailable"
            }
            NativeReferenceHostBuildErrorKind::SessionStore => {
                "native reference-host session store is unavailable"
            }
            NativeReferenceHostBuildErrorKind::Credential => {
                "native reference-host credential is unavailable"
            }
            NativeReferenceHostBuildErrorKind::HttpTransport => {
                "native reference-host HTTP transport construction failed"
            }
            NativeReferenceHostBuildErrorKind::Provider => {
                "native reference-host provider construction failed"
            }
            NativeReferenceHostBuildErrorKind::Engine => {
                "native reference-host engine construction failed"
            }
        })
    }
}

impl Error for NativeReferenceHostBuildError {}

/// Fully composed native reference host for the built-in AI Gateway selection.
pub struct NativeReferenceHost {
    engine: Engine,
    loaded_config: LoadedNativeConfig,
    credential_source: Option<AiGatewayCredentialSource>,
}

impl NativeReferenceHost {
    /// Composes the production AI Gateway HTTP reference host from explicit roots.
    ///
    /// The roots must already exist. This function does not create a runtime,
    /// poll the permission prompt, touch session records, or perform network I/O.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or any explicit component cannot be constructed safely.
    pub fn compose_ai_gateway_http(
        loaded_config: LoadedNativeConfig,
        credential_environment: AiGatewayCredentialEnvironment,
        workspace_root: &Path,
        session_root: &Path,
        permission_prompter: Arc<dyn PermissionPrompter>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let (list_files, read_file) = open_workspace_tools(workspace_root)?;
        let session_store = open_session_store(session_root)?;

        let credential = discover_ai_gateway_credential(credential_environment).map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Credential)
        })?;
        let credential_source = credential.source();
        let transport =
            AiGatewayHttpTransport::new(credential.into_bearer_token()).map_err(|_| {
                NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::HttpTransport)
            })?;

        Self::finish_composition(
            loaded_config,
            Arc::new(transport),
            list_files,
            read_file,
            session_store,
            permission_prompter,
            Some(credential_source),
        )
    }

    /// Composes a reference host over an explicitly injected AI Gateway transport.
    ///
    /// This path retains the same configuration, workspace, session-store, and
    /// permission selections as production composition, but performs no
    /// credential discovery or HTTP transport construction.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or any explicit component cannot be constructed safely.
    pub fn compose_with_ai_gateway_transport(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        workspace_root: &Path,
        session_root: &Path,
        permission_prompter: Arc<dyn PermissionPrompter>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let (list_files, read_file) = open_workspace_tools(workspace_root)?;
        let session_store = open_session_store(session_root)?;

        Self::finish_composition(
            loaded_config,
            transport,
            list_files,
            read_file,
            session_store,
            permission_prompter,
            None,
        )
    }

    /// Returns the composed provider-neutral engine.
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Returns the exact loaded native configuration retained by this host.
    #[must_use]
    pub const fn loaded_config(&self) -> &LoadedNativeConfig {
        &self.loaded_config
    }

    /// Returns the production credential source, if credential discovery ran.
    #[must_use]
    pub const fn credential_source(&self) -> Option<AiGatewayCredentialSource> {
        self.credential_source
    }

    /// Consumes this host and returns its provider-neutral engine.
    #[must_use]
    pub fn into_engine(self) -> Engine {
        self.engine
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_composition(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        list_files: crate::ListFilesTool,
        read_file: crate::ReadFileTool,
        session_store: FileSessionStore,
        permission_prompter: Arc<dyn PermissionPrompter>,
        credential_source: Option<AiGatewayCredentialSource>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        let provider = AiGatewayProvider::new(loaded_config.config().model().to_owned(), transport)
            .map_err(|_| {
                NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Provider)
            })?;
        let permission_handler = AskPermissionHandler::shared_prompter(permission_prompter);
        let engine = Engine::builder()
            .provider(provider)
            .session_store(session_store)
            .permission_handler(permission_handler)
            .tool(list_files)
            .tool(read_file)
            .build()
            .map_err(|_| {
                NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Engine)
            })?;

        Ok(Self {
            engine,
            loaded_config,
            credential_source,
        })
    }
}

impl fmt::Debug for NativeReferenceHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeReferenceHost")
            .finish_non_exhaustive()
    }
}

fn validate_selections(
    loaded_config: &LoadedNativeConfig,
) -> Result<(), NativeReferenceHostBuildError> {
    let config = loaded_config.config();
    if config.provider() != NativeProviderKind::VercelAiGateway
        || config.transport() != NativeTransportKind::AiGatewayHttp
        || config.permission_mode() != PermissionMode::Ask
    {
        return Err(NativeReferenceHostBuildError::new(
            NativeReferenceHostBuildErrorKind::UnsupportedSelection,
        ));
    }
    Ok(())
}

fn open_workspace_tools(
    workspace_root: &Path,
) -> Result<(crate::ListFilesTool, crate::ReadFileTool), NativeReferenceHostBuildError> {
    WorkspaceRoot::open(workspace_root)
        .and_then(WorkspaceRoot::into_tools)
        .map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::WorkspaceRoot)
        })
}

fn open_session_store(
    session_root: &Path,
) -> Result<FileSessionStore, NativeReferenceHostBuildError> {
    FileSessionStore::open(session_root).map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::SessionStore)
    })
}
