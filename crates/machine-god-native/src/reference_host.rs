use std::error::Error;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use machine_god_core::{
    BoxFuture, CancellationToken, Engine, NetworkTarget, SessionStore, SubagentAuthority,
    SubagentAuthorityError, SubagentAuthorityErrorKind, SubagentOutcome, SubagentRequest,
    SubagentTool,
};
use rustix::fd::OwnedFd;

use crate::background_inspection::NativeBackgroundRecordInspector;
use crate::background_supervisor::LazyProductionBackgroundStarter;
use crate::workspace::{WorkspaceRoot, WorkspaceTools};
use crate::{
    AiGatewayCredentialEnvironment, AiGatewayCredentialSource, AiGatewayHttpTransport,
    AiGatewayProvider, AiGatewayTransport, AiGatewayVisionTransport, AiGatewayWebSearchTransport,
    AskPermissionHandler, AskUserQuestionTool, FileSessionStore, LoadedNativeConfig,
    McpFeatureAuthority, McpFeatureError, McpFeatureErrorKind, McpFeaturePayload,
    McpFeatureRequest, McpFeaturesTool, McpSearchToolsTool, McpSelectTool, McpToolCatalog,
    McpToolCatalogError, McpToolCatalogSnapshot, MemoryTool, NativeCredentialSourceKind,
    NativeProviderKind, NativeSessionLifecycle, NativeTransportKind, PermissionMode,
    PermissionPrompter, PreparedNativeRoots, QuestionPrompter, ReadToolResultTool,
    TerminalBackgroundCatalog, TerminalBackgroundInspector, TerminalBackgroundStarter,
    TerminalBackgroundWaitDelay, TerminalBackgroundWaitDelayError, TerminalTool, VisionDeadline,
    VisionLimits, VisionTool, VisionTransportError, VisionTransportErrorKind, WebFetchTool,
    WebSearchDeadline, WebSearchLimits, WebSearchTool, WebSearchTransportErrorKind,
    discover_ai_gateway_credential,
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
        let provider = AiGatewayProvider::new(model, transport).map_err(|_| {
            NativeReferenceHostBuildError::new(NativeReferenceHostBuildErrorKind::Provider)
        })?;
        let web_fetch = compose_web_fetch()?;
        let permission_handler = AskPermissionHandler::shared_prompter(permission_prompter);
        let ask_user_question = AskUserQuestionTool::shared_prompter(question_prompter);
        let terminal = compose_terminal(
            workspace_tools.terminal_root,
            &workspace_tools.canonical_workspace,
            workspace_tools.background_root,
            &session_store,
            terminal_wait_delay,
        )?;
        let session_store = Arc::new(session_store);
        let (engine_session_store, read_tool_result) = session_store_components(&session_store);
        let engine = Engine::builder()
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
            .tool(terminal)
            .tool(vision)
            .tool(web_fetch)
            .tool(web_search)
            .tool(workspace_tools.write_file)
            .build()
            .map_err(|_| {
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
    let background: Arc<dyn TerminalBackgroundStarter> = background;
    let catalog: Arc<dyn TerminalBackgroundCatalog> = inspector.clone();
    let inspector: Arc<dyn TerminalBackgroundInspector> = inspector;
    terminal
        .with_background(canonical_workspace, environment, background)
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
        NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind, map_vision_deadline_error,
    };
    use crate::{VisionTransportErrorKind, WebSearchTransportError, WebSearchTransportErrorKind};

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
