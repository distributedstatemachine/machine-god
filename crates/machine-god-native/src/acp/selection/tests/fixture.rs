use super::*;
use crate::mcp::{
    context::NativeMcpContexts, lifetime::McpPeerLifetime, runtime::NativeMcpRuntimeClock,
};
use crate::*;
use std::{fs, os::unix::fs::DirBuilderExt, time::Instant};

pub(super) struct Factory {
    directory: PathBuf,
    pub workspace: PathBuf,
    environment: NativeEnvironment,
    pub preparations: AtomicUsize,
    pub wait: Arc<AtomicBool>,
    pub cancel_observed: Arc<AtomicBool>,
    pub provider_started: Arc<AtomicBool>,
}
impl Factory {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mg-acp-selection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let directory = directory.canonicalize().unwrap();
        let workspace = directory.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let state = directory.join("state");
        fs::DirBuilder::new().mode(0o700).create(&state).unwrap();
        Self {
            directory,
            workspace,
            environment: NativeEnvironment::new(None, Some(state.into_os_string()), None),
            preparations: AtomicUsize::new(0),
            wait: Arc::new(AtomicBool::new(false)),
            cancel_observed: Arc::new(AtomicBool::new(false)),
            provider_started: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Drop for Factory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}
impl NativeAcpHostFactory for Factory {
    fn prepare(
        &self,
        workspace: PathBuf,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        self.preparations.fetch_add(1, Ordering::Relaxed);
        let environment = self.environment.clone();
        let wait = self.wait.clone();
        let observed = self.cancel_observed.clone();
        let provider_started = self.provider_started.clone();
        Box::pin(async move {
            if wait.load(Ordering::Acquire) {
                cancellation.cancelled().await;
                observed.store(true, Ordering::Release);
                return Err(AcpSessionError::Cancelled);
            }
            let roots = PreparedNativeRoots::prepare(
                NativeRootSelection::from_environment(&environment, &workspace).unwrap(),
            )
            .unwrap();
            let open = |path: &std::path::Path| std::fs::File::open(path).unwrap().into();
            let authority = NativeWorkspaceAuthority::open_blocking(
                open(roots.workspace_root()),
                roots.workspace_root().to_path_buf(),
                Some(open(roots.state_root())),
                roots.state_root().to_path_buf(),
                vec![],
                false,
            )
            .unwrap();
            let contexts = Arc::new(NativePermissionContexts::new());
            let clock = Arc::new(Clock);
            let mcp = NativeReferenceHostMcpOptions::new(
                Arc::new(NativeMcpContexts::new()),
                clock.clone(),
            )
            .with_ephemeral_startup(NativeReferenceHostMcpEphemeralStartupOptions {
                captured_environment: vec![],
                stdio: None,
                clock,
                catalog_epoch: Instant::now(),
                owner_cancellation: CancellationToken::new(),
                #[cfg(feature = "mcp-http")]
                network: None,
                peer_lifetime: McpPeerLifetime::OwnerControlled,
                max_retained_bytes: 1024 * 1024,
                max_retained_generations: 4,
            })
            .unwrap();
            let options =
                NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
                    .with_workspace(authority, Arc::new(NativeWorkspaceContexts::new()))
                    .with_model_routes(Arc::new(NativeConversationModelRoutes::new()))
                    .with_observations(Arc::new(NativeConversationObservations::new()))
                    .with_permissions(NativeReferenceHostPermissionOptions::new(
                        contexts.clone(),
                        Arc::new(TokioPermissionReviewClock),
                    ))
                    .with_terminal(
                        NativeReferenceHostTerminalOptions::new(
                            "/no-process-helper-selected".into(),
                            Some("/bin/bash".into()),
                            vec![],
                        )
                        .unwrap(),
                    )
                    .with_mcp_runtime(mcp);
            let config=crate::config::parse_config_bytes(br#"{"schema_version":5,"permission_mode":"ask","sandbox_mode":"none","permission_rules":[],"provider":"vercel_ai_gateway","transport":"ai_gateway_http","credential_source":"environment","model":"fixture/main","effort":"auto","fast_mode":false}"#).unwrap();
            let defaults = config.model_preferences();
            let host=Arc::new(NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(LoadedNativeConfig::from_file(config),Arc::new(Transport(provider_started)),
                machine_god_core::NetworkTarget{scheme:"https".into(),host:"ai-gateway.vercel.sh".into(),port:None},roots,Arc::new(Prompter),Arc::new(Prompter),Arc::new(Deadline),options).unwrap());
            NativeAcpPreparedHost::new(
                host,
                NativeInteractiveSessionOptions::new(workspace, defaults).unwrap(),
                contexts,
            )
        })
    }
    fn list(
        &self,
        workspace: Option<PathBuf>,
        cursor: Option<NativeSessionCatalogCursor>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        let environment = self.environment.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(NativeSessionCatalogReadError::Cancelled);
            }
            let _ = (environment, workspace, cursor);
            Err(NativeSessionCatalogReadError::Unavailable)
        })
    }
}
struct Clock;
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
struct Deadline;
impl WebSearchDeadline for Deadline {
    fn wait_until(&self, _: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(std::future::pending())
    }
}
struct Prompter;
impl PermissionPrompter for Prompter {
    fn prompt(
        &self,
        _: machine_god_core::PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async { Ok(PermissionPromptDecision::Deny) })
    }
}
impl QuestionPrompter for Prompter {
    fn prompt(
        &self,
        _: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        Box::pin(std::future::pending())
    }
}
struct Transport(Arc<AtomicBool>);
impl AiGatewayTransport for Transport {
    fn stream(
        &self,
        _: AiGatewayTransportRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
        Box::pin(async move {
            self.0.store(true, Ordering::Release);
            let stream = futures_util::stream::once(async move {
                cancellation.cancelled().await;
                Err(machine_god_core::ProviderError::new(
                    machine_god_core::ProviderErrorKind::Cancelled,
                    "cancelled",
                    "cancelled",
                    false,
                ))
            });
            Ok(Box::pin(stream) as AiGatewayByteStream)
        })
    }
}
