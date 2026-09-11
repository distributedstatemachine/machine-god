use std::collections::VecDeque;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::native::{
    AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest, FileUndoTracker,
    NativeConversationModelRoutes, NativeConversationObservations, NativeEnvironment,
    NativePermissionContexts, NativeReferenceHost, NativeReferenceHostConversationOptions,
    NativeReferenceHostPermissionOptions, NativeReferenceHostTerminalOptions, NativeRootSelection,
    PermissionPromptDecision, PermissionPromptError, PermissionPrompter, PreparedNativeRoots,
    QuestionPromptError, QuestionPromptOutcome, QuestionPromptRequest, QuestionPrompter,
    TokioPermissionReviewClock, WebSearchDeadline, WebSearchTransportError, load_native_config,
};
use futures_util::stream;
use machine_god_core::{BoxFuture, CancellationToken, NetworkTarget, PermissionRequest};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-interactive-owner-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("temporary directory creation failed: {error}"),
            }
        }
        panic!("temporary directory allocation exhausted");
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.0);
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

#[derive(Default)]
struct TransportState {
    responses: VecDeque<Vec<u8>>,
    requests: Vec<Value>,
}

#[derive(Clone, Default)]
pub struct ScriptedTransport(Arc<Mutex<TransportState>>);

impl ScriptedTransport {
    pub fn push(&self, response: Vec<u8>) {
        self.0.lock().unwrap().responses.push_back(response);
    }

    pub fn requests(&self) -> Vec<Value> {
        self.0.lock().unwrap().requests.clone()
    }
}

impl AiGatewayTransport for ScriptedTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
        Box::pin(async move {
            let (_, body) = request.into_parts();
            let response = {
                let mut state = self.0.lock().unwrap();
                state.requests.push(serde_json::from_slice(&body).unwrap());
                state.responses.pop_front().expect("scripted response")
            };
            Ok(Box::pin(stream::iter([Ok(response)])) as AiGatewayByteStream)
        })
    }
}

struct AllowPrompter;

impl PermissionPrompter for AllowPrompter {
    fn prompt(
        &self,
        _request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async { Ok(PermissionPromptDecision::AllowOnce) })
    }
}

struct NoQuestions;

impl QuestionPrompter for NoQuestions {
    fn prompt(
        &self,
        _request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        panic!("interactive owner fixture did not script a question")
    }
}

struct NeverDeadline;

impl WebSearchDeadline for NeverDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(std::future::pending())
    }
}

pub struct Fixture {
    pub host: Arc<NativeReferenceHost>,
    pub workspace: PathBuf,
    pub transport: ScriptedTransport,
    pub undo: Arc<FileUndoTracker>,
    pub routes: Arc<NativeConversationModelRoutes>,
    pub observations: Arc<NativeConversationObservations>,
    state_root: PathBuf,
    _temporary: TemporaryDirectory,
}

impl Fixture {
    pub fn new() -> Self {
        Self::with_prompter(Arc::new(AllowPrompter))
    }

    pub fn with_prompter(prompter: Arc<dyn PermissionPrompter>) -> Self {
        Self::configured(false, false, prompter)
    }

    pub fn new_with_workspace() -> Self {
        Self::configured(true, false, Arc::new(AllowPrompter))
    }

    pub fn new_with_skills() -> Self {
        Self::configured(false, true, Arc::new(AllowPrompter))
    }

    fn configured(
        with_workspace: bool,
        with_skills: bool,
        prompter: Arc<dyn PermissionPrompter>,
    ) -> Self {
        let temporary = TemporaryDirectory::new();
        let workspace = temporary.0.join("workspace");
        let state = temporary.0.join("state");
        fs::create_dir(&workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let configuration = temporary.0.join("configuration");
        fs::create_dir_all(configuration.join("machine-god")).unwrap();
        fs::write(
            configuration.join("machine-god/config.json"),
            json!({
                "schema_version":5, "permission_mode":"ask", "sandbox_mode":"none",
                "permission_rules":[], "provider":"vercel_ai_gateway",
                "transport":"ai_gateway_http", "credential_source":"environment",
                "model":"fixture/default", "effort":"auto", "fast_mode":false,
            })
            .to_string(),
        )
        .unwrap();
        let config = load_native_config(&NativeEnvironment::new(
            Some(configuration.into_os_string()),
            None,
            None,
        ))
        .unwrap();
        let environment = NativeEnvironment::new(None, Some(state.into_os_string()), None);
        let roots = PreparedNativeRoots::prepare(
            NativeRootSelection::from_environment(&environment, &workspace).unwrap(),
        )
        .unwrap();
        let state_root = roots.state_root().to_owned();
        let helper = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY").map_or_else(
            || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/machine-god"),
            PathBuf::from,
        );
        assert!(
            helper.is_file(),
            "build the release CLI before interactive owner tests"
        );
        let undo = Arc::new(FileUndoTracker::new());
        let routes = Arc::new(NativeConversationModelRoutes::new());
        let observations = Arc::new(NativeConversationObservations::new());
        let mut options = NativeReferenceHostConversationOptions::new(undo.clone())
            .with_model_routes(routes.clone())
            .with_observations(observations.clone())
            .with_terminal(
                NativeReferenceHostTerminalOptions::new(
                    helper,
                    Some("/bin/bash".into()),
                    vec![("PATH".into(), "/usr/bin:/bin".into())],
                )
                .unwrap(),
            )
            .with_permissions(NativeReferenceHostPermissionOptions::new(
                Arc::new(NativePermissionContexts::new()),
                Arc::new(TokioPermissionReviewClock),
            ));
        if with_workspace {
            let primary = workspace.clone();
            let state = state_root.canonicalize().unwrap();
            let authority = std::thread::spawn(move || {
                super::native::NativeWorkspaceAuthority::open_blocking(
                    fs::File::open(&primary).unwrap().into(),
                    primary,
                    Some(fs::File::open(&state).unwrap().into()),
                    state,
                    vec![],
                    false,
                )
                .unwrap()
            })
            .join()
            .unwrap();
            options = options.with_workspace(
                authority,
                Arc::new(super::native::NativeWorkspaceContexts::new()),
            );
        }
        if with_skills {
            options = options.with_skills(skills_service(&state_root));
        }
        let transport = ScriptedTransport::default();
        let host = Arc::new(NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots_and_conversation(
            config, Arc::new(transport.clone()), NetworkTarget {
                scheme: "https".into(), host: "ai-gateway.vercel.sh".into(), port: None,
            }, roots, prompter, Arc::new(NoQuestions), Arc::new(NeverDeadline), options,
        ).unwrap());
        Self {
            host,
            workspace,
            transport,
            undo,
            routes,
            observations,
            state_root,
            _temporary: temporary,
        }
    }

    pub fn finish(self) {
        let completion = self.host.terminal_shutdown_completion().unwrap();
        assert_eq!(
            Arc::strong_count(&self.host),
            1,
            "drop interactive owners before host shutdown"
        );
        drop(self.host);
        completion.wait_on_worker().unwrap();
    }

    pub fn block_publication(&self, id: &machine_god_core::SessionId) -> PublicationBlock {
        let mut hash = Sha256::new();
        hash.update(b"machine-god:file-session:v1:");
        hash.update(id.as_str().as_bytes());
        let path = self
            .state_root
            .join(format!("session-{:x}.tmp", hash.finalize()));
        fs::create_dir(&path).unwrap();
        PublicationBlock(path)
    }
}

fn skills_service(state_root: &Path) -> Arc<super::native::NativeSkillsService> {
    let managed = Arc::new(super::native::NativeManagedSkills::open(state_root, None).unwrap());
    let catalog = Arc::new(
        super::native::NativeSkillCatalog::new(vec![managed.catalog_root().unwrap()]).unwrap(),
    );
    Arc::new(super::native::NativeSkillsService::new(
        catalog,
        Some(managed),
    ))
}

pub struct PublicationBlock(PathBuf);

impl Drop for PublicationBlock {
    fn drop(&mut self) {
        fs::remove_dir(&self.0).unwrap();
    }
}

pub fn call(name: &str, input: &Value) -> Vec<u8> {
    format!("data: {}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"tool-calls\"}}}}\n\n",
        json!({"type":"tool-call","toolCallId":"reusable-call","toolName":name,"input":input}))
        .into_bytes()
}

pub fn answer() -> Vec<u8> {
    b"data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"complete\"}\n\ndata: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n".to_vec()
}
