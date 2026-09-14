use super::{
    CapturedAcpLaunch, CapturedHostInputs, GatewayNetwork, McpSelection, TerminalCapture,
    check_cancelled,
};
use machine_god_core::{BoxFuture, CancellationToken};
use machine_god_native::{
    NativeInteractivePromptBridge, NativeInteractiveSessionOptions, NativeOwnedWorkerScope,
    NativePermissionContexts, NativeRootSelection, NativeSessionCatalogCursor,
    NativeSessionCatalogInvalidRecords, NativeSessionCatalogPage, NativeSessionCatalogQuery,
    NativeSessionCatalogReadError, WebSearchDeadline,
    acp::{
        interaction::NativeAcpElicitationPresenter,
        selection::{NativeAcpHostFactory, NativeAcpPreparedHost},
        session::AcpSessionError,
    },
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
            workers,
        }
    }
}
impl NativeAcpHostFactory for AcpHostFactory {
    fn prepare(
        &self,
        workspace: PathBuf,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        let environment = self.environment.clone();
        let terminal = self.terminal.clone();
        let network = self.network.clone();
        let handle = self.handle.clone();
        let deadline = self.deadline.clone();
        let bridge = self.bridge.clone();
        let presenter = self.presenter.clone();
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
                        },
                        || Ok(()),
                        true,
                        CapturedHostInputs {
                            environment: environment.to_vec(),
                            roots,
                            terminal,
                            mcp: McpSelection::Ephemeral,
                            permission_contexts: permission_contexts.clone(),
                            cancellation,
                            network,
                        },
                        handle,
                        deadline,
                    )?;
                    let host = Arc::new(prepared.host);
                    let completion = host.terminal_shutdown_completion().ok_or(())?;
                    let result = (|| {
                        let mut options = NativeInteractiveSessionOptions::new(
                            host.workspace_root().to_owned(),
                            host.loaded_config().config().model_preferences(),
                        )
                        .map_err(|_| ())?;
                        if let Some(catalog) = prepared.catalog {
                            options = options.with_catalog(catalog);
                        }
                        NativeAcpPreparedHost::new(
                            host.clone(),
                            options,
                            permission_contexts,
                            prepared.acp_workspace.ok_or(())?,
                        )
                        .map_err(|_| ())
                    })();
                    drop(host);
                    if result.is_err() {
                        completion.wait_on_worker().map_err(|_| ())?;
                    }
                    result
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
