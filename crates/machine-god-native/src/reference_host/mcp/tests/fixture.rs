use super::*;
use machine_god_core::{PermissionRequest, ProviderError};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};

pub(in crate::reference_host) struct Directory(pub PathBuf);
impl Directory {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mg-mcp-host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[derive(Default)]
pub(in crate::reference_host) struct Clock(pub AtomicUsize);
impl NativeMcpRuntimeClock for Clock {
    fn now(&self) -> Instant {
        self.0.fetch_add(1, Ordering::Relaxed);
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
impl NativePermissionReviewClock for Clock {
    fn now(&self) -> Instant {
        NativeMcpRuntimeClock::now(self)
    }
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
impl WebSearchDeadline for Clock {
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
            Ok(())
        })
    }
}
#[derive(Default)]
pub(in crate::reference_host) struct Prompt {
    pub calls: AtomicUsize,
    pub deny: std::sync::atomic::AtomicBool,
}
impl PermissionPrompter for Prompt {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(if self.deny.load(Ordering::Relaxed) {
                PermissionPromptDecision::Deny
            } else {
                PermissionPromptDecision::AllowOnce
            })
        })
    }
}
impl QuestionPrompter for Prompt {
    fn prompt(
        &self,
        _: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        Box::pin(async { panic!("unexpected human question") })
    }
}
#[derive(Default)]
pub(in crate::reference_host) struct Transport {
    pub responses: Mutex<VecDeque<Vec<u8>>>,
    pub model_responses: Mutex<HashMap<String, VecDeque<Vec<u8>>>>,
    pub requests: Mutex<Vec<Value>>,
    pub reviews: AtomicUsize,
}
impl AiGatewayTransport for Transport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, ProviderError>> {
        Box::pin(async move {
            let model = request
                .headers()
                .iter()
                .find(|header| header.name() == "ai-language-model-id")
                .map(|header| header.value().to_owned());
            let request = machine_god_core::json::from_slice(request.body()).unwrap();
            let review = request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "permission_decision");
            self.requests.lock().unwrap().push(request);
            let bytes = if review {
                self.reviews.fetch_add(1, Ordering::Relaxed);
                call(
                    "assessment",
                    "permission_decision",
                    &json!({"risk":"low","authorization":"unknown","decision":"allow","rationale":"Requested development task."}),
                )
            } else {
                model
                    .as_ref()
                    .and_then(|model| {
                        self.model_responses
                            .lock()
                            .unwrap()
                            .get_mut(model)
                            .and_then(VecDeque::pop_front)
                    })
                    .or_else(|| self.responses.lock().unwrap().pop_front())
                    .unwrap_or_else(answer)
            };
            Ok(Box::pin(futures_util::stream::iter([Ok(bytes)])) as AiGatewayByteStream)
        })
    }
}
pub(super) fn call(id: &str, name: &str, input: &Value) -> Vec<u8> {
    let call = json!({"type":"tool-call","toolCallId":id,"toolName":name,"input":input});
    format!("data: {call}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"tool-calls\"}}}}\n\n").into_bytes()
}
pub(super) fn answer() -> Vec<u8> {
    b"data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"complete\"}\n\ndata: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n".to_vec()
}
pub(super) fn terminal() -> NativeReferenceHostTerminalOptions {
    NativeReferenceHostTerminalOptions::new(
        "/explicit-unexecuted-machine-god-helper".into(),
        Some("/bin/bash".into()),
        vec![],
    )
    .unwrap()
}
pub(super) fn permission(clock: Arc<Clock>) -> NativeReferenceHostPermissionOptions {
    NativeReferenceHostPermissionOptions::new(Arc::new(NativePermissionContexts::new()), clock)
}
pub(in crate::reference_host) struct Fixture {
    pub host: Option<NativeReferenceHost>,
    pub transport: Arc<Transport>,
    pub prompt: Arc<Prompt>,
    pub contexts: Arc<NativeMcpContexts>,
    pub clock: Arc<Clock>,
    pub workspace: PathBuf,
    pub state: PathBuf,
    _directory: Directory,
}
impl Fixture {
    pub fn new(mode: &str, enabled: bool) -> Self {
        Self::with_options(mode, enabled, |options, _, _| options)
    }

    pub fn with_options(
        mode: &str,
        enabled: bool,
        select: impl FnOnce(
            NativeReferenceHostConversationOptions,
            &Directory,
            Arc<Clock>,
        ) -> NativeReferenceHostConversationOptions,
    ) -> Self {
        Self::with_options_and_bridge(mode, enabled, None, select)
    }

    pub fn with_options_and_bridge(
        mode: &str,
        enabled: bool,
        bridge: Option<Arc<crate::NativeInteractivePromptBridge>>,
        select: impl FnOnce(
            NativeReferenceHostConversationOptions,
            &Directory,
            Arc<Clock>,
        ) -> NativeReferenceHostConversationOptions,
    ) -> Self {
        let directory = Directory::new();
        let workspace = directory.0.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let state = directory.0.join("state");
        fs::DirBuilder::new().mode(0o700).create(&state).unwrap();
        let environment = NativeEnvironment::new(None, Some(state.into_os_string()), None);
        let roots = PreparedNativeRoots::prepare(
            NativeRootSelection::from_environment(&environment, &workspace).unwrap(),
        )
        .unwrap();
        let state = roots.state_root().to_owned();
        let contexts = Arc::new(NativeMcpContexts::new());
        let clock = Arc::new(Clock::default());
        let mut options =
            NativeReferenceHostConversationOptions::new(Arc::new(FileUndoTracker::new()))
                .with_terminal(terminal())
                .with_permissions(permission(clock.clone()));
        if enabled {
            options = options.with_mcp_runtime(NativeReferenceHostMcpOptions::new(
                contexts.clone(),
                clock.clone(),
            ));
        } else {
            options = options.with_mcp_contexts(contexts.clone());
        }
        let options = select(options, &directory, clock.clone());
        let transport = Arc::new(Transport::default());
        let prompt = Arc::new(Prompt::default());
        let (permission, question): (Arc<dyn PermissionPrompter>, Arc<dyn QuestionPrompter>) =
            match bridge {
                Some(bridge) => (bridge.clone(), bridge),
                None => (prompt.clone(), prompt.clone()),
            };
        let config = crate::config::parse_config_bytes(format!(r#"{{"schema_version":5,"permission_mode":"{mode}","sandbox_mode":"none","permission_rules":[],"provider":"vercel_ai_gateway","transport":"ai_gateway_http","credential_source":"environment","model":"fixture/main","effort":"auto","fast_mode":false}}"#).as_bytes()).unwrap();
        let host = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            LoadedNativeConfig::from_file(config), transport.clone(), machine_god_core::NetworkTarget { scheme: "https".into(), host: "ai-gateway.vercel.sh".into(), port: None },
            roots, permission, question, clock.clone(), options,
        ).unwrap();
        Self {
            host: Some(host),
            transport,
            prompt,
            contexts,
            clock,
            workspace,
            state,
            _directory: directory,
        }
    }
    pub fn host(&self) -> &NativeReferenceHost {
        self.host.as_ref().unwrap()
    }
    pub async fn conversation(&self) -> NativeConversation {
        let host = self.host();
        let conversation = NativeConversation::create(
            host.session_lifecycle(),
            NativeSessionMetadata::new(&self.workspace, 1, NativeSessionOrigin::Cli).unwrap(),
        )
        .await
        .unwrap();
        host.configure_conversation_mcp(
            host.configure_conversation_permissions(conversation)
                .unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            let completion = host.terminal_shutdown_completion().unwrap();
            host.close_mcp();
            drop(host);
            completion.wait_on_worker().unwrap();
        }
    }
}
pub(super) fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(15), future)
                .await
                .expect("host fixture must settle");
        });
}
pub(super) async fn collect(runtime: &NativeConversationRuntime) -> Vec<TurnEvent> {
    runtime
        .enqueue("perform the requested development operation".into())
        .unwrap();
    let turn = runtime.start_next(10).await.unwrap().unwrap();
    turn.map(|event| event.unwrap().payload).collect().await
}
