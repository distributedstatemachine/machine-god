use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use machine_god_core::{
    BoxFuture, CancellationToken, Engine, EngineLimits, NetworkTarget, SessionIncarnationId,
    SessionStore, SubagentAuthority, SubagentAuthorityError, SubagentAuthorityErrorKind,
    SubagentOutcome, SubagentRequest, SubagentTool, Tool, ToolName,
};
use rustix::fd::OwnedFd;

use crate::background_inspection::NativeBackgroundRecordInspector;
use crate::background_process::ValidatedBackgroundEnvironment;
use crate::background_supervisor::LazyProductionBackgroundStarter;
use crate::terminal_host::{NativeTerminalHost, NativeTerminalHostResource};
use crate::terminal_host_authority::{TerminalHostAccountShell, TerminalHostAuthorityInputs};
use crate::workspace::{WorkspaceRoot, WorkspaceTools};
use crate::{
    AiGatewayCredentialEnvironment, AiGatewayCredentialSource, AiGatewayHttpTransport,
    AiGatewayLimits, AiGatewayProvider, AiGatewayToolInputLimits, AiGatewayTransport,
    AiGatewayVisionTransport, AiGatewayWebSearchTransport, AskPermissionHandler,
    AskUserQuestionTool, FileSessionStore, LoadedNativeConfig, McpFeatureAuthority,
    McpFeatureError, McpFeatureErrorKind, McpFeaturePayload, McpFeatureRequest, McpFeaturesTool,
    McpSearchToolsTool, McpSelectTool, McpToolCatalog, McpToolCatalogError, McpToolCatalogSnapshot,
    MemoryTool, NativeCredentialSourceKind, NativeProviderKind, NativeSessionLifecycle,
    NativeToolResultArchiveAdapter, NativeTransportKind, PermissionMode, PermissionPrompter,
    PreparedNativeRoots, QuestionPrompter, ReadToolResultTool, TerminalBackgroundCatalog,
    TerminalBackgroundInspector, TerminalBackgroundOutputReader, TerminalBackgroundSignaler,
    TerminalBackgroundStarter, TerminalBackgroundWaitDelay, TerminalBackgroundWaitDelayError,
    TerminalBackgroundWriter, TerminalTool, ToolResultArchive, VisionDeadline, VisionLimits,
    VisionTool, VisionTransportError, VisionTransportErrorKind, WebFetchTool, WebSearchDeadline,
    WebSearchLimits, WebSearchTool, WebSearchTransportErrorKind, discover_ai_gateway_credential,
};

/// Stable stage at which native reference-host composition failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeReferenceHostBuildErrorKind {
    /// The loaded configuration selects a component this host cannot compose.
    UnsupportedSelection,
    /// The workspace root could not be safely retained for native tools.
    WorkspaceRoot,
    /// The existing session-store root could not be safely retained.
    SessionStore,
    /// The memory tool could not retain the exact session-store root identity.
    Memory,
    /// AI Gateway credential discovery failed.
    Credential,
    /// The production AI Gateway HTTP transport could not be constructed.
    HttpTransport,
    /// The production bounded web-fetch transport could not be constructed.
    WebFetchTransport,
    /// The production bounded web-search transport could not be constructed.
    WebSearchTransport,
    /// The private Gateway vision worker could not be constructed.
    VisionTransport,
    /// The bounded vision tool could not retain its configured authorities.
    VisionConfig,
    /// The bounded terminal tool could not snapshot its process environment.
    TerminalConfig,
    /// The bounded background supervisor could not be composed.
    BackgroundConfig,
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
            NativeReferenceHostBuildErrorKind::Memory => {
                "native reference-host memory construction failed"
            }
            NativeReferenceHostBuildErrorKind::Credential => {
                "native reference-host credential is unavailable"
            }
            NativeReferenceHostBuildErrorKind::HttpTransport => {
                "native reference-host HTTP transport construction failed"
            }
            NativeReferenceHostBuildErrorKind::WebFetchTransport => {
                "native reference-host web-fetch transport construction failed"
            }
            NativeReferenceHostBuildErrorKind::WebSearchTransport => {
                "native reference-host web-search transport construction failed"
            }
            NativeReferenceHostBuildErrorKind::VisionTransport => {
                "native reference-host vision transport construction failed"
            }
            NativeReferenceHostBuildErrorKind::VisionConfig => {
                "native reference-host vision construction failed"
            }
            NativeReferenceHostBuildErrorKind::TerminalConfig => {
                "native reference-host terminal construction failed"
            }
            NativeReferenceHostBuildErrorKind::BackgroundConfig => {
                "native reference-host background construction failed"
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

/// Explicit, frozen terminal launch selections for native host composition.
///
/// The helper must be a trusted machine-god CLI executable implementing its
/// private terminal helper modes. No executable is inferred from `current_exe`,
/// `PATH`, or an embedding application's test runner. Construction validates
/// bounded data only: it neither opens nor executes the selected programs,
/// reads the process environment, nor queries the account database.
///
/// This value does not construct a host or change an existing host's tool
/// catalog. It carries no process, registry, or runtime lifetime authority.
#[derive(Clone)]
pub struct NativeReferenceHostTerminalOptions {
    pub(crate) helper_program: PathBuf,
    pub(crate) tmux_program: Option<PathBuf>,
    pub(crate) account_shell: Option<PathBuf>,
    pub(crate) environment: ValidatedBackgroundEnvironment,
}

impl NativeReferenceHostTerminalOptions {
    /// Freezes explicitly supplied helper, account-shell, and environment data.
    ///
    /// `account_shell` is the trusted account-database selection, not `SHELL`.
    /// `None` leaves login-shell selection unavailable; it does not trigger
    /// discovery. A request can still select an explicit supported shell.
    /// Unsupported account shells retain the existing platform fallback rules
    /// when a later request resolves its shell.
    ///
    /// # Errors
    /// Returns only the redacted terminal-configuration stage for invalid or
    /// oversized paths, malformed environment entries, duplicate keys, or an
    /// environment exceeding the existing terminal transport limits.
    pub fn new(
        helper_program: PathBuf,
        account_shell: Option<PathBuf>,
        environment: Vec<(OsString, OsString)>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_terminal_program(&helper_program)?;
        if let Some(account_shell) = &account_shell {
            crate::TerminalShell::from_account_shell(Some(account_shell), None, None)
                .map_err(|_| terminal_options_error())?;
        }
        let environment = ValidatedBackgroundEnvironment::new(environment)
            .map_err(|_| terminal_options_error())?;
        Ok(Self {
            helper_program,
            tmux_program: None,
            account_shell,
            environment,
        })
    }

    /// Selects a trusted tmux executable without searching for or probing it.
    ///
    /// Omitting this selection leaves the tmux backend unavailable. Version
    /// checks and native launch remain the explicitly owned worker's work.
    ///
    /// # Errors
    /// Returns the redacted terminal-configuration stage for an invalid path.
    pub fn with_tmux(mut self, program: PathBuf) -> Result<Self, NativeReferenceHostBuildError> {
        validate_terminal_program(&program)?;
        self.tmux_program = Some(program);
        Ok(self)
    }

    /// Returns the exact caller-selected private-helper executable.
    #[must_use]
    pub fn helper_program(&self) -> &Path {
        &self.helper_program
    }

    /// Returns the optional caller-selected tmux executable.
    #[must_use]
    pub fn tmux_program(&self) -> Option<&Path> {
        self.tmux_program.as_deref()
    }

    /// Returns the frozen account-shell selection, without resolving a request.
    #[must_use]
    pub fn account_shell(&self) -> Option<&Path> {
        self.account_shell.as_deref()
    }

    /// Borrows the exact validated caller-supplied environment snapshot.
    ///
    /// Unlike this explicit accessor, debug and error output never expose it.
    #[must_use]
    pub fn environment(&self) -> &[(OsString, OsString)] {
        self.environment.entries()
    }
}

impl fmt::Debug for NativeReferenceHostTerminalOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeReferenceHostTerminalOptions")
            .finish_non_exhaustive()
    }
}

struct TerminalCompositionSelection {
    options: NativeReferenceHostTerminalOptions,
    state_path: PathBuf,
}

fn terminal_options_error() -> NativeReferenceHostBuildError {
    NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::TerminalConfig)
}

fn validate_terminal_program(program: &Path) -> Result<(), NativeReferenceHostBuildError> {
    let text = program.to_str().ok_or_else(terminal_options_error)?;
    if !program.is_absolute()
        || text.len() > crate::terminal_helper::MAX_PROGRAM_BYTES
        || text.contains('\0')
        || program.file_name().is_none()
    {
        return Err(terminal_options_error());
    }
    Ok(())
}

/// Fully composed native reference host for the built-in AI Gateway selection.
pub struct NativeReferenceHost {
    engine: Engine,
    session_store: Arc<FileSessionStore>,
    session_lifecycle: NativeSessionLifecycle,
    loaded_config: LoadedNativeConfig,
    credential_source: Option<AiGatewayCredentialSource>,
}

impl NativeReferenceHost {
    /// Composes the production AI Gateway HTTP reference host from explicit roots.
    ///
    /// The roots must already exist. This function does not create a runtime,
    /// poll the permission prompt, touch session records, or perform network I/O.
    /// The trusted host must select disjoint workspace and session roots; this
    /// constructor does not compare their identity or ancestor relationships.
    /// The production transport is shared by language-model, web-search, and
    /// vision requests. The explicit deadline authority is shared by web-search
    /// and vision and must be usable in the runtime that drives both tools.
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
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let workspace_tools = open_workspace_tools(workspace_root)?;
        let session_store = open_session_store(session_root)?;
        let memory = open_memory_tool(&session_store)?;

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
            production_ai_gateway_target(),
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            Some(credential_source),
        )
    }

    /// Composes the production AI Gateway HTTP reference host from retained,
    /// identity-checked roots.
    ///
    /// This function consumes the workspace and state descriptors retained by
    /// [`PreparedNativeRoots`] and does not reopen either selected path. It does
    /// not create a runtime, poll the permission prompt, touch session records,
    /// or perform network I/O. The production transport is shared by
    /// language-model, web-search, and vision requests. The explicit deadline
    /// authority is shared by web-search and vision and must be usable in the
    /// runtime that drives both tools.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or a component cannot be constructed safely.
    pub fn compose_ai_gateway_http_with_prepared_roots(
        loaded_config: LoadedNativeConfig,
        credential_environment: AiGatewayCredentialEnvironment,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let (workspace_tools, session_store) = consume_prepared_roots(prepared_roots)?;
        let memory = open_memory_tool(&session_store)?;

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
            production_ai_gateway_target(),
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            Some(credential_source),
        )
    }

    /// Composes the twelve-action terminal host with explicit helper authority.
    ///
    /// Call this synchronous constructor on a blocking worker. It consumes the
    /// retained roots, validates their path bindings, creates dedicated private
    /// startup/archive directories and an archive lock, and captures native
    /// launch/probe configuration. It starts no terminal session, hidden async
    /// runtime, prompt, or network request. Profile-owner startup stays lazy.
    /// Existing constructors without terminal options retain their legacy tool.
    ///
    /// # Errors
    /// Returns a fixed stage-only failure for invalid selections, unavailable
    /// credentials, unsafe roots, or failure to compose the explicit authority.
    pub fn compose_ai_gateway_http_with_prepared_roots_and_terminal(
        loaded_config: LoadedNativeConfig,
        credential_environment: AiGatewayCredentialEnvironment,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        terminal_options: NativeReferenceHostTerminalOptions,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let selection = TerminalCompositionSelection {
            options: terminal_options,
            state_path: prepared_roots.state_root().to_owned(),
        };
        let (workspace_tools, session_store) = consume_prepared_roots(prepared_roots)?;
        let memory = open_memory_tool(&session_store)?;
        let credential = discover_ai_gateway_credential(credential_environment).map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Credential)
        })?;
        let credential_source = credential.source();
        let transport =
            AiGatewayHttpTransport::new(credential.into_bearer_token()).map_err(|_| {
                NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::HttpTransport)
            })?;
        Self::finish_composition_with_extensions(
            loaded_config,
            Arc::new(transport),
            production_ai_gateway_target(),
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            Some(credential_source),
            Arc::new(EmptyMcpToolCatalog),
            Arc::new(EmptyMcpFeatureAuthority),
            Arc::new(EmptySubagentAuthority),
            Some(selection),
        )
    }

    /// Composes a reference host over an explicitly injected AI Gateway transport.
    ///
    /// This path retains the same configuration, workspace, session-store, and
    /// permission selections as production composition, but performs no
    /// credential discovery or HTTP transport construction. `network_target`
    /// must be the canonical HTTP(S) endpoint contacted by `transport`; that
    /// exact target is presented for both web-search and vision authorization.
    /// The injected transport is shared by language-model, web-search, and
    /// vision requests. The explicit deadline authority is shared by web-search
    /// and vision and must be usable in the runtime that drives both tools.
    /// The trusted host must select disjoint workspace and session roots; this
    /// constructor does not compare their identity or ancestor relationships.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or any explicit component cannot be constructed safely.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_ai_gateway_transport(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        workspace_root: &Path,
        session_root: &Path,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let workspace_tools = open_workspace_tools(workspace_root)?;
        let session_store = open_session_store(session_root)?;
        let memory = open_memory_tool(&session_store)?;

        Self::finish_composition(
            loaded_config,
            transport,
            network_target,
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            None,
        )
    }

    /// Composes a reference host with an explicitly injected MCP metadata catalog.
    ///
    /// This is the bounded extension seam used by hosts that already own MCP
    /// discovery and policy admission. Catalog acquisition is inert until the
    /// model calls `mcp_search_tools` or `mcp_select_tool`; this constructor
    /// performs no MCP I/O. Injection supplies admitted metadata and executable
    /// routing. Selection grants no authority; later dynamic calls follow their
    /// ordinary preparation and declared authorization disposition.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or any ordinary reference-host component cannot be constructed safely.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_ai_gateway_transport_and_mcp_catalog(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        workspace_root: &Path,
        session_root: &Path,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        mcp_catalog: Arc<dyn McpToolCatalog>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let workspace_tools = open_workspace_tools(workspace_root)?;
        let session_store = open_session_store(session_root)?;
        let memory = open_memory_tool(&session_store)?;

        Self::finish_composition_with_extensions(
            loaded_config,
            transport,
            network_target,
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            None,
            mcp_catalog,
            Arc::new(EmptyMcpFeatureAuthority),
            Arc::new(EmptySubagentAuthority),
            None,
        )
    }

    /// Composes a reference host with explicitly injected MCP tool and feature
    /// authorities.
    ///
    /// This extends the catalog-only seam with bounded read-only resource,
    /// prompt, and completion access. Both injected allocations are retained
    /// exactly and remain inert during construction. The feature authority is
    /// responsible for exact server/identity admission and live revalidation
    /// before it returns untrusted data; it grants no core permission or other
    /// product authority.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or any ordinary reference-host component cannot be constructed safely.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_ai_gateway_transport_and_mcp(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        workspace_root: &Path,
        session_root: &Path,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        mcp_catalog: Arc<dyn McpToolCatalog>,
        mcp_feature_authority: Arc<dyn McpFeatureAuthority>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let workspace_tools = open_workspace_tools(workspace_root)?;
        let session_store = open_session_store(session_root)?;
        let memory = open_memory_tool(&session_store)?;

        Self::finish_composition_with_extensions(
            loaded_config,
            transport,
            network_target,
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            None,
            mcp_catalog,
            mcp_feature_authority,
            Arc::new(EmptySubagentAuthority),
            None,
        )
    }

    /// Composes a reference host with an explicitly injected foreground
    /// subagent authority and inert MCP authorities.
    ///
    /// The injected allocation is retained exactly and remains inert during
    /// construction. It owns each bounded child run and must not detach work
    /// or expose authority through its result.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or any ordinary reference-host component cannot be constructed safely.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_ai_gateway_transport_and_subagent(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        workspace_root: &Path,
        session_root: &Path,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        subagent_authority: Arc<dyn SubagentAuthority>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let workspace_tools = open_workspace_tools(workspace_root)?;
        let session_store = open_session_store(session_root)?;
        let memory = open_memory_tool(&session_store)?;

        Self::finish_composition_with_extensions(
            loaded_config,
            transport,
            network_target,
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            None,
            Arc::new(EmptyMcpToolCatalog),
            Arc::new(EmptyMcpFeatureAuthority),
            subagent_authority,
            None,
        )
    }

    /// Composes a reference host with explicitly injected MCP and foreground
    /// subagent authorities.
    ///
    /// Every injected allocation is retained exactly and remains inert during
    /// construction. The subagent authority owns one bounded foreground child
    /// run and must not detach work or expose authority through its result.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or any ordinary reference-host component cannot be constructed safely.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_ai_gateway_transport_and_mcp_and_subagent(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        workspace_root: &Path,
        session_root: &Path,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        mcp_catalog: Arc<dyn McpToolCatalog>,
        mcp_feature_authority: Arc<dyn McpFeatureAuthority>,
        subagent_authority: Arc<dyn SubagentAuthority>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let workspace_tools = open_workspace_tools(workspace_root)?;
        let session_store = open_session_store(session_root)?;
        let memory = open_memory_tool(&session_store)?;

        Self::finish_composition_with_extensions(
            loaded_config,
            transport,
            network_target,
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            None,
            mcp_catalog,
            mcp_feature_authority,
            subagent_authority,
            None,
        )
    }

    /// Composes a reference host over an explicitly injected AI Gateway
    /// transport and retained, identity-checked roots.
    ///
    /// This path performs no credential discovery or HTTP transport
    /// construction and does not reopen either path represented by
    /// [`PreparedNativeRoots`]. `network_target` must be the canonical HTTP(S)
    /// endpoint contacted by `transport`; that exact target is presented for
    /// both web-search and vision authorization. The injected transport is
    /// shared by language-model, web-search, and vision requests. The explicit
    /// deadline authority is shared by web-search and vision and must be usable
    /// in the runtime that drives both tools.
    ///
    /// # Errors
    ///
    /// Returns a fixed stage-only error if a configured selection is unsupported
    /// or a component cannot be constructed safely.
    pub fn compose_with_ai_gateway_transport_and_prepared_roots(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let (workspace_tools, session_store) = consume_prepared_roots(prepared_roots)?;
        let memory = open_memory_tool(&session_store)?;

        Self::finish_composition(
            loaded_config,
            transport,
            network_target,
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            None,
        )
    }

    /// Composes the complete terminal host over injected Gateway transport.
    ///
    /// This is the explicit-transport counterpart of
    /// [`Self::compose_ai_gateway_http_with_prepared_roots_and_terminal`], with
    /// the same blocking-worker and retained-root requirements. It performs no
    /// credential discovery and never infers an executable from `current_exe`.
    /// The supplied target must identify the endpoint the transport contacts.
    ///
    /// # Errors
    /// Returns a fixed stage-only failure for invalid selections, unsafe roots,
    /// or failure to compose the explicitly supplied native authority.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_ai_gateway_transport_and_prepared_roots_and_terminal(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        terminal_options: NativeReferenceHostTerminalOptions,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_selections(&loaded_config)?;
        let selection = TerminalCompositionSelection {
            options: terminal_options,
            state_path: prepared_roots.state_root().to_owned(),
        };
        let (workspace_tools, session_store) = consume_prepared_roots(prepared_roots)?;
        let memory = open_memory_tool(&session_store)?;
        Self::finish_composition_with_extensions(
            loaded_config,
            transport,
            network_target,
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            None,
            Arc::new(EmptyMcpToolCatalog),
            Arc::new(EmptyMcpFeatureAuthority),
            Arc::new(EmptySubagentAuthority),
            Some(selection),
        )
    }

    /// Returns the composed provider-neutral engine.
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Returns the concrete store shared exactly with the engine, result reader,
    /// and session lifecycle.
    #[must_use]
    pub const fn session_store(&self) -> &Arc<FileSessionStore> {
        &self.session_store
    }

    /// Returns by-ID durable lifecycle operations over this host's engine and
    /// exact concrete store.
    #[must_use]
    pub const fn session_lifecycle(&self) -> &NativeSessionLifecycle {
        &self.session_lifecycle
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
        network_target: NetworkTarget,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        workspace_tools: WorkspaceTools,
        session_store: FileSessionStore,
        memory: MemoryTool,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        credential_source: Option<AiGatewayCredentialSource>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::finish_composition_with_extensions(
            loaded_config,
            transport,
            network_target,
            web_search_deadline,
            workspace_tools,
            session_store,
            memory,
            permission_prompter,
            question_prompter,
            credential_source,
            Arc::new(EmptyMcpToolCatalog),
            Arc::new(EmptyMcpFeatureAuthority),
            Arc::new(EmptySubagentAuthority),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_composition_with_extensions(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        workspace_tools: WorkspaceTools,
        session_store: FileSessionStore,
        memory: MemoryTool,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        credential_source: Option<AiGatewayCredentialSource>,
        mcp_catalog: Arc<dyn McpToolCatalog>,
        mcp_feature_authority: Arc<dyn McpFeatureAuthority>,
        subagent_authority: Arc<dyn SubagentAuthority>,
        terminal_selection: Option<TerminalCompositionSelection>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        let model = loaded_config.config().model().to_owned();
        let vision_transport = AiGatewayVisionTransport::new(model.clone(), Arc::clone(&transport))
            .map_err(|_| {
                NativeReferenceHostBuildError::new(
                    NativeReferenceHostBuildErrorKind::VisionTransport,
                )
            })?;
        let (vision_deadline, terminal_wait_delay) =
            compose_deadline_adapters(&web_search_deadline);
        let vision = VisionTool::from_root_descriptor(
            workspace_tools.vision_root,
            network_target.clone(),
            Arc::new(vision_transport),
            vision_deadline,
            VisionLimits::default(),
        )
        .map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::VisionConfig)
        })?;
        let search_transport =
            AiGatewayWebSearchTransport::new(model.clone(), Arc::clone(&transport)).map_err(
                |_| {
                    NativeReferenceHostBuildError::new(
                        NativeReferenceHostBuildErrorKind::WebSearchTransport,
                    )
                },
            )?;
        let web_search = WebSearchTool::with_bounded_transport(
            network_target,
            Arc::new(search_transport),
            web_search_deadline,
            WebSearchLimits::default(),
        )
        .map_err(|_| {
            NativeReferenceHostBuildError::new(
                NativeReferenceHostBuildErrorKind::WebSearchTransport,
            )
        })?;
        let (provider, engine_limits) = if terminal_selection.is_some() {
            compose_full_terminal_provider(model, transport)?
        } else {
            compose_provider(model, transport)?
        };
        let web_fetch = compose_web_fetch()?;
        let permission_handler = AskPermissionHandler::shared_prompter(permission_prompter);
        let ask_user_question = AskUserQuestionTool::shared_prompter(question_prompter);
        let (terminal, host_resource, archive): (Arc<dyn Tool>, _, _) =
            if let Some(selection) = terminal_selection {
                let full = compose_full_terminal(
                    workspace_tools.terminal_root,
                    workspace_tools.canonical_workspace,
                    &session_store,
                    selection,
                )?;
                (Arc::new(full.tool), Some(full.resource), Some(full.archive))
            } else {
                let terminal = compose_terminal(
                    workspace_tools.terminal_root,
                    &workspace_tools.canonical_workspace,
                    workspace_tools.background_root,
                    &session_store,
                    terminal_wait_delay,
                )?;
                (Arc::new(terminal), None, None)
            };
        let session_store = Arc::new(session_store);
        let (engine_session_store, read_tool_result) = session_store_components(&session_store);
        let read_tool_result = match archive {
            Some(archive) => read_tool_result.with_archive(archive),
            None => read_tool_result,
        };
        let builder = Engine::builder()
            .limits(engine_limits)
            .provider(provider)
            .shared_session_store(engine_session_store)
            .permission_handler(permission_handler)
            .tool(ask_user_question)
            .tool(workspace_tools.copy_file)
            .tool(workspace_tools.create_folder)
            .tool(workspace_tools.delete_file)
            .tool(workspace_tools.edit_file)
            .tool(workspace_tools.file_info)
            .tool(workspace_tools.glob_files)
            .tool(workspace_tools.grep_files)
            .tool(workspace_tools.install_skill)
            .tool(workspace_tools.list_files)
            .tool(McpSearchToolsTool::shared_catalog(Arc::clone(&mcp_catalog)))
            .tool(McpSelectTool::shared_catalog(mcp_catalog))
            .tool(McpFeaturesTool::shared_authority(mcp_feature_authority))
            .tool(memory)
            .tool(workspace_tools.open_file)
            .tool(workspace_tools.read_file)
            .tool(read_tool_result)
            .tool(workspace_tools.rename_file)
            .tool(workspace_tools.semantic_search)
            .tool(workspace_tools.skill)
            .tool(SubagentTool::shared_authority(subagent_authority))
            .shared_tool(terminal)
            .tool(vision)
            .tool(web_fetch)
            .tool(web_search)
            .tool(workspace_tools.write_file);
        let builder = match host_resource {
            Some(resource) => builder.host_resource(resource),
            None => builder,
        };
        let engine = builder.build().map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Engine)
        })?;
        let session_lifecycle =
            NativeSessionLifecycle::new(engine.clone(), Arc::clone(&session_store)).map_err(
                |_| NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Engine),
            )?;

        Ok(Self {
            engine,
            session_store,
            session_lifecycle,
            loaded_config,
            credential_source,
        })
    }
}

fn compose_full_terminal_provider(
    model: String,
    transport: Arc<dyn AiGatewayTransport>,
) -> Result<(AiGatewayProvider, EngineLimits), NativeReferenceHostBuildError> {
    let input_bytes = crate::MAX_TERMINAL_ACTION_ARGUMENT_BYTES;
    let input_nodes = crate::MAX_TERMINAL_ACTION_ARGUMENT_NODES;
    let provider = AiGatewayProvider::with_limits(model, transport, AiGatewayLimits::default())
        .and_then(|provider| {
            provider.with_tool_input_limits([(
                ToolName::new(crate::TERMINAL_TOOL_NAME).expect("terminal tool name is valid"),
                AiGatewayToolInputLimits {
                    max_argument_bytes: input_bytes,
                    max_json_nodes: input_nodes,
                },
            )])
        })
        .map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Provider)
        })?;
    // Complete payloads have an independent aggregate budget. Ordinary tools
    // and persisted transcript references keep their conservative defaults.
    let limits = EngineLimits {
        max_cumulative_complete_tool_argument_bytes: NonZeroUsize::new(input_bytes)
            .expect("terminal input byte bound is nonzero"),
        max_cumulative_complete_tool_argument_nodes: NonZeroUsize::new(input_nodes)
            .expect("terminal input node bound is nonzero"),
        max_cumulative_complete_tool_result_bytes: NonZeroUsize::new(
            crate::MAX_TERMINAL_COMPLETE_TOOL_OUTPUT_BYTES,
        )
        .expect("terminal output byte bound is nonzero"),
        ..EngineLimits::default()
    };
    Ok((provider, limits))
}

struct FullTerminalComposition {
    tool: crate::TerminalActionTool,
    resource: NativeTerminalHostResource,
    archive: Arc<NativeToolResultArchiveAdapter>,
}

fn compose_full_terminal(
    workspace: OwnedFd,
    workspace_path: PathBuf,
    session_store: &FileSessionStore,
    selection: TerminalCompositionSelection,
) -> Result<FullTerminalComposition, NativeReferenceHostBuildError> {
    use std::fmt::Write as _;
    let state_root = session_store
        .try_clone_root_descriptor()
        .map_err(|_| terminal_options_error())?;
    crate::background_store::BackgroundStore::validate_state_root(&state_root)
        .map_err(|_| terminal_options_error())?;
    let state_path =
        std::fs::canonicalize(selection.state_path).map_err(|_| terminal_options_error())?;
    crate::terminal_helper::validate_startup_directory(&state_root, &state_path)
        .map_err(|_| terminal_options_error())?;
    let artifacts = crate::terminal_catalog::prepare_directory(&state_root, "terminal-startup")
        .map_err(|_| terminal_options_error())?;
    let archive_root =
        crate::terminal_catalog::prepare_directory(&state_root, "tool-result-archive")
            .map_err(|_| terminal_options_error())?;
    let archive = Arc::new(ToolResultArchive::from_root_descriptor(archive_root));
    archive.prepare().map_err(|_| terminal_options_error())?;
    let archive = Arc::new(NativeToolResultArchiveAdapter::new(archive));
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| terminal_options_error())?;
    let mut host_identity = String::from("terminal-host-");
    for byte in random {
        write!(&mut host_identity, "{byte:02x}").expect("String formatting is infallible");
    }
    let host_identity =
        SessionIncarnationId::new(host_identity).map_err(|_| terminal_options_error())?;
    let options = selection.options;
    let inputs = TerminalHostAuthorityInputs {
        workspace,
        default_cwd: workspace_path.clone(),
        workspace_path,
        environment: options.environment.entries().to_vec(),
        account_shell: TerminalHostAccountShell::Explicit(options.account_shell),
        cli_executable: options.helper_program,
        tmux_executable: options.tmux_program,
        artifacts,
        artifact_path: state_path.join("terminal-startup"),
    };
    let (tool, resource) = NativeTerminalHost::compose_on_worker(inputs, state_root, host_identity)
        .map_err(|_| terminal_options_error())?;
    Ok(FullTerminalComposition {
        tool: tool
            .with_input_publisher(archive.clone())
            .with_result_publisher(archive.clone()),
        resource,
        archive,
    })
}

fn compose_provider(
    model: String,
    transport: Arc<dyn AiGatewayTransport>,
) -> Result<(AiGatewayProvider, EngineLimits), NativeReferenceHostBuildError> {
    // A semantic 64 KiB command can require six JSON bytes per input byte.
    // Align provider admission and engine preflight with the tool's canonical
    // envelope, without changing generic embedder defaults.
    let argument_bytes = crate::MAX_TERMINAL_SERIALIZED_ARGUMENT_BYTES;
    let provider = AiGatewayProvider::with_limits(
        model,
        transport,
        AiGatewayLimits {
            max_tool_arguments_bytes: argument_bytes,
            ..AiGatewayLimits::default()
        },
    )
    .map_err(|_| NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Provider))?;
    Ok((
        provider,
        EngineLimits {
            max_tool_argument_bytes: NonZeroUsize::new(argument_bytes)
                .expect("terminal argument envelope is nonzero"),
            ..EngineLimits::default()
        },
    ))
}

fn compose_terminal(
    terminal_root: OwnedFd,
    canonical_workspace: &Path,
    background_root: OwnedFd,
    session_store: &FileSessionStore,
    wait_delay: Arc<dyn TerminalBackgroundWaitDelay>,
) -> Result<TerminalTool, NativeReferenceHostBuildError> {
    let terminal = TerminalTool::from_root_descriptor(terminal_root).map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::TerminalConfig)
    })?;
    let canonical_workspace = canonical_workspace
        .to_str()
        .ok_or_else(|| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::BackgroundConfig)
        })?
        .to_owned();
    let state_root = session_store.try_clone_root_descriptor().map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::BackgroundConfig)
    })?;
    let inspector_root = session_store.try_clone_root_descriptor().map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::BackgroundConfig)
    })?;
    let inspector = Arc::new(
        NativeBackgroundRecordInspector::from_root_descriptor(
            inspector_root,
            canonical_workspace.clone(),
        )
        .map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::BackgroundConfig)
        })?,
    );
    let background = Arc::new(
        LazyProductionBackgroundStarter::from_root_descriptors(
            canonical_workspace.clone(),
            background_root,
            state_root,
        )
        .map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::BackgroundConfig)
        })?,
    );
    let environment = background.environment_identity();
    let output_reader: Arc<dyn TerminalBackgroundOutputReader> = background.clone();
    let signaler: Arc<dyn TerminalBackgroundSignaler> = background.clone();
    let writer: Arc<dyn TerminalBackgroundWriter> = background.clone();
    let background: Arc<dyn TerminalBackgroundStarter> = background;
    let catalog: Arc<dyn TerminalBackgroundCatalog> = inspector.clone();
    let inspector: Arc<dyn TerminalBackgroundInspector> = inspector;
    terminal
        .with_background(canonical_workspace, environment, background)
        .and_then(|terminal| terminal.with_output_reader(output_reader))
        .and_then(|terminal| terminal.with_signaler(signaler))
        .and_then(|terminal| terminal.with_writer(writer))
        .and_then(|terminal| terminal.with_catalog(catalog))
        .and_then(|terminal| terminal.with_inspector(inspector))
        .and_then(|terminal| terminal.with_wait_delay(wait_delay))
        .map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::TerminalConfig)
        })
}

fn compose_web_fetch() -> Result<WebFetchTool, NativeReferenceHostBuildError> {
    WebFetchTool::new().map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::WebFetchTransport)
    })
}

fn session_store_components(
    session_store: &Arc<FileSessionStore>,
) -> (Arc<dyn SessionStore>, ReadToolResultTool) {
    let erased = Arc::clone(session_store) as Arc<dyn SessionStore>;
    let reader = ReadToolResultTool::shared_session_store(Arc::clone(&erased));
    (erased, reader)
}

#[derive(Clone, Copy, Debug)]
struct EmptyMcpToolCatalog;

impl McpToolCatalog for EmptyMcpToolCatalog {
    fn snapshot(
        &self,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async {
            Ok(McpToolCatalogSnapshot::new(Vec::new())
                .expect("the empty MCP tool catalog is always valid"))
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct EmptyMcpFeatureAuthority;

impl McpFeatureAuthority for EmptyMcpFeatureAuthority {
    fn call(
        &self,
        _request: McpFeatureRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpFeaturePayload, McpFeatureError>> {
        Box::pin(async { Err(McpFeatureError::new(McpFeatureErrorKind::Unavailable)) })
    }
}

#[derive(Clone, Copy, Debug)]
struct EmptySubagentAuthority;

impl SubagentAuthority for EmptySubagentAuthority {
    fn run(
        &self,
        _request: SubagentRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<SubagentOutcome, SubagentAuthorityError>> {
        Box::pin(async {
            Err(SubagentAuthorityError::new(
                SubagentAuthorityErrorKind::Unavailable,
            ))
        })
    }
}

struct VisionDeadlineAdapter {
    inner: Arc<dyn WebSearchDeadline>,
}

struct TerminalBackgroundWaitDelayAdapter {
    inner: Arc<dyn WebSearchDeadline>,
}

fn compose_deadline_adapters(
    deadline: &Arc<dyn WebSearchDeadline>,
) -> (
    Arc<dyn VisionDeadline>,
    Arc<dyn TerminalBackgroundWaitDelay>,
) {
    // The public host requires one inert absolute deadline authority for web
    // search, vision, and persisted background waits. These private adapters
    // translate only fixed error categories and never expose diagnostics.
    (
        Arc::new(VisionDeadlineAdapter {
            inner: Arc::clone(deadline),
        }),
        Arc::new(TerminalBackgroundWaitDelayAdapter {
            inner: Arc::clone(deadline),
        }),
    )
}

impl TerminalBackgroundWaitDelay for TerminalBackgroundWaitDelayAdapter {
    fn wait_until(
        &self,
        deadline: Instant,
    ) -> BoxFuture<'_, Result<(), TerminalBackgroundWaitDelayError>> {
        let wait = self.inner.wait_until(deadline);
        Box::pin(async move {
            wait.await
                .map_err(|_| TerminalBackgroundWaitDelayError::new())
        })
    }
}

impl VisionDeadline for VisionDeadlineAdapter {
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, Result<(), VisionTransportError>> {
        let wait = self.inner.wait_until(deadline);
        Box::pin(async move { wait.await.map_err(map_vision_deadline_error) })
    }
}

fn map_vision_deadline_error(error: crate::WebSearchTransportError) -> VisionTransportError {
    let kind = match error.kind() {
        WebSearchTransportErrorKind::InvalidRequest => VisionTransportErrorKind::InvalidRequest,
        WebSearchTransportErrorKind::Authentication => VisionTransportErrorKind::Authentication,
        WebSearchTransportErrorKind::RateLimited => VisionTransportErrorKind::RateLimited,
        WebSearchTransportErrorKind::Timeout => VisionTransportErrorKind::Timeout,
        WebSearchTransportErrorKind::Unavailable => VisionTransportErrorKind::Unavailable,
        WebSearchTransportErrorKind::InvalidResponse => VisionTransportErrorKind::InvalidResponse,
        WebSearchTransportErrorKind::Protocol => VisionTransportErrorKind::Protocol,
        WebSearchTransportErrorKind::ResponseTooLarge
        | WebSearchTransportErrorKind::ResultTooLarge => VisionTransportErrorKind::ResponseTooLarge,
        WebSearchTransportErrorKind::RuntimeRequired => VisionTransportErrorKind::RuntimeRequired,
        WebSearchTransportErrorKind::Cancelled => VisionTransportErrorKind::Cancelled,
    };
    VisionTransportError::new(kind)
}

fn production_ai_gateway_target() -> NetworkTarget {
    NetworkTarget {
        scheme: "https".to_owned(),
        host: "ai-gateway.vercel.sh".to_owned(),
        port: None,
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
    if config.credential_source() != NativeCredentialSourceKind::Environment
        || config.provider() != NativeProviderKind::VercelAiGateway
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
) -> Result<WorkspaceTools, NativeReferenceHostBuildError> {
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

fn open_memory_tool(
    session_store: &FileSessionStore,
) -> Result<MemoryTool, NativeReferenceHostBuildError> {
    let root = session_store.try_clone_root_descriptor().map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Memory)
    })?;
    Ok(MemoryTool::from_root_descriptor(root))
}

fn consume_prepared_roots(
    prepared_roots: PreparedNativeRoots,
) -> Result<(WorkspaceTools, FileSessionStore), NativeReferenceHostBuildError> {
    prepared_roots.into_parts().map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::WorkspaceRoot)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind,
        NativeReferenceHostTerminalOptions, map_vision_deadline_error,
    };
    use crate::background_process::{
        MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES, MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES,
        MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES,
        MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES,
    };
    use crate::{VisionTransportErrorKind, WebSearchTransportError, WebSearchTransportErrorKind};
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    #[test]
    fn terminal_options_freeze_explicit_inputs_without_discovery_or_execution() {
        let environment = vec![("SHELL".into(), "/ignored/environment/shell".into())];
        let options = NativeReferenceHostTerminalOptions::new(
            "/not-opened/private-machine-god".into(),
            Some("/not-opened/account/bash".into()),
            environment.clone(),
        )
        .unwrap();
        assert_eq!(
            options.helper_program(),
            Path::new("/not-opened/private-machine-god")
        );
        assert_eq!(
            options.account_shell(),
            Some(Path::new("/not-opened/account/bash"))
        );
        assert_eq!(options.environment(), environment);
        assert_eq!(options.tmux_program(), None);
        let options = options.with_tmux("/not-opened/tmux".into()).unwrap();
        assert_eq!(options.tmux_program(), Some(Path::new("/not-opened/tmux")));
        let clone = options.clone();
        assert!(options.environment.shares_storage_with(&clone.environment));
        assert_eq!(
            format!("{options:?}"),
            "NativeReferenceHostTerminalOptions { .. }"
        );
    }

    #[test]
    fn terminal_options_absent_account_does_not_use_environment_shell() {
        let options = NativeReferenceHostTerminalOptions::new(
            "/explicit/machine-god".into(),
            None,
            vec![("SHELL".into(), "/must-not-be-selected/zsh".into())],
        )
        .unwrap();
        assert_eq!(options.account_shell(), None);
        let options = NativeReferenceHostTerminalOptions::new(
            "/explicit/machine-god".into(),
            Some("/explicit/account/fish".into()),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            options.account_shell(),
            Some(Path::new("/explicit/account/fish"))
        );
    }

    #[test]
    fn terminal_options_reject_invalid_programs_with_redacted_errors() {
        for program in [
            PathBuf::from(""),
            "relative/machine-god".into(),
            "/".into(),
            "/private/secret\0suffix".into(),
            format!("/{}", "a".repeat(crate::terminal_helper::MAX_PROGRAM_BYTES)).into(),
            PathBuf::from(OsString::from_vec(b"/private/\xff".to_vec())),
        ] {
            let error = NativeReferenceHostTerminalOptions::new(program.clone(), None, Vec::new())
                .unwrap_err();
            assert_eq!(
                error.kind(),
                NativeReferenceHostBuildErrorKind::TerminalConfig
            );
            assert_eq!(
                error.to_string(),
                "native reference-host terminal construction failed"
            );
            let error = NativeReferenceHostTerminalOptions::new(
                "/explicit/machine-god".into(),
                None,
                Vec::new(),
            )
            .unwrap()
            .with_tmux(program)
            .unwrap_err();
            assert_eq!(
                error.kind(),
                NativeReferenceHostBuildErrorKind::TerminalConfig
            );
        }
        for shell in ["relative/bash", "/private/secret\0bash"] {
            assert!(
                NativeReferenceHostTerminalOptions::new(
                    "/explicit/machine-god".into(),
                    Some(shell.into()),
                    Vec::new(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn terminal_options_apply_existing_environment_bounds_and_uniqueness() {
        let mut invalid = vec![
            vec![(OsString::new(), "value".into())],
            vec![("BAD=KEY".into(), "value".into())],
            vec![("KEY\0".into(), "value".into())],
            vec![("KEY".into(), "value\0".into())],
            vec![("KEY".into(), "one".into()), ("KEY".into(), "two".into())],
            vec![(
                "K".repeat(MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES + 1)
                    .into(),
                "v".into(),
            )],
            vec![(
                "KEY".into(),
                "v".repeat(MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES + 1)
                    .into(),
            )],
        ];
        invalid.push(
            (0..=MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES)
                .map(|index| (format!("K{index}").into(), "v".into()))
                .collect(),
        );
        invalid.push(
            (0..=MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES
                / MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES)
                .map(|index| {
                    (
                        format!("K{index}").into(),
                        "v".repeat(MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES)
                            .into(),
                    )
                })
                .collect(),
        );
        for environment in invalid {
            let error = NativeReferenceHostTerminalOptions::new(
                "/explicit/machine-god".into(),
                None,
                environment,
            )
            .unwrap_err();
            assert_eq!(
                error.kind(),
                NativeReferenceHostBuildErrorKind::TerminalConfig
            );
        }
        let raw = vec![("RAW".into(), OsString::from_vec(vec![0xff, 0xfe]))];
        let options = NativeReferenceHostTerminalOptions::new(
            "/explicit/machine-god".into(),
            None,
            raw.clone(),
        )
        .unwrap();
        assert_eq!(options.environment(), raw);
    }

    #[test]
    fn memory_configuration_failure_has_one_fixed_redacted_shape() {
        let error = NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Memory);

        assert_eq!(
            error.to_string(),
            "native reference-host memory construction failed"
        );
        assert_eq!(
            format!("{error:?}"),
            "NativeReferenceHostBuildError { kind: Memory }"
        );
    }

    #[test]
    fn terminal_configuration_failure_has_one_fixed_redacted_shape() {
        let error =
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::TerminalConfig);

        assert_eq!(
            error.to_string(),
            "native reference-host terminal construction failed"
        );
        assert_eq!(
            format!("{error:?}"),
            "NativeReferenceHostBuildError { kind: TerminalConfig }"
        );
    }

    #[test]
    fn vision_construction_failures_have_fixed_stage_only_shapes() {
        for (kind, expected) in [
            (
                NativeReferenceHostBuildErrorKind::VisionTransport,
                "native reference-host vision transport construction failed",
            ),
            (
                NativeReferenceHostBuildErrorKind::VisionConfig,
                "native reference-host vision construction failed",
            ),
        ] {
            let error = NativeReferenceHostBuildError::new(kind);
            assert_eq!(error.to_string(), expected);
            assert_eq!(
                format!("{error:?}"),
                format!("NativeReferenceHostBuildError {{ kind: {kind:?} }}")
            );
        }
    }

    #[test]
    fn shared_deadline_adapter_maps_every_stable_error_kind_exactly() {
        for (source, expected) in [
            (
                WebSearchTransportErrorKind::InvalidRequest,
                VisionTransportErrorKind::InvalidRequest,
            ),
            (
                WebSearchTransportErrorKind::Authentication,
                VisionTransportErrorKind::Authentication,
            ),
            (
                WebSearchTransportErrorKind::RateLimited,
                VisionTransportErrorKind::RateLimited,
            ),
            (
                WebSearchTransportErrorKind::Timeout,
                VisionTransportErrorKind::Timeout,
            ),
            (
                WebSearchTransportErrorKind::Unavailable,
                VisionTransportErrorKind::Unavailable,
            ),
            (
                WebSearchTransportErrorKind::InvalidResponse,
                VisionTransportErrorKind::InvalidResponse,
            ),
            (
                WebSearchTransportErrorKind::Protocol,
                VisionTransportErrorKind::Protocol,
            ),
            (
                WebSearchTransportErrorKind::ResponseTooLarge,
                VisionTransportErrorKind::ResponseTooLarge,
            ),
            (
                WebSearchTransportErrorKind::ResultTooLarge,
                VisionTransportErrorKind::ResponseTooLarge,
            ),
            (
                WebSearchTransportErrorKind::RuntimeRequired,
                VisionTransportErrorKind::RuntimeRequired,
            ),
            (
                WebSearchTransportErrorKind::Cancelled,
                VisionTransportErrorKind::Cancelled,
            ),
        ] {
            assert_eq!(
                map_vision_deadline_error(WebSearchTransportError::new(source)).kind(),
                expected
            );
        }
    }
}
