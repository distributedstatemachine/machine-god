use super::{
    CapturedAcpLaunch, CapturedHostInputs, GatewayNetwork, McpSelection, TerminalCapture,
    check_cancelled,
};
use machine_god_core::{BoxFuture, CancellationToken};
use machine_god_native::{
    NativeInteractivePromptBridge, NativeInteractiveSessionOptions, NativeOwnedWorkerScope,
    NativePermissionContexts, NativeReferenceHost, NativeReferenceHostManagedOptions,
    NativeReferenceHostMcpOptions, NativeRootSelection, NativeSessionCatalogCursor,
    NativeSessionCatalogInvalidRecords, NativeSessionCatalogPage, NativeSessionCatalogQuery,
    NativeSessionCatalogReadError, NativeSessionOrigin, PreparedNativeRoots, WebSearchDeadline,
    acp::{
        interaction::NativeAcpElicitationPresenter,
        selection::{NativeAcpHostFactory, NativeAcpHostReuse, NativeAcpPreparedHost},
        session::AcpSessionError,
    },
    mcp::ephemeral::NativeMcpNetworkRequirement,
};
use std::{ffi::OsString, fmt, path::PathBuf, sync::Arc};

/// Captured native launch authorities, never ACP session/product state.
pub(in crate::ask::production) struct AcpHostFactory {
    environment: Arc<[(OsString, OsString)]>,
    terminal: TerminalCapture,
    network: GatewayNetwork,
    handle: tokio::runtime::Handle,
    deadline: Arc<dyn WebSearchDeadline>,
    bridge: Arc<NativeInteractivePromptBridge>,
    presenter: Arc<NativeAcpElicitationPresenter>,
    managed: NativeReferenceHostManagedOptions,
    workers: NativeOwnedWorkerScope,
}
impl fmt::Debug for AcpHostFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AcpHostFactory { <redacted> }")
    }
}
impl AcpHostFactory {
    /// Retains bounded explicit launch authority without preparing a host.
    pub(in crate::ask::production) fn new(
        captured: CapturedAcpLaunch,
        handle: tokio::runtime::Handle,
        deadline: Arc<dyn WebSearchDeadline>,
        bridge: Arc<NativeInteractivePromptBridge>,
        presenter: Arc<NativeAcpElicitationPresenter>,
        managed: NativeReferenceHostManagedOptions,
        workers: NativeOwnedWorkerScope,
    ) -> Self {
        Self {
            environment: captured.environment,
            terminal: captured.terminal,
            network: captured.network,
            handle,
            deadline,
            bridge,
            presenter,
            managed,
            workers,
        }
    }
}
impl NativeAcpHostFactory for AcpHostFactory {
    fn prepare_reuse(
        &self,
        current: Arc<NativeReferenceHost>,
        workspace: PathBuf,
        network: NativeMcpNetworkRequirement,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<Option<NativeAcpHostReuse>, AcpSessionError>> {
        let environment = super::super::skills_startup::environment(&self.environment);
        let workers = self.workers.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(AcpSessionError::Cancelled);
            }
            workers
                .run(move || {
                    if cancellation.is_cancelled() {
                        return Err(AcpSessionError::Cancelled);
                    }
                    let selected = NativeRootSelection::from_environment(&environment, &workspace)
                        .map_err(|_| AcpSessionError::InvalidConfiguration)?;
                    let roots = PreparedNativeRoots::prepare(selected)
                        .map_err(|_| AcpSessionError::Unavailable)?;
                    let Some(reuse) = NativeAcpHostReuse::capture(&current, &roots)? else {
                        return Ok(None);
                    };
                    if cancellation.is_cancelled() {
                        return Err(AcpSessionError::Cancelled);
                    }
                    let network = NativeReferenceHostMcpOptions::capture_ephemeral_network(network)
                        .map_err(|_| AcpSessionError::Unavailable)?;
                    if cancellation.is_cancelled() {
                        return Err(AcpSessionError::Cancelled);
                    }
                    Ok(Some(reuse.with_network(network)))
                })
                .await
                .map_err(|_| AcpSessionError::Unavailable)?
        })
    }

    fn prepare(
        &self,
        workspace: PathBuf,
        mcp_network: NativeMcpNetworkRequirement,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        let environment = self.environment.clone();
        let terminal = self.terminal.clone();
        let network = self.network.clone();
        let handle = self.handle.clone();
        let deadline = self.deadline.clone();
        let bridge = self.bridge.clone();
        let presenter = self.presenter.clone();
        let managed = self.managed.clone();
        let workers = self.workers.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(AcpSessionError::Closed);
            }
            workers
                .run(move || {
                    check_cancelled(&cancellation)?;
                    let roots = NativeRootSelection::from_environment(
                        &super::super::skills_startup::environment(&environment),
                        &workspace,
                    )
                    .map_err(|_| ())?;
                    let permission_contexts = Arc::new(NativePermissionContexts::new());
                    let prepared = super::super::prepare_conversation_host_captured(
                        &crate::workspace::launch::LaunchWorkspaceOptions::EMPTY,
                        super::super::ConversationAdapters {
                            permission: bridge.clone(),
                            question: bridge,
                            mcp: Some(presenter),
                            background_url: None,
                            managed: Some(managed),
                            catalog_loading: super::super::CatalogLoading::Eager,
                        },
                        || Ok(()),
                        true,
                        CapturedHostInputs {
                            environment: environment.to_vec(),
                            roots,
                            terminal,
                            mcp: McpSelection::Ephemeral(mcp_network),
                            permission_contexts: permission_contexts.clone(),
                            cancellation,
                            network,
                        },
                        handle,
                        deadline,
                    )?;
                    prepare_owner(prepared, permission_contexts)
                })
                .await
                .map_err(|_| AcpSessionError::Unavailable)?
                .map_err(|()| AcpSessionError::Unavailable)
        })
    }

    fn list(
        &self,
        workspace: Option<PathBuf>,
        cursor: Option<NativeSessionCatalogCursor>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        let environment = super::super::skills_startup::environment(&self.environment);
        let workers = self.workers.clone();
        let handle = self.handle.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(NativeSessionCatalogReadError::Cancelled);
            }
            workers
                .run(move || {
                    if cancellation.is_cancelled() {
                        return Err(NativeSessionCatalogReadError::Cancelled);
                    }
                    let mut query = NativeSessionCatalogQuery::new(100)
                        .map_err(NativeSessionCatalogReadError::Catalog)?
                        .with_invalid_records(NativeSessionCatalogInvalidRecords::SkipAndReport);
                    if let Some(workspace) = workspace {
                        query = query
                            .with_workspace(&workspace)
                            .map_err(NativeSessionCatalogReadError::Catalog)?;
                    }
                    if let Some(cursor) = cursor {
                        query = query.with_continuation(cursor);
                    }
                    let query = handle
                        .block_on(query.resolve_workspace_alias())
                        .map_err(NativeSessionCatalogReadError::Catalog)?;
                    if cancellation.is_cancelled() {
                        return Err(NativeSessionCatalogReadError::Cancelled);
                    }
                    let page = handle
                        .block_on(machine_god_native::list_native_session_catalog(
                            environment,
                            query,
                        ))
                        .map_err(NativeSessionCatalogReadError::Catalog)?;
                    if cancellation.is_cancelled() {
                        Err(NativeSessionCatalogReadError::Cancelled)
                    } else {
                        Ok(page)
                    }
                })
                .await
                .map_err(|_| NativeSessionCatalogReadError::Unavailable)?
        })
    }
}

// Runs on the existing owned preparation worker. Journal acquisition uses the
// descriptor retained during root preparation, not a second pathname lookup.
fn prepare_owner(
    prepared: super::super::PreparedConversationHost<tokio::runtime::Handle>,
    permission_contexts: Arc<NativePermissionContexts>,
) -> Result<NativeAcpPreparedHost, ()> {
    let completion = prepared.host.terminal_shutdown_completion().ok_or(())?;
    let result = (move || {
        let mut host = prepared.host;
        let preferences = host.loaded_config().config().model_preferences();
        let mut options = NativeInteractiveSessionOptions::new(
            host.workspace_root().to_owned(),
            preferences.clone(),
        )
        .map_err(|_| ())?;
        if let Some(catalog) = prepared.catalog {
            options = options.with_catalog(catalog);
        }
        let identity = prepared.acp_workspace.ok_or(())?;
        let state = prepared.acp_state.ok_or(())?;
        let agents = prepared
            .runtime
            .block_on(host.open_workspace_managed_agents(
                state,
                preferences,
                NativeSessionOrigin::Acp,
            ))
            .map_err(|_| ())?;
        let host = Arc::new(host);
        match NativeAcpPreparedHost::new_managed(
            host,
            options,
            permission_contexts,
            identity,
            agents,
        ) {
            Ok(prepared) => Ok(prepared),
            Err((_, mut agents)) => {
                agents.request_shutdown();
                let _ = prepared.runtime.block_on(std::future::poll_fn(|cx| {
                    agents.poll_shutdown(cx, super::super::wall_clock_ms().unwrap_or(0))
                }));
                Err(())
            }
        }
    })();
    if result.is_err() {
        completion.wait_on_worker().map_err(|_| ())?;
    }
    result
}
