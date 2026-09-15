use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

mod construction;
mod managed_host;
mod mcp;
mod permissions;
mod services;
pub(crate) mod subagent;
pub(crate) mod workspace_binding;
pub use managed_host::NativeReferenceHostManagedOptions;
pub use mcp::{NativeReferenceHostMcpEphemeralStartupOptions, NativeReferenceHostMcpOptions};
pub use permissions::NativeReferenceHostPermissionOptions;
use permissions::{PermissionComposition, ReferenceHostToolCatalog};
use services::NativeHostServices;
use workspace_binding::WorkspaceBinding;

use machine_god_core::{
    BoxFuture, CancellationToken, Engine, EngineLimits, ManagedSubagentAuthority,
    ManagedSubagentError, ManagedSubagentInvocation, ManagedSubagentResult, NetworkTarget,
    SessionIncarnationId, SessionStore, SubagentTool, Tool, ToolContext, ToolName,
};
use rustix::fd::OwnedFd;

use crate::background_inspection::NativeBackgroundRecordInspector;
use crate::background_process::ValidatedBackgroundEnvironment;
use crate::background_supervisor::LazyProductionBackgroundStarter;
use crate::file_history_tool::{NativeFileHistoryKind, NativeFileHistoryTool};
use crate::terminal_host::{NativeTerminalHost, NativeTerminalHostResource};
use crate::terminal_host_authority::{TerminalHostAccountShell, TerminalHostAuthorityInputs};
use crate::workspace::{WorkspaceRoot, WorkspaceTools};
use crate::{
    AiGatewayBearerToken, AiGatewayCredentialEnvironment, AiGatewayCredentialSource,
    AiGatewayHttpConfigError, AiGatewayHttpTransport, AiGatewayLimits, AiGatewayProvider,
    AiGatewayToolInputLimits, AiGatewayTransport, AiGatewayVisionTransport,
    AiGatewayWebSearchTransport, AskUserQuestionTool, DiscoveredAiGatewayCredential,
    FileSessionStore, FileUndoTracker, LoadedNativeConfig, McpFeatureAuthority, McpFeatureError,
    McpFeatureErrorKind, McpFeaturePayload, McpFeatureRequest, McpSearchToolsTool, McpSelectTool,
    McpToolCatalog, McpToolCatalogError, McpToolCatalogSnapshot, MemoryTool,
    NativeCredentialSourceKind, NativeProviderKind, NativeSessionLifecycle,
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
    /// Explicit permission authority could not be composed.
    PermissionConfig,
    /// Explicit native MCP archive/runtime authority could not be composed.
    McpConfig,
    /// Managed construction requires the complete shared native execution domain.
    ManagedConfig,
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
            NativeReferenceHostBuildErrorKind::PermissionConfig => {
                "native reference-host permission configuration failed"
            }
            NativeReferenceHostBuildErrorKind::McpConfig => {
                "native reference-host MCP configuration failed"
            }
            NativeReferenceHostBuildErrorKind::ManagedConfig => {
                "native reference-host managed-agent configuration failed"
            }
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

/// Explicit process-local conversation authority for reference-host composition.
///
/// Supplying the tracker authorizes bounded preimage reads and later inverse
/// mutations under its retained descriptors for all five file mutation tools.
/// An ordinary write/delete/rename approval alone does not grant these reads.
/// The trusted host owns this additional authority choice and must clear or
/// replace the process-local tracker at its conversation-lifetime boundary.
/// Construction is inert and does not capture files, open roots, or start work.
#[derive(Clone)]
pub struct NativeReferenceHostConversationOptions {
    managed: Option<NativeReferenceHostManagedOptions>,
    background_url: Option<BackgroundUrlSelection>,
    mcp_runtime: Option<NativeReferenceHostMcpOptions>,
    mcp_management: Option<Arc<crate::mcp::management::NativeMcpManagementService>>,
    mcp_contexts: Option<Arc<crate::mcp::context::NativeMcpContexts>>,
    skills: Option<Arc<crate::NativeSkillsService>>,
    workspace_binding: Option<WorkspaceBinding>,
    undo_tracker: Arc<FileUndoTracker>,
    terminal: Option<NativeReferenceHostTerminalOptions>,
    model_routes: Option<Arc<crate::NativeConversationModelRoutes>>,
    observations: Option<Arc<crate::NativeConversationObservations>>,
    permissions: Option<NativeReferenceHostPermissionOptions>,
}

impl NativeReferenceHostConversationOptions {
    /// Retains the caller's exact shared tracker without reading or resetting it.
    #[must_use]
    pub fn new(undo_tracker: Arc<FileUndoTracker>) -> Self {
        Self {
            managed: None,
            background_url: None,
            mcp_runtime: None,
            mcp_management: None,
            mcp_contexts: None,
            skills: None,
            workspace_binding: None,
            undo_tracker,
            terminal: None,
            model_routes: None,
            observations: None,
            permissions: None,
        }
    }

    /// Installs weak managed tool routes. The caller must subsequently prepare
    /// and drive the outer native manager before admitting model work.
    #[must_use]
    pub fn with_managed_agents(mut self, options: NativeReferenceHostManagedOptions) -> Self {
        self.managed = Some(options);
        self
    }

    /// Also selects the existing complete terminal authority for this host.
    /// Without this explicit selection, composition keeps the legacy terminal.
    #[must_use]
    pub fn with_terminal(mut self, terminal: NativeReferenceHostTerminalOptions) -> Self {
        self.terminal = Some(terminal);
        self
    }

    /// Retains an explicit desktop launcher selection without inspecting or
    /// executing it. Composition binds it to the complete terminal's workers;
    /// invalid optional environment authority leaves the launcher unavailable.
    #[must_use]
    pub fn with_background_url_opener(
        mut self,
        executable: crate::NativeBackgroundUrlExecutable,
        environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> Self {
        self.background_url = Some(BackgroundUrlSelection {
            executable,
            environment,
        });
        self
    }

    /// Retains explicit human-invoked skill authority without discovery or writes.
    /// Composition requires complete terminal options for owned worker cleanup.
    #[must_use]
    pub fn with_skills(mut self, skills: Arc<crate::NativeSkillsService>) -> Self {
        self.skills = Some(skills);
        self
    }

    /// Retains explicit MCP profile authority without reading or activating it.
    /// Composition requires complete terminal options for owned worker cleanup.
    #[must_use]
    pub fn with_mcp_management(
        mut self,
        service: Arc<crate::mcp::management::NativeMcpManagementService>,
    ) -> Self {
        self.mcp_management = Some(service);
        self
    }

    /// Retains exact-turn MCP routing without registering sessions or opening servers.
    /// Attach each conversation with `configure_conversation_mcp` before admission.
    #[must_use]
    pub fn with_mcp_contexts(
        mut self,
        contexts: Arc<crate::mcp::context::NativeMcpContexts>,
    ) -> Self {
        self.mcp_contexts = Some(contexts);
        self
    }

    /// Selects real native MCP execution with the host's shared archive and
    /// permission controller. Requires complete terminal and permission options;
    /// a separately selected MCP context allocation must be exactly identical.
    /// This option itself connects no server and opens no archive or runtime.
    #[must_use]
    pub fn with_mcp_runtime(mut self, options: NativeReferenceHostMcpOptions) -> Self {
        self.mcp_runtime = Some(options);
        self
    }

    /// Selects explicit additional-root authority and exact-turn routing.
    /// Composition validates primary/state descriptors against the prepared roots.
    /// Attach each conversation through `configure_conversation_workspace` before
    /// admission. Retaining these allocations performs no filesystem work.
    #[must_use]
    pub fn with_workspace(
        mut self,
        authority: crate::NativeWorkspaceAuthority,
        contexts: Arc<crate::NativeWorkspaceContexts>,
    ) -> Self {
        self.workspace_binding = Some(WorkspaceBinding {
            authority,
            contexts,
        });
        self
    }

    /// Selects incarnation-bound current-model snapshots for web search.
    /// Register each conversation runtime with this same routing allocation.
    #[must_use]
    pub fn with_model_routes(mut self, routes: Arc<crate::NativeConversationModelRoutes>) -> Self {
        self.model_routes = Some(routes);
        self
    }

    /// Records native file observations for conversations attached to this registry.
    /// Attach each conversation with its exact shared allocation before admission.
    #[must_use]
    pub fn with_observations(
        mut self,
        observations: Arc<crate::NativeConversationObservations>,
    ) -> Self {
        self.observations = Some(observations);
        self
    }

    /// Enables native mode/rule enforcement and selected-file approval reads.
    /// Requires the complete terminal selection and matching conversation routes.
    #[must_use]
    pub fn with_permissions(mut self, permissions: NativeReferenceHostPermissionOptions) -> Self {
        self.permissions = Some(permissions);
        self
    }
}

impl fmt::Debug for NativeReferenceHostConversationOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeReferenceHostConversationOptions")
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct PreparedCompositionOptions {
    managed: Option<NativeReferenceHostManagedOptions>,
    background_url: Option<BackgroundUrlSelection>,
    mcp_runtime: Option<NativeReferenceHostMcpOptions>,
    mcp_management: Option<Arc<crate::mcp::management::NativeMcpManagementService>>,
    mcp_contexts: Option<Arc<crate::mcp::context::NativeMcpContexts>>,
    skills: Option<Arc<crate::NativeSkillsService>>,
    workspace_binding: Option<WorkspaceBinding>,
    undo_tracker: Option<Arc<FileUndoTracker>>,
    terminal: Option<NativeReferenceHostTerminalOptions>,
    model_routes: Option<Arc<crate::NativeConversationModelRoutes>>,
    observations: Option<Arc<crate::NativeConversationObservations>>,
    permissions: Option<NativeReferenceHostPermissionOptions>,
}

impl From<NativeReferenceHostConversationOptions> for PreparedCompositionOptions {
    fn from(options: NativeReferenceHostConversationOptions) -> Self {
        Self {
            managed: options.managed,
            background_url: options.background_url,
            mcp_runtime: options.mcp_runtime,
            mcp_management: options.mcp_management,
            mcp_contexts: options.mcp_contexts,
            skills: options.skills,
            undo_tracker: Some(options.undo_tracker),
            workspace_binding: options.workspace_binding,
            terminal: options.terminal,
            model_routes: options.model_routes,
            observations: options.observations,
            permissions: options.permissions,
        }
    }
}

#[derive(Clone)]
struct BackgroundUrlSelection {
    executable: crate::NativeBackgroundUrlExecutable,
    environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
}

impl BackgroundUrlSelection {
    fn bind(
        self,
        terminal: &SelectedTerminalComposition,
    ) -> Result<crate::NativeBackgroundUrlOpener, crate::NativeBackgroundOpenError> {
        crate::NativeBackgroundUrlOpener::from_executable(
            self.executable,
            self.environment,
            terminal
                .resource
                .as_ref()
                .ok_or(crate::NativeBackgroundOpenError::Unavailable)?
                .worker_scope(),
        )
    }
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
    managed: Option<managed_host::ManagedHostAssembly>,
    services: Arc<NativeHostServices>,
    background_opener:
        Option<Result<crate::NativeBackgroundUrlOpener, crate::NativeBackgroundOpenError>>,
    mcp_runtime: Option<Arc<crate::mcp::runtime::NativeMcpRuntime>>,
    mcp_controller: Option<Arc<crate::mcp::controller::NativeMcpController>>,
    mcp_ephemeral: Option<Arc<crate::mcp::ephemeral::NativeMcpEphemeralOwner>>,
    mcp_clock: Option<Arc<dyn crate::mcp::runtime::NativeMcpRuntimeClock>>,
    reserved_tool_names: Box<[ToolName]>,
    mcp_management: Option<Arc<crate::mcp::management::NativeMcpManagementService>>,
    mcp_contexts: Option<Arc<crate::mcp::context::NativeMcpContexts>>,
    skills: Option<Arc<crate::NativeSkillsService>>,
    workspace_binding: Option<WorkspaceBinding>,
    workspace_root: PathBuf,
    loaded_config: LoadedNativeConfig,
    credential_source: Option<AiGatewayCredentialSource>,
    undo_tracker: Option<Arc<FileUndoTracker>>,
}

impl NativeReferenceHost {
    /// Shares the already-bound launcher; this accessor captures no authority.
    pub(crate) fn background_url_opener(&self) -> Option<crate::NativeBackgroundUrlOpener> {
        self.background_opener.as_ref()?.as_ref().ok().cloned()
    }

    pub(crate) fn has_background_url_selection(&self) -> bool {
        self.background_opener.is_some()
    }

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
        Self::compose_production_prepared(
            loaded_config,
            credential_environment,
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            PreparedCompositionOptions::default(),
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
        Self::compose_production_prepared(
            loaded_config,
            credential_environment,
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            PreparedCompositionOptions {
                terminal: Some(terminal_options),
                ..Default::default()
            },
        )
    }

    /// Composes production HTTP with explicitly authorized shared file undo.
    ///
    /// Consumes the retained prepared roots without reopening selected paths.
    /// The exact options tracker is shared by write, edit, delete, rename, and
    /// copy tools before engine construction. Existing constructors do not
    /// implicitly enable undo or acquire preimage-read authority.
    /// Optional terminal selection has the same blocking-worker, directory
    /// preparation, and lifetime requirements as the terminal-only constructor.
    /// No permission prompt, file snapshot, undo, or network request runs here.
    ///
    /// # Errors
    /// Returns a redacted stage-only failure for unsupported selections,
    /// credentials, unsafe roots, or component construction failure.
    pub fn compose_ai_gateway_http_with_prepared_roots_and_conversation(
        loaded_config: LoadedNativeConfig,
        credential_environment: AiGatewayCredentialEnvironment,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        conversation_options: NativeReferenceHostConversationOptions,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::compose_production_prepared(
            loaded_config,
            credential_environment,
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            conversation_options.into(),
        )
    }

    fn compose_production_prepared(
        loaded_config: LoadedNativeConfig,
        credential_environment: AiGatewayCredentialEnvironment,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        options: PreparedCompositionOptions,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::compose_production_prepared_with_credential(
            loaded_config,
            || {
                discover_ai_gateway_credential(credential_environment).map_err(|_| {
                    NativeReferenceHostBuildError::new(
                        NativeReferenceHostBuildErrorKind::Credential,
                    )
                })
            },
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            options,
            production_ai_gateway_transport,
        )
    }

    /// Composes production HTTP and shared file undo using an already acquired
    /// credential, preserving its concrete OIDC or API-key source observation.
    ///
    /// A trusted startup owner may first borrow this credential for an
    /// authenticated model catalog, then move it here for inference. No second
    /// credential acquisition or process-credential discovery occurs. Root, undo,
    /// optional terminal, and blocking-worker requirements match
    /// [`Self::compose_ai_gateway_http_with_prepared_roots_and_conversation`].
    /// Composition itself makes no network request and starts no runtime.
    ///
    /// # Errors
    /// Returns existing redacted stage-only failures for unsupported selections,
    /// unsafe roots, or component construction failure.
    pub fn compose_ai_gateway_http_with_prepared_roots_and_conversation_and_credential(
        loaded_config: LoadedNativeConfig,
        credential: DiscoveredAiGatewayCredential,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        conversation_options: NativeReferenceHostConversationOptions,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::compose_production_prepared_with_credential(
            loaded_config,
            || Ok(credential),
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            conversation_options.into(),
            production_ai_gateway_transport,
        )
    }

    /// Composes the prepared credential-bearing host with a trusted transport factory.
    ///
    /// Preserves the acquired credential's source and moves its token into the
    /// one-shot factory after the ordinary selection and retained-root checks.
    /// No credential discovery is repeated. The factory must return the canonical
    /// target actually contacted by its transport; any factory effects belong to
    /// the explicitly injecting host. Composition never polls the returned transport.
    /// All conversation authorities and cleanup requirements match the production
    /// constructor. This is a programmatic seam, not a CLI endpoint selection.
    ///
    /// # Errors
    /// Returns the existing redacted composition stages. Factory configuration
    /// failures map to `HttpTransport` without reflecting their inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_ai_gateway_with_prepared_roots_and_conversation_and_credential_and_transport(
        loaded_config: LoadedNativeConfig,
        credential: DiscoveredAiGatewayCredential,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        conversation_options: NativeReferenceHostConversationOptions,
        make_transport: impl FnOnce(
            AiGatewayBearerToken,
        ) -> Result<
            (Arc<dyn AiGatewayTransport>, NetworkTarget),
            AiGatewayHttpConfigError,
        >,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::compose_production_prepared_with_credential(
            loaded_config,
            || Ok(credential),
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            conversation_options.into(),
            make_transport,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_production_prepared_with_credential(
        loaded_config: LoadedNativeConfig,
        acquire_credential: impl FnOnce() -> Result<
            DiscoveredAiGatewayCredential,
            NativeReferenceHostBuildError,
        >,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        options: PreparedCompositionOptions,
        make_transport: impl FnOnce(
            AiGatewayBearerToken,
        ) -> Result<
            (Arc<dyn AiGatewayTransport>, NetworkTarget),
            AiGatewayHttpConfigError,
        >,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_prepared_selections(&loaded_config, &options)?;
        let mcp_management = options.mcp_management.clone();
        let mcp_contexts = options
            .mcp_contexts
            .clone()
            .or_else(|| options.mcp_runtime.as_ref().map(|mcp| mcp.contexts.clone()));
        let mcp_options = mcp::Selection {
            options: options.mcp_runtime.clone(),
            management: mcp_management.clone(),
        };
        let skills = options.skills.clone();
        let background_url = options.background_url.clone();
        let undo_tracker = options.undo_tracker.clone();
        let model_routes = options.model_routes.clone();
        let observations = options.observations.clone();
        let permissions = options.permissions.clone();
        let managed = managed_host::select(&options)?;
        let (workspace_tools, session_store, selection) =
            consume_prepared_composition(prepared_roots, options)?;
        let memory = open_memory_tool(&session_store)?;
        let credential = acquire_credential()?;
        let credential_source = credential.source();
        let (transport, network_target) =
            make_transport(credential.into_bearer_token()).map_err(|_| {
                NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::HttpTransport)
            })?;
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
            Some(credential_source),
            Arc::new(EmptyMcpToolCatalog),
            Arc::new(EmptyMcpFeatureAuthority),
            Arc::new(EmptySubagentAuthority),
            selection,
            model_routes.clone(),
            observations.clone(),
            permissions,
            mcp_options,
            background_url,
            managed,
        )
        .map(|mut host| {
            host.mcp_management = mcp_management;
            host.mcp_contexts = mcp_contexts;
            host.skills = skills;
            host.undo_tracker = undo_tracker;
            host
        })
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
            None,
            None,
            None,
            mcp::Selection::default(),
            None,
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
            None,
            None,
            None,
            mcp::Selection::default(),
            None,
            None,
        )
    }

    /// Composes a reference host with an explicitly injected managed
    /// subagent authority and inert MCP authorities.
    ///
    /// The injected allocation is retained exactly and remains inert during
    /// construction. It must validate actual admitted-turn identity and own
    /// bounded durable child execution independently of the calling turn.
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
        subagent_authority: Arc<dyn ManagedSubagentAuthority>,
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
            None,
            None,
            None,
            mcp::Selection::default(),
            None,
            None,
        )
    }

    /// Composes a reference host with explicitly injected MCP and managed
    /// subagent authorities.
    ///
    /// Every injected allocation is retained exactly and remains inert during
    /// construction. The subagent authority validates actual admitted-turn
    /// identity and owns durable child execution and settlement.
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
        subagent_authority: Arc<dyn ManagedSubagentAuthority>,
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
            None,
            None,
            None,
            mcp::Selection::default(),
            None,
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
        Self::compose_injected_prepared(
            loaded_config,
            transport,
            network_target,
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            PreparedCompositionOptions::default(),
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
        Self::compose_injected_prepared(
            loaded_config,
            transport,
            network_target,
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            PreparedCompositionOptions {
                terminal: Some(terminal_options),
                ..Default::default()
            },
        )
    }

    /// Composes injected Gateway transport with explicitly authorized shared undo.
    ///
    /// This is the injected-transport counterpart of
    /// [`Self::compose_ai_gateway_http_with_prepared_roots_and_conversation`].
    /// It retains the same exact tracker and prepared-root authority, performs
    /// no credential discovery, and requires the supplied canonical target to
    /// identify the endpoint actually contacted by the injected transport.
    /// Optional terminal selection retains its existing blocking-worker and
    /// directory-preparation contract. No file snapshots or undo run here.
    ///
    /// # Errors
    /// Returns a redacted stage-only failure for unsupported selections, unsafe
    /// roots, or failure to construct the explicitly selected components.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        conversation_options: NativeReferenceHostConversationOptions,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        Self::compose_injected_prepared(
            loaded_config,
            transport,
            network_target,
            prepared_roots,
            permission_prompter,
            question_prompter,
            web_search_deadline,
            conversation_options.into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_injected_prepared(
        loaded_config: LoadedNativeConfig,
        transport: Arc<dyn AiGatewayTransport>,
        network_target: NetworkTarget,
        prepared_roots: PreparedNativeRoots,
        permission_prompter: Arc<dyn PermissionPrompter>,
        question_prompter: Arc<dyn QuestionPrompter>,
        web_search_deadline: Arc<dyn WebSearchDeadline>,
        options: PreparedCompositionOptions,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        validate_prepared_selections(&loaded_config, &options)?;
        let mcp_management = options.mcp_management.clone();
        let mcp_contexts = options
            .mcp_contexts
            .clone()
            .or_else(|| options.mcp_runtime.as_ref().map(|mcp| mcp.contexts.clone()));
        let mcp_options = mcp::Selection {
            options: options.mcp_runtime.clone(),
            management: mcp_management.clone(),
        };
        let skills = options.skills.clone();
        let background_url = options.background_url.clone();
        let undo_tracker = options.undo_tracker.clone();
        let model_routes = options.model_routes.clone();
        let observations = options.observations.clone();
        let permissions = options.permissions.clone();
        let managed = managed_host::select(&options)?;
        let (workspace_tools, session_store, selection) =
            consume_prepared_composition(prepared_roots, options)?;
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
            selection,
            model_routes.clone(),
            observations.clone(),
            permissions,
            mcp_options,
            background_url,
            managed,
        )
        .map(|mut host| {
            host.mcp_management = mcp_management;
            host.mcp_contexts = mcp_contexts;
            host.skills = skills;
            host.undo_tracker = undo_tracker;
            host
        })
    }

    /// Returns the composed provider-neutral engine.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.services.engine
    }

    /// Returns the canonical workspace association captured during composition.
    /// Tools retain descriptor authority; this label is not a fresh pathname or
    /// filesystem-identity check after an external rename or replacement.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Returns the exact optional skills allocation without discovery or effects.
    #[must_use]
    pub fn skills(&self) -> Option<Arc<crate::NativeSkillsService>> {
        self.skills.clone()
    }

    /// Returns the exact optional profile-management allocation without effects.
    #[must_use]
    pub fn mcp_management(
        &self,
    ) -> Option<Arc<crate::mcp::management::NativeMcpManagementService>> {
        self.mcp_management.clone()
    }

    /// Observes the exact selected router without registering or authorizing work.
    #[must_use]
    pub fn mcp_contexts(&self) -> Option<Arc<crate::mcp::context::NativeMcpContexts>> {
        self.mcp_contexts.clone()
    }

    /// Attaches the host's shared MCP router before the first admitted turn.
    /// Hosts without this selection leave the conversation unchanged.
    ///
    /// # Errors
    /// Rejects busy conversations, duplicate identities or exhausted route capacity.
    pub fn configure_conversation_mcp(
        &self,
        conversation: crate::NativeConversation,
    ) -> Result<crate::NativeConversation, crate::NativeConversationError> {
        let conversation = match &self.mcp_controller {
            Some(controller) => conversation.with_mcp_readiness(controller)?,
            None => conversation,
        };
        let conversation = match &self.mcp_ephemeral {
            Some(owner) => conversation.with_mcp_ephemeral_readiness(owner)?,
            None => conversation,
        };
        match &self.mcp_contexts {
            Some(contexts) => conversation.with_mcp_contexts(contexts),
            None => Ok(conversation),
        }
    }

    /// Attaches this host's exact native permission routes before admitting work.
    /// Legacy hosts leave the conversation unchanged. No prompt or file snapshot
    /// occurs; restored saved rules are validated, never restored as live grants.
    /// # Errors
    /// Rejects duplicate/busy registration or invalid saved permission metadata.
    pub fn configure_conversation_permissions(
        &self,
        conversation: crate::NativeConversation,
    ) -> Result<crate::NativeConversation, crate::NativeConversationError> {
        if self.services.permissions.is_none() {
            return Ok(conversation);
        }
        let policy =
            configured_permission_policy(self.loaded_config.config(), &self.workspace_root)?;
        self.configure_conversation_permissions_with_policy(conversation, policy)
    }

    /// Attaches the exact host routes with an explicitly selected policy.
    /// This is trusted host authority, not restoration of prior live grants.
    /// Saved rules are validated and the supplied policy replaces config defaults.
    ///
    /// # Errors
    /// Rejects hosts without native permission composition, duplicate/busy
    /// registration, or invalid saved permission metadata.
    pub fn configure_conversation_permissions_with_policy(
        &self,
        conversation: crate::NativeConversation,
        policy: crate::NativePermissionPolicySnapshot,
    ) -> Result<crate::NativeConversation, crate::NativeConversationError> {
        let controller = self
            .services
            .permissions
            .as_ref()
            .ok_or(crate::NativeConversationError::Engine)?;
        let contexts = self
            .services
            .permission_contexts
            .as_ref()
            .ok_or(crate::NativeConversationError::Engine)?;
        conversation
            .with_permission_controller(controller, policy)?
            .with_permission_contexts(contexts)
    }

    /// Observes this host's exact permission-context routes without enrolling
    /// another session or acquiring effects. Prepared ACP hosts use this to bind
    /// client presentation to the selected host, never a connection-global route.
    #[must_use]
    pub fn permission_contexts(&self) -> Option<Arc<crate::NativePermissionContexts>> {
        self.services.permission_contexts.as_ref().map(Arc::clone)
    }

    /// Observes settlement of this host's complete terminal workers, including
    /// transferred child cleanup, without keeping any Engine/Session alive.
    /// Legacy constructors return `None`. Retain this handle before dropping
    /// all real Engine/Session handles, then wait only on a blocking host thread.
    /// Waiting while a real host handle remains alive cannot complete.
    #[must_use]
    pub fn terminal_shutdown_completion(&self) -> Option<crate::NativeOwnedWorkerCompletion> {
        self.services.terminal_shutdown.clone()
    }

    /// Returns this complete terminal host's explicit session-lifecycle authority.
    /// Clones and unpolled requests do not keep the host alive. Legacy terminal
    /// composition returns `None`; this does not initialize a fallback supervisor.
    #[must_use]
    pub fn terminal_lifecycle_requester(&self) -> Option<crate::NativeTerminalLifecycleRequester> {
        self.services.terminal_lifecycle.clone()
    }

    /// Owner-scoped background observation/control over this complete terminal
    /// host. Retaining it does not keep backends alive or initialize a legacy host.
    #[must_use]
    pub fn terminal_background_requester(
        &self,
    ) -> Option<crate::NativeTerminalBackgroundRequester> {
        self.services.terminal_background.clone()
    }

    /// Returns the exact tracker injected into all five file mutation tools.
    /// Hosts constructed without conversation options return `None`; no tracker
    /// is manufactured, cleared, or granted new filesystem authority here.
    #[must_use]
    pub fn undo_tracker(&self) -> Option<Arc<FileUndoTracker>> {
        self.undo_tracker.clone()
    }

    /// Shares the actual terminal/archive completion owner; never creates a scope.
    pub(crate) fn control_workers(&self) -> Option<crate::NativeOwnedWorkerScope> {
        self.services.control_workers.clone()
    }

    /// Parses explicit slash arguments using this host's actual tool registry.
    /// No configuration, process, session or filesystem operation occurs.
    /// # Errors
    /// Rejects invalid grammar, unknown exact tool names and bounded-input overflow.
    pub fn parse_allowlist(
        &self,
        rest: &str,
    ) -> Result<crate::NativeAllowlistRequest, crate::NativeAllowlistParseError> {
        crate::allowlist::parse(rest, |name| self.allowlist_tool_registered(name))
    }

    pub(crate) fn allowlist_tool_registered(&self, name: &str) -> bool {
        ToolName::new(name).is_ok_and(|name| self.services.engine.tool(&name).is_some())
    }

    /// Creates an inert catalog reader over this exact store and canonical
    /// workspace, using the actual host-owned completion scope. No discovery or
    /// fallback worker scope occurs. Clones of the reader share scan admission.
    /// # Errors
    /// Legacy hosts without owned workers return `Unavailable`.
    pub fn session_catalog_reader(
        &self,
    ) -> Result<crate::NativeSessionCatalogReader, crate::NativeSessionCatalogReadError> {
        let workers = self
            .control_workers()
            .ok_or(crate::NativeSessionCatalogReadError::Unavailable)?;
        Ok(crate::NativeSessionCatalogReader::new(
            Arc::clone(&self.services.session_store),
            self.workspace_root.clone(),
            workers,
        ))
    }

    /// Returns the exact optional current-model registry injected into search.
    /// No registry or conversation registration is created by this accessor.
    #[must_use]
    pub fn model_routes(&self) -> Option<Arc<crate::NativeConversationModelRoutes>> {
        self.services.model_routes.clone()
    }

    /// Returns the exact optional observation registry injected into file tools.
    /// This does not inspect files, attach a conversation, or publish history.
    #[must_use]
    pub fn observations(&self) -> Option<Arc<crate::NativeConversationObservations>> {
        self.services.observations.clone()
    }

    /// Returns the concrete store shared exactly with the engine, result reader,
    /// and session lifecycle.
    #[must_use]
    pub fn session_store(&self) -> &Arc<FileSessionStore> {
        &self.services.session_store
    }

    /// Returns by-ID durable lifecycle operations over this host's engine and
    /// exact concrete store.
    #[must_use]
    pub fn session_lifecycle(&self) -> &NativeSessionLifecycle {
        &self.services.session_lifecycle
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
        self.services.engine.clone()
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
            None,
            None,
            None,
            mcp::Selection::default(),
            None,
            None,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "Linear fallible assembly keeps the construction cleanup guard alive until every resource transfers to the engine."
    )]
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
        subagent_authority: Arc<dyn ManagedSubagentAuthority>,
        terminal_selection: Option<TerminalCompositionSelection>,
        model_routes: Option<Arc<crate::NativeConversationModelRoutes>>,
        observations: Option<Arc<crate::NativeConversationObservations>>,
        permission_options: Option<NativeReferenceHostPermissionOptions>,
        mut mcp_options: mcp::Selection,
        background_url: Option<BackgroundUrlSelection>,
        managed: Option<managed_host::Selection>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        let workspace_binding = workspace_tools.workspace_binding.clone();
        let (workspace_tools, permission_setup) =
            permissions::install_workspace(workspace_tools, permission_options)?;
        let permission_contexts = permission_setup
            .as_ref()
            .map(|setup| Arc::clone(&setup.contexts));
        let model = loaded_config.config().model().to_owned();
        let (provider, engine_limits) = compose_selected_provider(
            model.clone(),
            Arc::clone(&transport),
            terminal_selection.is_some(),
        )?;
        let mut catalog = ReferenceHostToolCatalog::new(
            observations.clone(),
            engine_limits,
            permission_setup.is_some(),
        );
        let authority = catalog.workspace(
            workspace_tools,
            permission_setup.as_ref().map(|setup| &setup.registry),
        );
        let workspace_root = authority.canonical_workspace.clone();
        let SharedNetworkTools {
            vision,
            web_search,
            terminal_wait_delay,
        } = compose_network_tools(
            authority.vision_root,
            &model,
            &transport,
            network_target,
            web_search_deadline,
            model_routes.clone(),
        )?;
        // Declare before the resource: failed/unwound assembly drops every
        // actual owner before this observer joins the newly created scope.
        let mut construction = construction::Construction::default();
        let selected_terminal = compose_selected_terminal(
            authority.terminal_root,
            authority.canonical_workspace,
            authority.background_root,
            &session_store,
            terminal_wait_delay,
            terminal_selection,
            TerminalScopeSelection::new(permission_setup.as_ref(), workspace_binding.as_ref()),
        )?;
        construction.observe(selected_terminal.resource.as_ref());
        let managed = managed
            .map(|selection| {
                managed_host::ManagedHostAssembly::new(
                    selection,
                    selected_terminal
                        .archive
                        .clone()
                        .ok_or_else(managed_host::error)?,
                )
            })
            .transpose()?;
        if let Some(managed) = &managed {
            mcp_options.options.get_or_insert_with(|| {
                NativeReferenceHostMcpOptions::new(
                    Arc::new(crate::mcp::context::NativeMcpContexts::new()),
                    managed.clock.clone(),
                )
            });
        }
        let web_fetch = compose_web_fetch(selected_terminal.resource.as_ref())?;
        let background_opener = background_url.map(|selected| selected.bind(&selected_terminal));
        let mcp::Selected {
            composition: mcp,
            catalog: mcp_catalog,
            managed_seed: managed_mcp_seed,
        } = mcp::select(
            mcp_options,
            &selected_terminal,
            permission_setup.as_ref(),
            mcp_catalog,
            background_opener
                .as_ref()
                .and_then(|selected| selected.as_ref().ok())
                .map(crate::NativeBackgroundUrlOpener::mcp_launcher),
        )?;
        let SelectedTerminalComposition {
            tool: terminal,
            resource: host_resource,
            archive,
            concrete: terminal_concrete,
        } = selected_terminal;
        let session_store = Arc::new(session_store);
        let subagent_authority = match &managed {
            Some(managed) => managed.authority()?,
            None => subagent_authority,
        };
        let subagent_tool: Arc<dyn Tool> = match archive.as_ref() {
            Some(archive) => Arc::new(subagent::NativeManagedSubagentTool::new(
                subagent_authority,
                archive.clone(),
            )),
            None => Arc::new(SubagentTool::shared_authority(subagent_authority)),
        };
        let (engine_session_store, read_tool_result) = session_store_parts(&session_store, archive);
        catalog.question(AskUserQuestionTool::shared_prompter(question_prompter));
        let (mcp_catalog, features) = match &managed {
            Some(managed) => (
                Arc::new(managed.mcp.requester()) as Arc<dyn McpToolCatalog>,
                Arc::new(managed.mcp.features_tool()) as Arc<dyn Tool>,
            ),
            None => (
                mcp_catalog,
                mcp::features(mcp.as_ref(), mcp_feature_authority),
            ),
        };
        catalog.extensions(
            mcp_catalog,
            features,
            subagent_tool,
            managed.as_ref().map(|managed| managed.mcp.as_ref()),
        );
        catalog.add(memory, None);
        catalog.add(read_tool_result, None);
        catalog.terminal(terminal, terminal_concrete)?;
        catalog.vision(vision);
        catalog.add(web_fetch, None);
        catalog.add(web_search, None);
        let permissions = catalog.finish_permissions(
            permission_setup,
            host_resource.as_ref(),
            transport,
            Arc::clone(&permission_prompter),
            mcp.as_ref(),
            managed
                .as_ref()
                .map(|managed| managed.mcp.permission_preparer()),
        )?;
        let builder = Engine::builder()
            .limits(engine_limits)
            .provider(provider)
            .shared_session_store(engine_session_store);
        let builder = catalog.into_builder(
            builder,
            permissions
                .as_ref()
                .map(|permissions| permissions.controller.clone()),
            permission_prompter,
        );
        Self::from_composed_builder(
            builder,
            workspace_root,
            session_store,
            loaded_config,
            credential_source,
            host_resource,
            permissions,
            permission_contexts,
            mcp,
            managed_mcp_seed,
            model_routes,
            observations,
        )
        .map(|mut host| {
            host.workspace_binding = workspace_binding;
            host.background_opener = background_opener;
            host.managed = managed;
            construction.transfer();
            host
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn from_composed_builder(
        builder: machine_god_core::EngineBuilder,
        workspace_root: PathBuf,
        session_store: Arc<FileSessionStore>,
        loaded_config: LoadedNativeConfig,
        credential_source: Option<AiGatewayCredentialSource>,
        host_resource: Option<NativeTerminalHostResource>,
        permissions: Option<permissions::ComposedPermissions>,
        permission_contexts: Option<Arc<crate::NativePermissionContexts>>,
        mcp: Option<mcp::Composition>,
        managed_mcp_seed: Option<Arc<mcp::ManagedMcpSeed>>,
        model_routes: Option<Arc<crate::NativeConversationModelRoutes>>,
        observations: Option<Arc<crate::NativeConversationObservations>>,
    ) -> Result<Self, NativeReferenceHostBuildError> {
        let terminal_shutdown = host_resource
            .as_ref()
            .map(NativeTerminalHostResource::completion);
        let control_workers = host_resource
            .as_ref()
            .map(NativeTerminalHostResource::worker_scope);
        let terminal_lifecycle = host_resource
            .as_ref()
            .map(NativeTerminalHostResource::lifecycle_requester);
        let terminal_background = host_resource
            .as_ref()
            .map(NativeTerminalHostResource::background_requester);
        let reserved_tool_names: Box<[ToolName]> =
            builder.registered_tool_names().cloned().collect();
        let mcp_clock = mcp.as_ref().map(|composition| composition.clock.clone());
        let (mcp_runtime, mcp_controller, mcp_ephemeral) =
            mcp::controller(mcp, control_workers.as_ref(), &reserved_tool_names)?;
        let builder = match (host_resource, &mcp_runtime) {
            (Some(resource), Some(runtime)) => builder.host_resource(mcp::HostResource {
                mcp: runtime.clone(),
                controller: mcp_controller.clone(),
                ephemeral: mcp_ephemeral.clone(),
                _terminal: resource,
            }),
            (Some(resource), None) => builder.host_resource(resource),
            (None, None) => builder,
            (None, Some(_)) => return Err(mcp::error()),
        };
        let engine = builder.build().map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Engine)
        })?;
        let session_lifecycle =
            NativeSessionLifecycle::new(engine.clone(), Arc::clone(&session_store)).map_err(
                |_| NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Engine),
            )?;

        Ok(Self {
            services: Arc::new(NativeHostServices {
                engine,
                session_store,
                session_lifecycle,
                terminal_shutdown,
                control_workers,
                terminal_lifecycle,
                terminal_background,
                model_routes,
                observations,
                permission_preparation: permissions
                    .as_ref()
                    .map(|permissions| permissions.preparation.clone()),
                managed_mcp_seed,
                permissions: permissions.map(|permissions| permissions.controller),
                permission_contexts,
            }),
            background_opener: None,
            reserved_tool_names,
            mcp_runtime,
            mcp_controller,
            mcp_ephemeral,
            mcp_clock,
            mcp_management: None,
            mcp_contexts: None,
            skills: None,
            workspace_binding: None,
            workspace_root,
            loaded_config,
            credential_source,
            undo_tracker: None,
            managed: None,
        })
    }
}

struct SharedNetworkTools {
    vision: VisionTool,
    web_search: WebSearchTool,
    terminal_wait_delay: Arc<dyn TerminalBackgroundWaitDelay>,
}

fn compose_network_tools(
    vision_root: OwnedFd,
    model: &str,
    transport: &Arc<dyn AiGatewayTransport>,
    network_target: NetworkTarget,
    deadline: Arc<dyn WebSearchDeadline>,
    model_routes: Option<Arc<crate::NativeConversationModelRoutes>>,
) -> Result<SharedNetworkTools, NativeReferenceHostBuildError> {
    let vision_transport = AiGatewayVisionTransport::dedicated(Arc::clone(transport));
    let (vision_deadline, terminal_wait_delay) = compose_deadline_adapters(&deadline);
    let vision = VisionTool::from_root_descriptor(
        vision_root,
        network_target.clone(),
        Arc::new(vision_transport),
        vision_deadline,
        VisionLimits::default(),
    )
    .map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::VisionConfig)
    })?;
    let search_transport = AiGatewayWebSearchTransport::new(
        model.to_owned(),
        Arc::clone(transport),
    )
    .map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::WebSearchTransport)
    })?;
    let web_search = WebSearchTool::with_bounded_transport(
        network_target,
        Arc::new(search_transport),
        deadline,
        WebSearchLimits::default(),
    )
    .map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::WebSearchTransport)
    })?;
    let web_search = match model_routes {
        Some(routes) => web_search.with_model_routes(routes),
        None => web_search,
    };
    Ok(SharedNetworkTools {
        vision,
        web_search,
        terminal_wait_delay,
    })
}

fn compose_selected_provider(
    model: String,
    transport: Arc<dyn AiGatewayTransport>,
    full_terminal: bool,
) -> Result<(AiGatewayProvider, EngineLimits), NativeReferenceHostBuildError> {
    if full_terminal {
        compose_full_terminal_provider(model, transport)
    } else {
        compose_provider(model, transport)
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
            provider.with_tool_input_limits([
                (
                    ToolName::new(crate::TERMINAL_TOOL_NAME).expect("terminal tool name is valid"),
                    AiGatewayToolInputLimits {
                        max_argument_bytes: input_bytes,
                        max_json_nodes: input_nodes,
                    },
                ),
                (
                    ToolName::new(machine_god_core::SUBAGENT_TOOL_NAME)
                        .expect("subagent tool name is valid"),
                    AiGatewayToolInputLimits {
                        max_argument_bytes: machine_god_core::MAX_SUBAGENT_ARGUMENT_BYTES,
                        max_json_nodes: machine_god_core::MAX_SUBAGENT_JSON_NODES,
                    },
                ),
            ])
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

struct SelectedTerminalComposition {
    tool: Arc<dyn Tool>,
    concrete: Option<Arc<crate::TerminalActionTool>>,
    resource: Option<NativeTerminalHostResource>,
    archive: Option<Arc<NativeToolResultArchiveAdapter>>,
}

struct TerminalScopeSelection {
    permission: Option<Arc<crate::NativeTerminalPermissionPolicy>>,
    workspace_contexts: Option<Arc<crate::NativeWorkspaceContexts>>,
}

impl TerminalScopeSelection {
    fn new(
        permission: Option<&PermissionComposition>,
        workspace: Option<&WorkspaceBinding>,
    ) -> Self {
        Self {
            permission: permission.map(|setup| Arc::clone(&setup.sandbox)),
            workspace_contexts: workspace.map(|binding| Arc::clone(&binding.contexts)),
        }
    }
}

fn compose_selected_terminal(
    workspace: OwnedFd,
    workspace_path: PathBuf,
    background_root: OwnedFd,
    session_store: &FileSessionStore,
    wait_delay: Arc<dyn TerminalBackgroundWaitDelay>,
    selection: Option<TerminalCompositionSelection>,
    scope: TerminalScopeSelection,
) -> Result<SelectedTerminalComposition, NativeReferenceHostBuildError> {
    if let Some(selection) = selection {
        let full =
            compose_full_terminal(workspace, workspace_path, session_store, selection, scope)?;
        let tool = Arc::new(full.tool);
        Ok(SelectedTerminalComposition {
            tool: tool.clone(),
            concrete: Some(tool),
            resource: Some(full.resource),
            archive: Some(full.archive),
        })
    } else {
        let tool = compose_terminal(
            workspace,
            &workspace_path,
            background_root,
            session_store,
            wait_delay,
        )?;
        Ok(SelectedTerminalComposition {
            tool: Arc::new(tool),
            concrete: None,
            resource: None,
            archive: None,
        })
    }
}

fn compose_full_terminal(
    workspace: OwnedFd,
    workspace_path: PathBuf,
    session_store: &FileSessionStore,
    selection: TerminalCompositionSelection,
    scope: TerminalScopeSelection,
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
    let (tool, resource) = match (scope.workspace_contexts, scope.permission) {
        (Some(contexts), permission) => NativeTerminalHost::compose_with_workspace_on_worker(
            inputs,
            state_root,
            host_identity,
            contexts,
            permission,
        ),
        (None, Some(permission)) => NativeTerminalHost::compose_with_permission_on_worker(
            inputs,
            state_root,
            host_identity,
            permission,
        ),
        (None, None) => NativeTerminalHost::compose_on_worker(inputs, state_root, host_identity),
    }
    .map_err(|_| terminal_options_error())?;
    let archive = Arc::new(
        NativeToolResultArchiveAdapter::new(archive).with_worker_scope(resource.worker_scope()),
    );
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

fn compose_web_fetch(
    resource: Option<&NativeTerminalHostResource>,
) -> Result<WebFetchTool, NativeReferenceHostBuildError> {
    // Complete hosts already own the scope through finalization. Defer unused
    // system DNS discovery without creating another lifetime/cleanup owner.
    let tool = match resource {
        Some(resource) => WebFetchTool::with_owned_workers(&resource.worker_scope()),
        None => WebFetchTool::new(),
    };
    tool.map_err(|_| {
        NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::WebFetchTransport)
    })
}

fn session_store_parts(
    session_store: &Arc<FileSessionStore>,
    archive: Option<Arc<NativeToolResultArchiveAdapter>>,
) -> (Arc<dyn SessionStore>, ReadToolResultTool) {
    let erased = Arc::clone(session_store) as Arc<dyn SessionStore>;
    let reader = ReadToolResultTool::shared_session_store(Arc::clone(&erased));
    let reader = match archive {
        Some(archive) => reader.with_archive(archive),
        None => reader,
    };
    (erased, reader)
}

#[derive(Clone, Copy, Debug)]
struct EmptyMcpToolCatalog;

impl McpToolCatalog for EmptyMcpToolCatalog {
    fn snapshot_for_turn(
        &self,
        _context: ToolContext,
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
    fn call_for_turn(
        &self,
        _context: ToolContext,
        _request: McpFeatureRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpFeaturePayload, McpFeatureError>> {
        Box::pin(async { Err(McpFeatureError::new(McpFeatureErrorKind::Unavailable)) })
    }
}

#[derive(Clone, Copy, Debug)]
struct EmptySubagentAuthority;

impl ManagedSubagentAuthority for EmptySubagentAuthority {
    fn execute(
        &self,
        _request: ManagedSubagentInvocation,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ManagedSubagentResult, ManagedSubagentError>> {
        Box::pin(async { Err(ManagedSubagentError::Unavailable) })
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

fn production_ai_gateway_transport(
    token: AiGatewayBearerToken,
) -> Result<(Arc<dyn AiGatewayTransport>, NetworkTarget), AiGatewayHttpConfigError> {
    Ok((
        Arc::new(AiGatewayHttpTransport::new(token)?),
        production_ai_gateway_target(),
    ))
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
    validate_provider_selections(loaded_config)?;
    let config = loaded_config.config();
    if config.permission_mode() != PermissionMode::Ask
        || config.sandbox_mode() != crate::NativeSandboxMode::None
        || !config.permission_rules().rules().is_empty()
        || config.has_workspace_permission_rules()
    {
        return Err(NativeReferenceHostBuildError::new(
            NativeReferenceHostBuildErrorKind::UnsupportedSelection,
        ));
    }
    Ok(())
}

fn configured_permission_policy(
    config: &crate::NativeConfig,
    workspace: &Path,
) -> Result<crate::NativePermissionPolicySnapshot, crate::NativeConversationError> {
    Ok(crate::NativePermissionPolicySnapshot::new(
        config.permission_mode(),
        Arc::new(
            config
                .permission_sources(workspace)
                .map_err(|_| crate::NativeConversationError::Engine)?
                .effective()
                .clone(),
        ),
    )
    .with_sandbox_mode(config.sandbox_mode()))
}

fn validate_prepared_selections(
    loaded_config: &LoadedNativeConfig,
    options: &PreparedCompositionOptions,
) -> Result<(), NativeReferenceHostBuildError> {
    managed_host::validate(options)?;
    if let Some(mcp) = &options.mcp_runtime {
        mcp.validate_controller(options.mcp_management.is_some())?;
    }
    if let Some(mcp) = &options.mcp_runtime
        && (options.terminal.is_none()
            || options.permissions.is_none()
            || options
                .mcp_contexts
                .as_ref()
                .is_some_and(|contexts| !Arc::ptr_eq(contexts, &mcp.contexts)))
    {
        return Err(mcp::error());
    }
    if (options.skills.is_some()
        || options.mcp_management.is_some()
        || options.background_url.is_some())
        && options.terminal.is_none()
    {
        return Err(terminal_options_error());
    }
    if options.permissions.is_none() {
        return validate_selections(loaded_config);
    }
    if options.terminal.is_none() {
        return Err(permissions::error());
    }
    validate_provider_selections(loaded_config)
}

fn validate_provider_selections(
    loaded_config: &LoadedNativeConfig,
) -> Result<(), NativeReferenceHostBuildError> {
    let config = loaded_config.config();
    if config.credential_source() != NativeCredentialSourceKind::Environment
        || config.provider() != NativeProviderKind::VercelAiGateway
        || config.transport() != NativeTransportKind::AiGatewayHttp
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

fn consume_prepared_composition(
    prepared_roots: PreparedNativeRoots,
    options: PreparedCompositionOptions,
) -> Result<
    (
        WorkspaceTools,
        FileSessionStore,
        Option<TerminalCompositionSelection>,
    ),
    NativeReferenceHostBuildError,
> {
    let selection = options
        .terminal
        .map(|terminal| TerminalCompositionSelection {
            options: terminal,
            state_path: prepared_roots.state_root().to_owned(),
        });
    let (mut tools, store) = consume_prepared_roots(prepared_roots)?;
    if let Some(binding) = options.workspace_binding {
        let error =
            || NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::WorkspaceRoot);
        binding
            .authority
            .snapshot()
            .map_err(|_| error())?
            .validate_host_binding(
                &tools.terminal_root,
                &tools.canonical_workspace,
                &store.try_clone_root_descriptor().map_err(|_| error())?,
            )
            .map_err(|_| error())?;
        tools.read_file = tools
            .read_file
            .with_workspace_contexts(Arc::clone(&binding.contexts));
        tools.file_info = tools
            .file_info
            .with_workspace_contexts(Arc::clone(&binding.contexts));
        tools.open_file = tools
            .open_file
            .with_workspace_contexts(Arc::clone(&binding.contexts));
        tools.create_folder = tools
            .create_folder
            .with_workspace_contexts(Arc::clone(&binding.contexts));
        tools.list_files = tools
            .list_files
            .with_workspace_contexts(Arc::clone(&binding.contexts));
        tools.glob_files = tools
            .glob_files
            .with_workspace_contexts(Arc::clone(&binding.contexts));
        tools.grep_files = tools
            .grep_files
            .with_workspace_contexts(Arc::clone(&binding.contexts));
        tools.workspace_binding = Some(binding);
    }
    if let Some(tracker) = options.undo_tracker {
        tools.write_file = tools.write_file.with_undo_tracker(Arc::clone(&tracker));
        tools.edit_file = tools.edit_file.with_undo_tracker(Arc::clone(&tracker));
        tools.delete_file = tools.delete_file.with_undo_tracker(Arc::clone(&tracker));
        tools.rename_file = tools.rename_file.with_undo_tracker(Arc::clone(&tracker));
        tools.copy_file = tools.copy_file.with_undo_tracker(tracker);
    }
    Ok((tools, store, selection))
}

#[cfg(test)]
mod tests {
    #[test]
    fn mcp_context_options_retain_exact_router_without_effect_authority() {
        use super::*;
        let contexts = Arc::new(crate::mcp::context::NativeMcpContexts::new());
        let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
            .with_mcp_contexts(contexts.clone());
        assert!(Arc::ptr_eq(
            options.mcp_contexts.as_ref().unwrap(),
            &contexts
        ));
        let prepared: PreparedCompositionOptions = options.into();
        assert!(Arc::ptr_eq(
            prepared.mcp_contexts.as_ref().unwrap(),
            &contexts
        ));
        assert!(prepared.mcp_management.is_none());
        assert!(prepared.terminal.is_none());
        let config = LoadedNativeConfig::from_file(crate::NativeConfig::default());
        assert!(validate_prepared_selections(&config, &prepared).is_ok());
        assert!(PreparedCompositionOptions::default().mcp_contexts.is_none());
    }

    #[test]
    fn mcp_management_options_are_inert_shared_and_require_owned_cleanup() {
        use super::*;
        use crate::mcp::{management::NativeMcpManagementService, store::NativeMcpConfigStore};
        let service = Arc::new(NativeMcpManagementService::new(Arc::new(
            NativeMcpConfigStore::new("/unopened-mcp-profile".into()).unwrap(),
        )));
        let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
            .with_mcp_management(service.clone());
        assert!(Arc::ptr_eq(
            options.mcp_management.as_ref().unwrap(),
            &service
        ));
        let prepared: PreparedCompositionOptions = options.clone().into();
        assert!(Arc::ptr_eq(
            prepared.mcp_management.as_ref().unwrap(),
            &service
        ));
        let config = LoadedNativeConfig::from_file(crate::NativeConfig::default());
        assert_eq!(
            validate_prepared_selections(&config, &prepared)
                .unwrap_err()
                .kind(),
            NativeReferenceHostBuildErrorKind::TerminalConfig
        );
        let prepared = options
            .with_terminal(
                NativeReferenceHostTerminalOptions::new("/unopened-helper".into(), None, vec![])
                    .unwrap(),
            )
            .into();
        assert!(validate_prepared_selections(&config, &prepared).is_ok());
    }

    #[test]
    fn skills_options_are_inert_shared_and_require_owned_terminal_lifecycle() {
        use super::*;
        let service = Arc::new(crate::NativeSkillsService::new(
            Arc::new(crate::NativeSkillCatalog::new(vec![]).unwrap()),
            None,
        ));
        let options = NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
            .with_skills(service.clone());
        assert!(Arc::ptr_eq(options.skills.as_ref().unwrap(), &service));
        let prepared: PreparedCompositionOptions = options.clone().into();
        assert!(Arc::ptr_eq(prepared.skills.as_ref().unwrap(), &service));
        let config = LoadedNativeConfig::from_file(crate::NativeConfig::default());
        assert_eq!(
            validate_prepared_selections(&config, &prepared)
                .unwrap_err()
                .kind(),
            NativeReferenceHostBuildErrorKind::TerminalConfig
        );
        let prepared = options
            .with_terminal(
                NativeReferenceHostTerminalOptions::new("/unopened-helper".into(), None, vec![])
                    .unwrap(),
            )
            .into();
        assert!(validate_prepared_selections(&config, &prepared).is_ok());
    }

    #[test]
    fn allowlist_local_source_is_effective_and_never_ignored_without_composition() {
        let config = crate::NativeConfig::default();
        let workspace = std::path::Path::new("/workspace");
        let (config, _) = config
            .with_permission_mutation(
                workspace,
                crate::NativeConfiguredPermissionScope::User,
                &crate::NativeConfiguredPermissionMutation::Add {
                    permission: "read".into(),
                    pattern: "user/*".into(),
                },
            )
            .unwrap();
        let (config, _) = config
            .with_permission_mutation(
                workspace,
                crate::NativeConfiguredPermissionScope::Local,
                &crate::NativeConfiguredPermissionMutation::Add {
                    permission: "read".into(),
                    pattern: "local/*".into(),
                },
            )
            .unwrap();
        assert_eq!(
            super::configured_permission_policy(&config, workspace)
                .unwrap()
                .configured_rules()
                .rules()[0]
                .pattern(),
            "local/*"
        );
        assert_eq!(
            super::configured_permission_policy(&config, std::path::Path::new("/other"))
                .unwrap()
                .configured_rules()
                .rules()[0]
                .pattern(),
            "user/*"
        );
        let (config, _) = config
            .with_permission_mutation(
                workspace,
                crate::NativeConfiguredPermissionScope::Local,
                &crate::NativeConfiguredPermissionMutation::Remove {
                    permission: "read".into(),
                    pattern: "local/*".into(),
                },
            )
            .unwrap();
        assert!(
            super::configured_permission_policy(&config, workspace)
                .unwrap()
                .configured_rules()
                .rules()
                .is_empty()
        );
        assert!(super::validate_selections(&crate::LoadedNativeConfig::from_file(config)).is_err());
        let config = crate::NativeConfig::default();
        let (config, _) = config
            .with_permission_mutation(
                workspace,
                crate::NativeConfiguredPermissionScope::Local,
                &crate::NativeConfiguredPermissionMutation::Add {
                    permission: "read".into(),
                    pattern: "local/*".into(),
                },
            )
            .unwrap();
        let (config, _) = config
            .with_permission_mutation(
                workspace,
                crate::NativeConfiguredPermissionScope::Local,
                &crate::NativeConfiguredPermissionMutation::Remove {
                    permission: "read".into(),
                    pattern: "local/*".into(),
                },
            )
            .unwrap();
        assert!(config.permission_rules().rules().is_empty());
        assert!(
            super::validate_selections(&crate::LoadedNativeConfig::from_file(config)).is_err(),
            "explicit empty local scope still requires native composition"
        );
    }
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
