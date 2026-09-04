#![cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{self, FileTimes};
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures_util::{StreamExt, stream};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, FilesystemAccess, NetworkTarget,
    PermissionRequest, Role, SUBAGENT_TOOL_NAME, SessionId, SessionIncarnationId, StopReason,
    SubagentAuthority, SubagentAuthorityError, SubagentOutcome, SubagentRequest, Tool, ToolCallId,
    ToolContext, ToolError, ToolName, ToolOutput, ToolSpec, TurnEvent, TurnId,
};
use machine_god_native::{
    AI_GATEWAY_DEFAULT_MODEL, ASK_USER_QUESTION_TOOL_NAME, AiGatewayByteStream,
    AiGatewayCredentialEnvironment, AiGatewayCredentialSource, AiGatewayTransport,
    AiGatewayTransportRequest, COPY_FILE_TOOL_NAME, CREATE_FOLDER_TOOL_NAME, ConfigOrigin,
    DELETE_FILE_TOOL_NAME, EDIT_FILE_TOOL_NAME, FILE_INFO_TOOL_NAME, GLOB_FILES_TOOL_NAME,
    GREP_FILES_TOOL_NAME, INSTALL_SKILL_TOOL_NAME, LIST_FILES_TOOL_NAME, LoadedNativeConfig,
    MCP_FEATURES_TOOL_NAME, MCP_SEARCH_TOOLS_TOOL_NAME, MCP_SELECT_TOOL_NAME, MEMORY_TOOL_NAME,
    McpFeatureAuthority, McpFeatureError, McpFeaturePayload, McpFeatureRequest, McpToolCatalog,
    McpToolCatalogError, McpToolCatalogSnapshot, McpToolMetadata, NativeEnvironment,
    NativeReferenceHost, NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind,
    OPEN_FILE_TOOL_NAME, PermissionPromptDecision, PermissionPromptError, PermissionPrompter,
    QuestionPromptAnswers, QuestionPromptError, QuestionPromptOutcome, QuestionPromptRequest,
    QuestionPrompter, READ_FILE_TOOL_NAME, READ_TOOL_RESULT_TOOL_NAME, RENAME_FILE_TOOL_NAME,
    SEMANTIC_SEARCH_TOOL_NAME, SKILL_TOOL_NAME, TERMINAL_TOOL_NAME, VISION_TOOL_NAME,
    WEB_FETCH_TOOL_NAME, WEB_SEARCH_TOOL_NAME, WRITE_FILE_TOOL_NAME, WebSearchDeadline,
    WebSearchTransportError, load_native_config,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

mod web_search_support;

use web_search_support::{never_deadline, production_gateway_target};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-reference-host-{label}-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create a temporary directory: {error}"),
            }
        }
        panic!("failed to allocate a unique temporary directory");
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let result = fs::remove_dir_all(&self.path);
        if std::thread::panicking() {
            return;
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove a temporary directory: {error}"),
        }
    }
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct TransportState {
    responses: VecDeque<Vec<u8>>,
    requests: Vec<CapturedRequest>,
}

#[derive(Clone)]
struct ScriptedTransport {
    state: Arc<Mutex<TransportState>>,
    diagnostic_marker: Arc<str>,
}

impl ScriptedTransport {
    fn new(marker: &str, responses: impl IntoIterator<Item = impl Into<Vec<u8>>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(TransportState {
                responses: responses.into_iter().map(Into::into).collect(),
                requests: Vec::new(),
            })),
            diagnostic_marker: Arc::from(marker),
        }
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl AiGatewayTransport for ScriptedTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
        assert!(!self.diagnostic_marker.is_empty());
        let (headers, body) = request.into_parts();
        let response = {
            let mut state = self.state.lock().unwrap();
            state.requests.push(CapturedRequest {
                headers: headers
                    .into_iter()
                    .map(machine_god_native::AiGatewayHeader::into_parts)
                    .collect(),
                body,
            });
            state
                .responses
                .pop_front()
                .expect("scripted transport response")
        };
        Box::pin(async move { Ok(Box::pin(stream::iter([Ok(response)])) as AiGatewayByteStream) })
    }
}

struct CapacityOneState {
    responses: VecDeque<Vec<u8>>,
    requests: Vec<CapturedRequest>,
}

#[derive(Clone)]
struct CapacityOneTransport {
    permits: Arc<Semaphore>,
    state: Arc<Mutex<CapacityOneState>>,
}

impl CapacityOneTransport {
    fn new(responses: impl IntoIterator<Item = impl Into<Vec<u8>>>) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(1)),
            state: Arc::new(Mutex::new(CapacityOneState {
                responses: responses.into_iter().map(Into::into).collect(),
                requests: Vec::new(),
            })),
        }
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl AiGatewayTransport for CapacityOneTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
        let permits = Arc::clone(&self.permits);
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let permit = permits
                .acquire_owned()
                .await
                .expect("test semaphore remains open");
            let (headers, body) = request.into_parts();
            let response = {
                let mut state = state.lock().unwrap();
                state.requests.push(CapturedRequest {
                    headers: headers
                        .into_iter()
                        .map(machine_god_native::AiGatewayHeader::into_parts)
                        .collect(),
                    body,
                });
                state
                    .responses
                    .pop_front()
                    .expect("capacity-one response script")
            };
            let stream = stream::iter([Ok(response)]).map(move |chunk| {
                let _permit = &permit;
                chunk
            });
            Ok(Box::pin(stream) as AiGatewayByteStream)
        })
    }
}

#[derive(Clone, Default)]
struct AllowingPrompter {
    requests: Arc<Mutex<Vec<PermissionRequest>>>,
}

impl AllowingPrompter {
    fn requests(&self) -> Vec<PermissionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl PermissionPrompter for AllowingPrompter {
    fn prompt(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async { Ok(PermissionPromptDecision::AllowOnce) })
    }
}

struct InertQuestionPrompter;

impl QuestionPrompter for InertQuestionPrompter {
    fn prompt(
        &self,
        _request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        panic!("reference-host fixture did not expect an ordinary question prompt")
    }
}

fn inert_question_prompter() -> Arc<dyn QuestionPrompter> {
    Arc::new(InertQuestionPrompter)
}

#[derive(Clone, Default)]
struct AnsweringQuestionPrompter {
    questions: Arc<Mutex<Vec<Vec<String>>>>,
}

impl QuestionPrompter for AnsweringQuestionPrompter {
    fn prompt(
        &self,
        request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        self.questions.lock().unwrap().push(
            request
                .questions()
                .iter()
                .map(|question| question.question().to_owned())
                .collect(),
        );
        let mut answers = QuestionPromptAnswers::new();
        answers.try_push("a free-form answer".to_owned()).unwrap();
        Box::pin(async move { Ok(QuestionPromptOutcome::Answered(answers)) })
    }
}

fn config_path(config_root: &Path) -> PathBuf {
    config_root.join("machine-god").join("config.json")
}

fn load_config(base: &Path, contents: &str) -> LoadedNativeConfig {
    let config_root = base.join("configuration-root");
    let path = config_path(&config_root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
    load_native_config(&NativeEnvironment::new(
        Some(config_root.into_os_string()),
        None,
        None,
    ))
    .unwrap()
}

fn load_v1_config(base: &Path) -> LoadedNativeConfig {
    load_config(base, r#"{"schema_version":1,"permission_mode":"ask"}"#)
}

fn load_v2_config(base: &Path, model: &str) -> LoadedNativeConfig {
    load_config(
        base,
        &format!(
            r#"{{"schema_version":2,"permission_mode":"ask","provider":"vercel_ai_gateway","transport":"ai_gateway_http","model":"{model}"}}"#
        ),
    )
}

fn built_in_config() -> LoadedNativeConfig {
    load_native_config(&NativeEnvironment::new(None, None, None)).unwrap()
}

fn roots(base: &Path) -> (PathBuf, PathBuf) {
    let workspace = base.join("workspace");
    let sessions = base.join("sessions");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&sessions).unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    (workspace, sessions)
}

fn tool_round_responses(final_text: &str) -> [Vec<u8>; 5] {
    let first = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"list-call\",\"toolName\":\"list_files\",\"input\":{\"path\":\"./nested//.\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"read-call\",\"toolName\":\"read_file\",\"input\":{\"path\":\"./nested//note.txt\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"info-call\",\"toolName\":\"file_info\",\"input\":{\"path\":\"./nested//note.txt\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"glob-call\",\"toolName\":\"glob_files\",\"input\":{\"pattern\":\"*.txt\",\"path\":\"./nested//.\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let second = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"grep-call\",\"toolName\":\"grep_files\",\"input\":{\"pattern\":\"RETAINED_FILE_CONTENT_SENTINEL\",\"path\":\"./nested//.\",\"include\":\"*.txt\",\"case_insensitive\":false,\"mode\":\"count\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"write-call\",\"toolName\":\"write_file\",\"input\":{\"path\":\"./nested//generated.txt\",\"content\":\"generated retained content\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"edit-call\",\"toolName\":\"edit_file\",\"input\":{\"path\":\"./nested//generated.txt\",\"old_string\":\"retained\",\"new_string\":\"edited\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"rename-call\",\"toolName\":\"rename_file\",\"input\":{\"old_path\":\"./nested//generated.txt\",\"new_path\":\"./nested//renamed.txt\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let third = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"copy-call\",\"toolName\":\"copy_file\",\"input\":{\"source\":\"./nested//renamed.txt\",\"destination\":\"./nested//copied.txt\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"create-call\",\"toolName\":\"create_folder\",\"input\":{\"path\":\"./nested//created\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let fourth = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"delete-copy-call\",\"toolName\":\"delete_file\",\"input\":{\"path\":\"./nested//copied.txt\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"delete-source-call\",\"toolName\":\"delete_file\",\"input\":{\"path\":\"./nested//renamed.txt\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let fifth = format!(
        "data: {{\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":{}}}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"stop\"}}}}\n\n",
        serde_json::to_string(final_text).unwrap()
    )
    .into_bytes();
    [first, second, third, fourth, fifth]
}

fn semantic_search_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"semantic-call\",\"toolName\":\"semantic_search\",\"input\":{\"query\":\"alpha responsibility\",\"path\":\"./scope//.\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"semantic search complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn skill_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"skill-call\",\"toolName\":\"skill\",\"input\":{\"name\":\"release-checks\",\"resource\":\"./references//linux.md\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"skill read complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn terminal_inspect_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"terminal-inspect-call\",\"toolName\":\"terminal\",\"input\":{\"action\":\"inspect\",\"background_id\":17}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"inspection complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn terminal_list_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"terminal-list-call\",\"toolName\":\"terminal\",\"input\":{\"action\":\"list\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"listing complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn terminal_wait_round_responses(background_id: u64, wait_ceiling_ms: u64) -> [Vec<u8>; 2] {
    let tool_call = format!(
        concat!(
            "data: {{\"type\":\"tool-call\",\"toolCallId\":\"terminal-wait-call\",",
            "\"toolName\":\"terminal\",\"input\":{{\"action\":\"wait\",",
            "\"background_id\":{background_id},\"return_when\":{{\"kind\":\"exit\"}},",
            "\"wait_ceiling_ms\":{wait_ceiling_ms}}}}}\n\n",
            "data: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"tool-calls\"}}}}\n\n"
        ),
        background_id = background_id,
        wait_ceiling_ms = wait_ceiling_ms,
    )
    .into_bytes();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"wait complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn digest_name(prefix: &str, domain: &[u8], value: &[u8], suffix: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(value);
    format!("{prefix}{:x}{suffix}", hasher.finalize())
}

#[allow(clippy::too_many_arguments)]
fn write_background_record(
    state_root: &Path,
    workspace: &Path,
    id: u64,
    updated_at_ms: u64,
    state: &str,
    pid: Option<u32>,
    exit_code: Option<i32>,
    command: &str,
) {
    let workspace_text = workspace.to_str().unwrap();
    let workspace_name = digest_name(
        "workspace-",
        b"machine-god:background-workspace:v1:",
        workspace_text.as_bytes(),
        "",
    );
    let record_name = digest_name(
        "record-",
        b"machine-god:background-record:v1:",
        &id.to_be_bytes(),
        ".json",
    );
    let record_root = state_root.join("background-v1").join(workspace_name);
    fs::create_dir_all(&record_root).unwrap();
    for directory in [
        state_root.to_owned(),
        state_root.join("background-v1"),
        record_root.clone(),
    ] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let record_path = record_root.join(record_name);
    fs::write(
        &record_path,
        serde_json::to_vec(&json!({
            "version": 1,
            "workspace": workspace_text,
            "id": id,
            "started_at_ms": 10,
            "updated_at_ms": updated_at_ms,
            "command": command,
            "cwd": workspace_text,
            "state": state,
            "pid": pid,
            "exit_code": exit_code,
            "server_url": null,
            "diagnostic": null
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(record_path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn read_background_state(state_root: &Path, workspace: &Path, id: u64) -> String {
    let workspace_text = workspace.to_str().unwrap();
    let workspace_name = digest_name(
        "workspace-",
        b"machine-god:background-workspace:v1:",
        workspace_text.as_bytes(),
        "",
    );
    let record_name = digest_name(
        "record-",
        b"machine-god:background-record:v1:",
        &id.to_be_bytes(),
        ".json",
    );
    let bytes = fs::read(
        state_root
            .join("background-v1")
            .join(workspace_name)
            .join(record_name),
    )
    .unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["state"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[cfg(target_os = "linux")]
fn expected_semantic_search_output() -> Value {
    json!({
        "content": {
            "query": "alpha responsibility",
            "path": "scope",
            "keywords": ["alpha", "responsibility"],
            "results": [{
                "path": "scope/concept.rs",
                "score": 2,
                "line_number": 1,
                "line": "alpha responsibility",
                "line_truncated": false,
            }],
            "visited_entries": 1,
            "candidate_files": 1,
            "searched_files": 1,
            "skipped_oversized_files": 0,
            "skipped_non_text_files": 0,
            "skipped_symlink_entries": 0,
            "matching_files": 1,
            "incomplete": false,
            "incomplete_reasons": [],
        },
        "is_error": false,
    })
}

fn web_search_round_responses() -> [Vec<u8>; 3] {
    let outer_tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"search-call\",\"toolName\":\"web_search\",\"input\":{\"query\":\"latest Rust release\",\"allowed_domains\":[\"rust-lang.org\"]}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let nested_search = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"provider-search\",\"toolName\":\"perplexity_search\",\"input\":\"{}\",\"providerExecuted\":true}\n\n",
        "data: {\"type\":\"tool-result\",\"toolCallId\":\"provider-search\",\"toolName\":\"perplexity_search\",\"result\":{\"id\":\"search-response\",\"results\":[{\"title\":\"Rust releases\",\"url\":\"https://www.rust-lang.org/tools/install\",\"snippet\":\"Current Rust release information\"}]}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\",\"raw\":\"provider-stop\"},\"usage\":{\"inputTokens\":{\"total\":10,\"noCache\":8,\"cacheRead\":2,\"cacheWrite\":0},\"outputTokens\":{\"total\":5,\"text\":5,\"reasoning\":0},\"raw\":{\"provider\":{\"promptTokens\":10}}},\"providerMetadata\":{\"gateway\":{\"route\":\"direct\"}}}\n\n",
        "data: [DONE]\n\n"
    )
    .as_bytes()
    .to_vec();
    let outer_finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"search complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [outer_tool_call, nested_search, outer_finish]
}

fn vision_round_responses() -> [Vec<u8>; 3] {
    let outer_tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"vision-call\",\"toolName\":\"vision\",\"input\":{\"paths\":[\"nested/PRIVATE_PATH_SENTINEL.png\"],\"focus\":\"Read the status indicator.\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let evidence = json!({
        "images": [{
            "image_id": 1,
            "status": "ok",
            "summary": "The status indicator is ready.",
            "visible_text": ["READY"],
            "details": ["The indicator is green."]
        }]
    });
    let nested_vision = format!(
        "data: {{\"type\":\"stream-start\",\"warnings\":[]}}\n\ndata: {{\"type\":\"text-start\",\"id\":\"vision-text\"}}\n\ndata: {{\"type\":\"text-delta\",\"id\":\"vision-text\",\"delta\":{}}}\n\ndata: {{\"type\":\"text-end\",\"id\":\"vision-text\"}}\n\ndata: {{\"type\":\"finish\",\"finishReason\":{{\"unified\":\"stop\"}}}}\n\ndata: [DONE]\n\n",
        serde_json::to_string(&evidence.to_string()).unwrap()
    )
    .into_bytes();
    let outer_finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"vision complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [outer_tool_call, nested_vision, outer_finish]
}

fn ask_user_question_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"question-call\",\"toolName\":\"ask_user_question\",\"input\":{\"questions\":[{\"question\":\"  Which path?  \",\"options\":[{\"label\":\"First\"},{\"label\":\"Second\"}]}]}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"question complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn mcp_search_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"mcp-search-call\",\"toolName\":\"mcp_search_tools\",\"input\":{\"query\":\"github issue\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"MCP search complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn mcp_select_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"mcp-select-call\",\"toolName\":\"mcp_select_tool\",\"input\":{\"name\":\"mcp_github_create_issue\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"MCP selection complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn mcp_features_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"mcp-features-call\",\"toolName\":\"mcp_features\",\"input\":{\"action\":\"resource_templates\",\"server\":\"fixture\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"MCP features complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

fn subagent_round_responses() -> [Vec<u8>; 2] {
    let tool_call = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"subagent-call\",\"toolName\":\"subagent\",\"input\":{\"command\":{\"create\":{\"name\":\"reviewer\",\"mode\":\"one_off\",\"prompt\":\"Review the current change\"}}}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let finish = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"Subagent complete\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [tool_call, finish]
}

#[derive(Clone, Copy, Debug)]
struct ReadyMcpCatalog;

impl McpToolCatalog for ReadyMcpCatalog {
    fn snapshot(
        &self,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
        Box::pin(async {
            Ok(McpToolCatalogSnapshot::new(vec![
                McpToolMetadata::new(
                    "mcp_github_create_issue",
                    "github",
                    "Create a GitHub issue",
                    "repository title schema-private-sentinel",
                    vec!["mcp".to_owned(), "issue".to_owned()],
                )
                .expect("static MCP metadata is valid")
                .with_tool(ReferenceDynamicTool)
                .expect("static MCP executable is valid"),
            ])
            .expect("static MCP catalog is valid"))
        })
    }
}

#[derive(Clone, Default)]
struct ReadyMcpFeatureAuthority {
    calls: Arc<AtomicU64>,
}

impl ReadyMcpFeatureAuthority {
    fn call_count(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl McpFeatureAuthority for ReadyMcpFeatureAuthority {
    fn call(
        &self,
        request: McpFeatureRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<McpFeaturePayload, McpFeatureError>> {
        Box::pin(async move {
            assert!(!cancellation.is_cancelled());
            assert_eq!(request.server(), "fixture");
            self.calls.fetch_add(1, Ordering::SeqCst);
            McpFeaturePayload::new(json!({
                "items": [{
                    "server": "fixture",
                    "identity": "custom://project/{path}",
                    "name": "project-file",
                    "title": "Project file",
                    "description": "UNTRUSTED_TEMPLATE_DESCRIPTION",
                    "mimeType": "text/plain",
                    "template": true
                }]
            }))
        })
    }
}

#[derive(Clone, Default)]
struct ReadySubagentAuthority {
    calls: Arc<AtomicU64>,
    requests: Arc<Mutex<Vec<SubagentRequest>>>,
}

impl ReadySubagentAuthority {
    fn call_count(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<SubagentRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl SubagentAuthority for ReadySubagentAuthority {
    fn run(
        &self,
        request: SubagentRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<SubagentOutcome, SubagentAuthorityError>> {
        let calls = Arc::clone(&self.calls);
        let requests = Arc::clone(&self.requests);
        Box::pin(async move {
            assert!(!cancellation.is_cancelled());
            calls.fetch_add(1, Ordering::SeqCst);
            requests.lock().unwrap().push(request);
            SubagentOutcome::new("No correctness findings in the bounded review")
        })
    }
}

struct ReferenceDynamicTool;

impl Tool for ReferenceDynamicTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("mcp_github_create_issue").unwrap(),
            description: "Create a GitHub issue".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"title": {"type": "string"}},
                "required": ["title"],
            }),
        }
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { Ok(ToolOutput::success(json!({"created": true}))) })
    }
}

fn compose_with_transport(
    loaded: LoadedNativeConfig,
    transport: ScriptedTransport,
    workspace: &Path,
    sessions: &Path,
    prompter: AllowingPrompter,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError> {
    compose_with_transport_and_deadline(
        loaded,
        transport,
        workspace,
        sessions,
        prompter,
        never_deadline(),
    )
}

fn compose_with_transport_and_deadline(
    loaded: LoadedNativeConfig,
    transport: ScriptedTransport,
    workspace: &Path,
    sessions: &Path,
    prompter: AllowingPrompter,
    deadline: Arc<dyn WebSearchDeadline>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError> {
    let transport: Arc<dyn AiGatewayTransport> = Arc::new(transport);
    let prompter: Arc<dyn PermissionPrompter> = Arc::new(prompter);
    NativeReferenceHost::compose_with_ai_gateway_transport(
        loaded,
        transport,
        production_gateway_target(),
        workspace,
        sessions,
        prompter,
        inert_question_prompter(),
        deadline,
    )
}

#[derive(Clone, Default)]
struct SleepingDeadline {
    calls: Arc<AtomicU64>,
}

impl SleepingDeadline {
    fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl WebSearchDeadline for SleepingDeadline {
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            Ok(())
        })
    }
}

fn compose_with_transport_and_mcp_catalog(
    loaded: LoadedNativeConfig,
    transport: ScriptedTransport,
    workspace: &Path,
    sessions: &Path,
    prompter: AllowingPrompter,
    catalog: Arc<dyn McpToolCatalog>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError> {
    NativeReferenceHost::compose_with_ai_gateway_transport_and_mcp_catalog(
        loaded,
        Arc::new(transport),
        production_gateway_target(),
        workspace,
        sessions,
        Arc::new(prompter),
        inert_question_prompter(),
        never_deadline(),
        catalog,
    )
}

fn compose_with_transport_and_mcp(
    loaded: LoadedNativeConfig,
    transport: ScriptedTransport,
    workspace: &Path,
    sessions: &Path,
    prompter: AllowingPrompter,
    catalog: Arc<dyn McpToolCatalog>,
    feature_authority: Arc<dyn McpFeatureAuthority>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError> {
    NativeReferenceHost::compose_with_ai_gateway_transport_and_mcp(
        loaded,
        Arc::new(transport),
        production_gateway_target(),
        workspace,
        sessions,
        Arc::new(prompter),
        inert_question_prompter(),
        never_deadline(),
        catalog,
        feature_authority,
    )
}

fn compose_with_transport_and_subagent(
    loaded: LoadedNativeConfig,
    transport: ScriptedTransport,
    workspace: &Path,
    sessions: &Path,
    prompter: AllowingPrompter,
    subagent_authority: Arc<dyn SubagentAuthority>,
) -> Result<NativeReferenceHost, NativeReferenceHostBuildError> {
    NativeReferenceHost::compose_with_ai_gateway_transport_and_subagent(
        loaded,
        Arc::new(transport),
        production_gateway_target(),
        workspace,
        sessions,
        Arc::new(prompter),
        inert_question_prompter(),
        never_deadline(),
        subagent_authority,
    )
}

fn collect_turn(host: &NativeReferenceHost, session_name: &str) -> (SessionId, Vec<TurnEvent>) {
    let session_id = SessionId::new(session_name).unwrap();
    let session = host
        .engine()
        .create_session(
            session_id.clone(),
            SessionIncarnationId::new(format!("{session_name}-incarnation")).unwrap(),
        )
        .unwrap();
    let events = futures_executor::block_on(async {
        session
            .prompt("inspect the workspace")
            .await
            .unwrap()
            .map(|event| event.map(|event| event.payload))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    (session_id, events)
}

fn body(request: &CapturedRequest) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

fn header<'a>(request: &'a CapturedRequest, name: &str) -> &'a str {
    request
        .headers
        .iter()
        .find_map(|(candidate, value)| (candidate == name).then_some(value.as_str()))
        .unwrap_or_else(|| panic!("missing request header {name}"))
}

fn decoded_tool_output(request: &Value, prompt_index: usize) -> Value {
    let encoded = request["prompt"][prompt_index]["content"][0]["output"]["value"]
        .as_str()
        .expect("tool output is encoded as text");
    serde_json::from_str(encoded).unwrap()
}

fn assert_completed(events: &[TurnEvent]) {
    assert!(matches!(
        events.last(),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
}

fn build_error<T>(
    result: Result<T, NativeReferenceHostBuildError>,
) -> NativeReferenceHostBuildError {
    match result {
        Ok(value) => {
            drop(value);
            panic!("native reference host unexpectedly composed");
        }
        Err(error) => error,
    }
}

fn assert_redacted(error: NativeReferenceHostBuildError, forbidden: &[&str]) {
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert!(!display.is_empty());
    for sentinel in forbidden {
        assert!(
            !display.contains(sentinel),
            "Display leaked sentinel {sentinel:?}: {display:?}"
        );
        assert!(
            !debug.contains(sentinel),
            "Debug leaked sentinel {sentinel:?}: {debug:?}"
        );
    }
}

fn assert_stage_debug(
    error: NativeReferenceHostBuildError,
    kind: NativeReferenceHostBuildErrorKind,
) {
    assert_eq!(
        format!("{error:?}"),
        format!("NativeReferenceHostBuildError {{ kind: {kind:?} }}")
    );
}

fn directory_is_empty(path: &Path) -> bool {
    fs::read_dir(path).unwrap().next().is_none()
}

fn assert_exact_native_tool_catalog(request: &Value) {
    let tools = request["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 26);
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            ASK_USER_QUESTION_TOOL_NAME,
            COPY_FILE_TOOL_NAME,
            CREATE_FOLDER_TOOL_NAME,
            DELETE_FILE_TOOL_NAME,
            EDIT_FILE_TOOL_NAME,
            FILE_INFO_TOOL_NAME,
            GLOB_FILES_TOOL_NAME,
            GREP_FILES_TOOL_NAME,
            INSTALL_SKILL_TOOL_NAME,
            LIST_FILES_TOOL_NAME,
            MCP_FEATURES_TOOL_NAME,
            MCP_SEARCH_TOOLS_TOOL_NAME,
            MCP_SELECT_TOOL_NAME,
            MEMORY_TOOL_NAME,
            OPEN_FILE_TOOL_NAME,
            READ_FILE_TOOL_NAME,
            READ_TOOL_RESULT_TOOL_NAME,
            RENAME_FILE_TOOL_NAME,
            SEMANTIC_SEARCH_TOOL_NAME,
            SKILL_TOOL_NAME,
            SUBAGENT_TOOL_NAME,
            TERMINAL_TOOL_NAME,
            VISION_TOOL_NAME,
            WEB_FETCH_TOOL_NAME,
            WEB_SEARCH_TOOL_NAME,
            WRITE_FILE_TOOL_NAME
        ]
    );
    assert!(tools.iter().all(|tool| tool["type"] == "function"));
    let terminal = tools
        .iter()
        .find(|tool| tool["name"] == TERMINAL_TOOL_NAME)
        .expect("terminal tool is registered");
    let forms = terminal["inputSchema"]["oneOf"].as_array().unwrap();
    assert_eq!(forms.len(), 5);
    assert_eq!(forms[0]["properties"]["action"]["const"], "exec");
    assert_eq!(forms[1]["properties"]["action"]["const"], "start");
    assert_eq!(forms[2]["properties"]["action"]["const"], "inspect");
    assert_eq!(forms[2]["required"], json!(["action", "background_id"]));
    assert_eq!(forms[3]["properties"]["action"]["const"], "wait");
    assert_eq!(
        forms[3]["required"],
        json!(["action", "background_id", "return_when", "wait_ceiling_ms"])
    );
    assert_eq!(forms[4]["properties"]["action"]["const"], "list");
    assert_eq!(forms[4]["required"], json!(["action"]));
    assert_eq!(forms[4]["additionalProperties"], false);
}

fn assert_exact_native_tool_permissions(prompter: &AllowingPrompter) {
    let permission_requests = prompter.requests();
    assert_eq!(permission_requests.len(), 12);
    assert_eq!(
        permission_requests[0].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Enumerate,
            path: "nested".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[1].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: "nested/note.txt".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[2].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Metadata,
            path: "nested/note.txt".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[3].capability,
        Capability::Filesystem {
            access: FilesystemAccess::EnumerateRecursive,
            path: "nested".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[4].capability,
        Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: "nested".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[5].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Write,
            path: "nested/generated.txt".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[6].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Edit,
            path: "nested/generated.txt".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[7].capability,
        Capability::FilesystemRename {
            old_path: "nested/generated.txt".to_owned(),
            new_path: "nested/renamed.txt".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[8].capability,
        Capability::FilesystemCopy {
            source: "nested/renamed.txt".to_owned(),
            destination: "nested/copied.txt".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[9].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Create,
            path: "nested/created".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[10].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Delete,
            path: "nested/copied.txt".to_owned(),
        }
    );
    assert_eq!(
        permission_requests[11].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Delete,
            path: "nested/renamed.txt".to_owned(),
        }
    );
}

fn assert_persisted_composed_turn(host: &NativeReferenceHost, session_id: SessionId) {
    let loaded_session = futures_executor::block_on(host.engine().load_session(session_id))
        .unwrap()
        .expect("the reference host persisted the completed session");
    let record = loaded_session.record();
    assert_eq!(record.messages.len(), 18);
    assert_eq!(record.messages[0].role, Role::User);
    assert_eq!(record.messages[1].role, Role::Assistant);
    assert_eq!(record.messages[2].role, Role::Tool);
    assert_eq!(record.messages[3].role, Role::Tool);
    assert_eq!(record.messages[4].role, Role::Tool);
    assert_eq!(record.messages[5].role, Role::Tool);
    assert_eq!(record.messages[6].role, Role::Assistant);
    assert_eq!(record.messages[7].role, Role::Tool);
    assert_eq!(record.messages[8].role, Role::Tool);
    assert_eq!(record.messages[9].role, Role::Tool);
    assert_eq!(record.messages[10].role, Role::Tool);
    assert_eq!(record.messages[11].role, Role::Assistant);
    assert_eq!(record.messages[12].role, Role::Tool);
    assert_eq!(record.messages[13].role, Role::Tool);
    assert_eq!(record.messages[14].role, Role::Assistant);
    assert_eq!(record.messages[15].role, Role::Tool);
    assert_eq!(record.messages[16].role, Role::Tool);
    assert_eq!(record.messages[17].role, Role::Assistant);
    assert_eq!(
        record.messages[17].content,
        [ContentBlock::Text {
            text: "composition complete".to_owned()
        }]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn composition_wires_custom_model_exact_tools_normalized_permissions_and_durable_results() {
    let temporary = TemporaryDirectory::new("full-wiring");
    let (workspace, sessions) = roots(temporary.path());
    let nested = workspace.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("note.txt"), "reference host contents\n").unwrap();
    fs::File::options()
        .write(true)
        .open(nested.join("note.txt"))
        .unwrap()
        .set_times(
            FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::new(1_700_000_789, 987_654_321)),
        )
        .unwrap();
    fs::write(nested.join("other.txt"), "other").unwrap();
    let model = "custom/reference-host-model-v2";
    let loaded = load_v2_config(temporary.path(), model);
    let transport = ScriptedTransport::new(
        "FULL_WIRING_FACTORY_SENTINEL",
        tool_round_responses("composition complete"),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        loaded,
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();

    let (session_id, events) = collect_turn(&host, "reference-host-wiring");
    assert_completed(&events);

    let requests = transport.requests();
    assert_eq!(requests.len(), 5);
    assert_eq!(header(&requests[0], "ai-language-model-id"), model);
    let first = body(&requests[0]);
    assert_exact_native_tool_catalog(&first);
    assert_exact_native_tool_permissions(&prompter);

    let second = body(&requests[1]);
    assert_eq!(second["prompt"].as_array().unwrap().len(), 6);
    assert_eq!(second["prompt"][2]["content"][0]["toolCallId"], "list-call");
    assert_eq!(
        decoded_tool_output(&second, 2),
        json!({
            "content": {
                "path": "nested",
                "entries": [
                    {"name": "note.txt", "kind": "file"},
                    {"name": "other.txt", "kind": "file"}
                ],
                "truncated": false
            },
            "is_error": false
        })
    );
    assert_eq!(second["prompt"][3]["content"][0]["toolCallId"], "read-call");
    assert_eq!(
        decoded_tool_output(&second, 3),
        json!({
            "content": {"content": "reference host contents\n"},
            "is_error": false
        })
    );
    assert_eq!(second["prompt"][4]["content"][0]["toolCallId"], "info-call");
    assert_eq!(
        decoded_tool_output(&second, 4),
        json!({
            "content": {
                "path": "nested/note.txt",
                "kind": "file",
                "size_bytes": 24,
                "modified": {
                    "unix_seconds": 1_700_000_789_i64,
                    "nanoseconds": 987_654_321_u32
                },
                "extension": "txt"
            },
            "is_error": false
        })
    );
    assert_eq!(second["prompt"][5]["content"][0]["toolCallId"], "glob-call");
    assert_eq!(
        decoded_tool_output(&second, 5),
        json!({
            "content": {
                "path": "nested",
                "pattern": "*.txt",
                "mode": "matches",
                "matches": ["nested/note.txt", "nested/other.txt"],
                "truncated": false
            },
            "is_error": false
        })
    );
    let third = body(&requests[2]);
    assert_eq!(third["prompt"].as_array().unwrap().len(), 11);
    assert_eq!(third["prompt"][7]["content"][0]["toolCallId"], "grep-call");
    assert_eq!(
        decoded_tool_output(&third, 7),
        json!({
            "content": {
                "pattern": "RETAINED_FILE_CONTENT_SENTINEL",
                "path": "nested",
                "include": "*.txt",
                "case_insensitive": false,
                "mode": "count",
                "head_limit": 100,
                "offset": 0,
                "context_lines": 0,
                "candidate_files": 2,
                "searched_files": 2,
                "skipped_oversized_files": 0,
                "skipped_non_text_files": 0,
                "matching_lines": 0,
                "matching_files": 0
            },
            "is_error": false
        })
    );
    assert_eq!(third["prompt"][8]["content"][0]["toolCallId"], "write-call");
    assert_eq!(
        decoded_tool_output(&third, 8),
        json!({
            "content": {
                "path": "nested/generated.txt",
                "bytes_written": "generated retained content".len()
            },
            "is_error": false
        })
    );
    assert_eq!(third["prompt"][9]["content"][0]["toolCallId"], "edit-call");
    assert_eq!(
        decoded_tool_output(&third, 9),
        json!({
            "content": {
                "path": "nested/generated.txt",
                "bytes_written": "generated edited content".len()
            },
            "is_error": false
        })
    );
    assert_eq!(
        third["prompt"][10]["content"][0]["toolCallId"],
        "rename-call"
    );
    assert_eq!(
        decoded_tool_output(&third, 10),
        json!({
            "content": {
                "old_path": "nested/generated.txt",
                "new_path": "nested/renamed.txt"
            },
            "is_error": false
        })
    );
    let fourth = body(&requests[3]);
    assert_eq!(fourth["prompt"].as_array().unwrap().len(), 14);
    assert_eq!(
        fourth["prompt"][12]["content"][0]["toolCallId"],
        "copy-call"
    );
    assert_eq!(
        decoded_tool_output(&fourth, 12),
        json!({
            "content": {
                "source": "nested/renamed.txt",
                "destination": "nested/copied.txt",
                "bytes_copied": "generated edited content".len()
            },
            "is_error": false
        })
    );
    assert_eq!(
        fourth["prompt"][13]["content"][0]["toolCallId"],
        "create-call"
    );
    assert_eq!(
        decoded_tool_output(&fourth, 13),
        json!({
            "content": {"path": "nested/created"},
            "is_error": false
        })
    );
    let fifth = body(&requests[4]);
    assert_eq!(fifth["prompt"].as_array().unwrap().len(), 17);
    assert_eq!(
        fifth["prompt"][15]["content"][0]["toolCallId"],
        "delete-copy-call"
    );
    assert_eq!(
        decoded_tool_output(&fifth, 15),
        json!({
            "content": {"path": "nested/copied.txt"},
            "is_error": false
        })
    );
    assert_eq!(
        fifth["prompt"][16]["content"][0]["toolCallId"],
        "delete-source-call"
    );
    assert_eq!(
        decoded_tool_output(&fifth, 16),
        json!({
            "content": {"path": "nested/renamed.txt"},
            "is_error": false
        })
    );
    assert!(!nested.join("generated.txt").exists());
    assert!(!nested.join("copied.txt").exists());
    assert!(!nested.join("renamed.txt").exists());
    assert!(nested.join("created").is_dir());

    drop(events);
    assert_persisted_composed_turn(&host, session_id);
    assert!(!directory_is_empty(&sessions));
}

#[test]
fn composed_mcp_search_uses_injected_catalog_without_permission_or_schema_leakage() {
    let temporary = TemporaryDirectory::new("mcp-search");
    let (workspace, sessions) = roots(temporary.path());
    let transport =
        ScriptedTransport::new("MCP_SEARCH_FACTORY_SENTINEL", mcp_search_round_responses());
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport_and_mcp_catalog(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
        Arc::new(ReadyMcpCatalog),
    )
    .unwrap();

    let (_, events) = collect_turn(&host, "reference-host-mcp-search");
    assert_completed(&events);
    assert!(prompter.requests().is_empty());

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_exact_native_tool_catalog(&body(&requests[0]));
    let continued = body(&requests[1]);
    assert_eq!(
        decoded_tool_output(&continued, 2),
        json!({
            "content": {
                "tools": [{
                    "name": "mcp_github_create_issue",
                    "server": "github",
                    "description": "Create a GitHub issue",
                    "purpose": "Create a GitHub issue",
                    "usage": ["mcp", "issue"]
                }],
                "count": 1
            },
            "is_error": false
        })
    );
    assert!(
        !serde_json::to_string(&continued["prompt"][2])
            .unwrap()
            .contains("schema-private-sentinel")
    );
}

#[test]
fn composed_mcp_select_advertises_the_injected_executable_on_the_next_round() {
    let temporary = TemporaryDirectory::new("mcp-select");
    let (workspace, sessions) = roots(temporary.path());
    let transport =
        ScriptedTransport::new("MCP_SELECT_FACTORY_SENTINEL", mcp_select_round_responses());
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport_and_mcp_catalog(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
        Arc::new(ReadyMcpCatalog),
    )
    .unwrap();

    let (_, events) = collect_turn(&host, "reference-host-mcp-select");
    assert_completed(&events);
    assert!(prompter.requests().is_empty());

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_exact_native_tool_catalog(&body(&requests[0]));
    let continued = body(&requests[1]);
    assert_eq!(
        decoded_tool_output(&continued, 2),
        json!({
            "content": "Selected dynamic MCP tool `mcp_github_create_issue`. Its executable schema will be available on the next model step; call `mcp_github_create_issue` with arguments matching the selected schema.",
            "is_error": false
        })
    );
    let continued_json = serde_json::to_string(&continued).unwrap();
    assert!(!continued_json.contains("schema-private-sentinel"));
    assert!(!continued_json.contains("PRIVATE_DESCRIPTION_SENTINEL"));
    assert_eq!(
        continued["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|tool| tool["name"] == "mcp_github_create_issue")
            .count(),
        1
    );
    let selected = continued["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "mcp_github_create_issue")
        .unwrap();
    assert_eq!(selected["description"], "Create a GitHub issue");
    assert_eq!(selected["inputSchema"]["required"], json!(["title"]));
}

#[test]
fn composed_mcp_features_uses_exact_injected_authority_without_permission() {
    let temporary = TemporaryDirectory::new("mcp-features");
    let (workspace, sessions) = roots(temporary.path());
    let transport = ScriptedTransport::new(
        "MCP_FEATURES_FACTORY_SENTINEL",
        mcp_features_round_responses(),
    );
    let prompter = AllowingPrompter::default();
    let feature_authority = Arc::new(ReadyMcpFeatureAuthority::default());
    let host = compose_with_transport_and_mcp(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
        Arc::new(ReadyMcpCatalog),
        feature_authority.clone(),
    )
    .unwrap();

    let (_, events) = collect_turn(&host, "reference-host-mcp-features");
    assert_completed(&events);
    assert!(prompter.requests().is_empty());
    assert_eq!(feature_authority.call_count(), 1);

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_exact_native_tool_catalog(&body(&requests[0]));
    assert_eq!(
        decoded_tool_output(&body(&requests[1]), 2),
        json!({
            "content": {
                "trust": "untrusted_external",
                "authority": "none",
                "action": "resource_templates",
                "server": "fixture",
                "items": [{
                    "server": "fixture",
                    "identity": "custom://project/{path}",
                    "name": "project-file",
                    "title": "Project file",
                    "description": "UNTRUSTED_TEMPLATE_DESCRIPTION",
                    "mimeType": "text/plain",
                    "template": true
                }]
            },
            "is_error": false
        })
    );
}

#[test]
fn composed_subagent_uses_exact_injected_authority_without_outer_permission() {
    let temporary = TemporaryDirectory::new("subagent");
    let (workspace, sessions) = roots(temporary.path());
    let transport = ScriptedTransport::new("SUBAGENT_FACTORY_SENTINEL", subagent_round_responses());
    let prompter = AllowingPrompter::default();
    let authority = Arc::new(ReadySubagentAuthority::default());
    let host = compose_with_transport_and_subagent(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
        authority.clone(),
    )
    .unwrap();

    assert_eq!(authority.call_count(), 0);
    let (_, events) = collect_turn(&host, "reference-host-subagent");
    assert_completed(&events);
    assert!(prompter.requests().is_empty());
    assert_eq!(authority.call_count(), 1);
    let requests = authority.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].name(), "reviewer");
    assert_eq!(requests[0].prompt(), "Review the current change");
    assert_eq!(requests[0].context().call_id.as_str(), "subagent-call");

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_exact_native_tool_catalog(&body(&requests[0]));
    assert_eq!(
        decoded_tool_output(&body(&requests[1]), 2),
        json!({
            "content": {
                "status": "completed",
                "trust": "untrusted_child",
                "authority": "none",
                "text": "No correctness findings in the bounded review"
            },
            "is_error": false
        })
    );
}

#[test]
fn reference_subagent_fixture_is_inert_until_its_future_is_polled() {
    let authority = ReadySubagentAuthority::default();
    let future = authority.run(
        SubagentRequest::new(
            ToolContext {
                session_id: SessionId::new("unpolled-subagent").unwrap(),
                session_incarnation_id: SessionIncarnationId::new("unpolled-subagent-incarnation")
                    .unwrap(),
                turn_id: TurnId::new("unpolled-subagent-turn").unwrap(),
                call_id: ToolCallId::new("unpolled-subagent-call").unwrap(),
            },
            "reviewer",
            "Review the current change",
        )
        .unwrap(),
        CancellationToken::new(),
    );

    assert_eq!(authority.call_count(), 0);
    assert!(authority.requests().is_empty());
    drop(future);
    assert_eq!(authority.call_count(), 0);
    assert!(authority.requests().is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn composed_semantic_search_uses_retained_workspace_and_persists_exact_result() {
    let temporary = TemporaryDirectory::new("semantic-search");
    let (workspace, sessions) = roots(temporary.path());
    fs::create_dir(workspace.join("scope")).unwrap();
    fs::write(workspace.join("scope/concept.rs"), "alpha responsibility\n").unwrap();
    let transport = ScriptedTransport::new(
        "SEMANTIC_SEARCH_FACTORY_SENTINEL",
        semantic_search_round_responses(),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();

    let retained_workspace = temporary.path().join("semantic-search-retained");
    fs::rename(&workspace, &retained_workspace).unwrap();
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("scope")).unwrap();
    fs::write(
        workspace.join("scope/concept.rs"),
        "SEMANTIC_REPLACEMENT_SENTINEL",
    )
    .unwrap();

    let (session_id, events) = collect_turn(&host, "semantic-search");
    assert_completed(&events);

    let permission_requests = prompter.requests();
    assert_eq!(permission_requests.len(), 1);
    assert_eq!(
        permission_requests[0].capability,
        Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: "scope".to_owned(),
        }
    );

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    let first = body(&requests[0]);
    assert_exact_native_tool_catalog(&first);
    let second = body(&requests[1]);
    assert_exact_native_tool_catalog(&second);
    assert_eq!(second["prompt"].as_array().unwrap().len(), 3);
    assert_eq!(
        second["prompt"][2]["content"][0]["toolCallId"],
        "semantic-call"
    );
    let expected = expected_semantic_search_output();
    assert_eq!(decoded_tool_output(&second, 2), expected);
    assert!(
        !serde_json::to_string(&second)
            .unwrap()
            .contains("SEMANTIC_REPLACEMENT_SENTINEL")
    );
    let finished_output = events
        .iter()
        .find_map(|event| match event {
            TurnEvent::ToolFinished { output, .. } => Some(output),
            _ => None,
        })
        .expect("semantic search emits one completed tool result");
    assert_eq!(finished_output.content, expected["content"]);
    assert!(!finished_output.is_error);

    let durable = futures_executor::block_on(host.engine().load_session(session_id))
        .unwrap()
        .expect("semantic search turn is durable");
    let record = durable.record();
    assert_eq!(record.messages.len(), 4);
    assert_eq!(record.messages[0].role, Role::User);
    assert_eq!(record.messages[1].role, Role::Assistant);
    assert_eq!(record.messages[2].role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = record.messages[2].content.as_slice() else {
        panic!("semantic search result is retained as one structured tool result")
    };
    assert_eq!(output.content, expected["content"]);
    assert!(!output.is_error);
    assert_eq!(record.messages[3].role, Role::Assistant);
    assert_eq!(
        record.messages[3].content,
        [ContentBlock::Text {
            text: "semantic search complete".to_owned(),
        }]
    );
    assert!(!directory_is_empty(&sessions));
}

#[cfg(target_os = "macos")]
#[test]
fn composed_semantic_search_preserves_catalog_and_returns_fixed_unsupported_result() {
    let temporary = TemporaryDirectory::new("semantic-search-unsupported");
    let (workspace, sessions) = roots(temporary.path());
    fs::create_dir(workspace.join("scope")).unwrap();
    fs::write(
        workspace.join("scope/concept.rs"),
        "SEMANTIC_MACOS_CONTENT_MUST_NOT_BE_READ",
    )
    .unwrap();
    let transport = ScriptedTransport::new(
        "SEMANTIC_MACOS_FACTORY_SENTINEL",
        semantic_search_round_responses(),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();

    let (session_id, events) = collect_turn(&host, "semantic-search-unsupported");
    assert_completed(&events);

    let permission_requests = prompter.requests();
    assert_eq!(permission_requests.len(), 1);
    assert_eq!(
        permission_requests[0].capability,
        Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: "scope".to_owned(),
        }
    );

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    let first = body(&requests[0]);
    assert_exact_native_tool_catalog(&first);
    let second = body(&requests[1]);
    assert_exact_native_tool_catalog(&second);
    let expected = json!({
        "content": {
            "code": "tool_error",
            "message": "tool execution failed",
            "retryable": false,
        },
        "is_error": true,
    });
    assert_eq!(decoded_tool_output(&second, 2), expected);
    assert!(
        !serde_json::to_string(&second)
            .unwrap()
            .contains("SEMANTIC_MACOS_CONTENT_MUST_NOT_BE_READ")
    );

    let finished_output = events
        .iter()
        .find_map(|event| match event {
            TurnEvent::ToolFinished { output, .. } => Some(output),
            _ => None,
        })
        .expect("unsupported semantic search emits one completed tool result");
    assert_eq!(finished_output.content, expected["content"]);
    assert!(finished_output.is_error);

    let durable = futures_executor::block_on(host.engine().load_session(session_id))
        .unwrap()
        .expect("unsupported semantic search turn is durable");
    let record = durable.record();
    let [ContentBlock::ToolResult { output, .. }] = record.messages[2].content.as_slice() else {
        panic!("unsupported semantic search result is retained as one structured tool result")
    };
    assert_eq!(output.content, expected["content"]);
    assert!(output.is_error);
    assert!(!directory_is_empty(&sessions));
}

#[test]
fn composed_skill_uses_retained_workspace_and_persists_exact_result() {
    let temporary = TemporaryDirectory::new("skill");
    let (workspace, sessions) = roots(temporary.path());
    let resource = workspace.join("skills/release-checks/references/linux.md");
    fs::create_dir_all(resource.parent().unwrap()).unwrap();
    let retained_content = "Run the retained release checks exactly.\n";
    fs::write(&resource, retained_content).unwrap();
    let transport = ScriptedTransport::new("SKILL_FACTORY_SENTINEL", skill_round_responses());
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();

    let retained_workspace = temporary.path().join("skill-retained");
    fs::rename(&workspace, &retained_workspace).unwrap();
    let replacement_resource = workspace.join("skills/release-checks/references/linux.md");
    fs::create_dir_all(replacement_resource.parent().unwrap()).unwrap();
    fs::write(
        &replacement_resource,
        "SKILL_REPLACEMENT_CONTENT_MUST_NOT_BE_READ",
    )
    .unwrap();

    let (session_id, events) = collect_turn(&host, "skill");
    assert_completed(&events);

    let permission_requests = prompter.requests();
    assert_eq!(permission_requests.len(), 1);
    assert_eq!(
        permission_requests[0].capability,
        Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: "skills/release-checks/references/linux.md".to_owned(),
        }
    );

    let expected = json!({
        "content": {
            "name": "release-checks",
            "resource": "references/linux.md",
            "offset": 0,
            "next_offset": retained_content.len(),
            "total_bytes": retained_content.len(),
            "content": retained_content,
            "truncated": false,
        },
        "is_error": false,
    });
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    let first = body(&requests[0]);
    assert_exact_native_tool_catalog(&first);
    let second = body(&requests[1]);
    assert_exact_native_tool_catalog(&second);
    assert_eq!(second["prompt"].as_array().unwrap().len(), 3);
    assert_eq!(
        second["prompt"][2]["content"][0]["toolCallId"],
        "skill-call"
    );
    assert_eq!(decoded_tool_output(&second, 2), expected);
    assert!(
        !serde_json::to_string(&second)
            .unwrap()
            .contains("SKILL_REPLACEMENT_CONTENT_MUST_NOT_BE_READ")
    );

    let finished_output = events
        .iter()
        .find_map(|event| match event {
            TurnEvent::ToolFinished { output, .. } => Some(output),
            _ => None,
        })
        .expect("skill emits one completed tool result");
    assert_eq!(finished_output.content, expected["content"]);
    assert!(!finished_output.is_error);

    let durable = futures_executor::block_on(host.engine().load_session(session_id))
        .unwrap()
        .expect("skill turn is durable");
    let record = durable.record();
    assert_eq!(record.messages.len(), 4);
    assert_eq!(record.messages[0].role, Role::User);
    assert_eq!(record.messages[1].role, Role::Assistant);
    assert_eq!(record.messages[2].role, Role::Tool);
    let [ContentBlock::ToolResult { output, .. }] = record.messages[2].content.as_slice() else {
        panic!("skill result is retained as one structured tool result")
    };
    assert_eq!(output.content, expected["content"]);
    assert!(!output.is_error);
    assert_eq!(record.messages[3].role, Role::Assistant);
    assert_eq!(
        record.messages[3].content,
        [ContentBlock::Text {
            text: "skill read complete".to_owned(),
        }]
    );
    assert_eq!(
        fs::read_to_string(retained_workspace.join("skills/release-checks/references/linux.md"))
            .unwrap(),
        retained_content
    );
    assert!(!directory_is_empty(&sessions));
}

#[test]
fn capacity_one_shared_transport_releases_outer_stream_for_custom_target_search() {
    let temporary = TemporaryDirectory::new("capacity-one-web-search");
    let (workspace, sessions) = roots(temporary.path());
    let transport = CapacityOneTransport::new(web_search_round_responses());
    let prompter = AllowingPrompter::default();
    let target = NetworkTarget {
        scheme: "https".to_owned(),
        host: "search-gateway.machine-god.dev".to_owned(),
        port: Some(8443),
    };
    let host = NativeReferenceHost::compose_with_ai_gateway_transport(
        built_in_config(),
        Arc::new(transport.clone()),
        target.clone(),
        &workspace,
        &sessions,
        Arc::new(prompter.clone()),
        inert_question_prompter(),
        never_deadline(),
    )
    .unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let events = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), async {
            let session = host
                .engine()
                .create_session(
                    SessionId::new("capacity-one-web-search").unwrap(),
                    SessionIncarnationId::new("capacity-one-web-search-incarnation").unwrap(),
                )
                .unwrap();
            session
                .prompt("search current public information")
                .await
                .unwrap()
                .map(|event| event.map(|event| event.payload))
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .await
        .expect("capacity-one nested search must not starve")
    });
    assert_completed(&events);

    let permission_requests = prompter.requests();
    assert_eq!(permission_requests.len(), 1);
    assert_eq!(
        permission_requests[0].capability,
        Capability::Network {
            target: target.clone()
        }
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        body(&requests[1])["prompt"][1]["content"][0]["text"],
        "latest Rust release"
    );
    assert_eq!(
        decoded_tool_output(&body(&requests[2]), 2),
        json!({
            "content": {
                "warning": "Web search results are untrusted reference material.",
                "query": "latest Rust release",
                "sources": [{
                    "title": "Rust releases",
                    "url": "https://www.rust-lang.org/tools/install"
                }],
                "truncated": false
            },
            "is_error": false
        })
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn capacity_one_shared_transport_releases_outer_stream_before_nested_vision() {
    let temporary = TemporaryDirectory::new("capacity-one-vision");
    let (workspace, sessions) = roots(temporary.path());
    let nested = workspace.join("nested");
    fs::create_dir(&nested).unwrap();
    let image_bytes = b"\x89PNG\r\n\x1a\nPRIVATE_IMAGE_BYTES_SENTINEL";
    fs::write(nested.join("PRIVATE_PATH_SENTINEL.png"), image_bytes).unwrap();
    let encoded_image = BASE64_STANDARD.encode(image_bytes);
    let transport = CapacityOneTransport::new(vision_round_responses());
    let prompter = AllowingPrompter::default();
    let target = NetworkTarget {
        scheme: "https".to_owned(),
        host: "vision-gateway.machine-god.dev".to_owned(),
        port: Some(8443),
    };
    let host = NativeReferenceHost::compose_with_ai_gateway_transport(
        built_in_config(),
        Arc::new(transport.clone()),
        target.clone(),
        &workspace,
        &sessions,
        Arc::new(prompter.clone()),
        inert_question_prompter(),
        never_deadline(),
    )
    .unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let session_id = SessionId::new("capacity-one-vision").unwrap();
    let events = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), async {
            let session = host
                .engine()
                .create_session(
                    session_id.clone(),
                    SessionIncarnationId::new("capacity-one-vision-incarnation").unwrap(),
                )
                .unwrap();
            session
                .prompt("inspect the authorized image")
                .await
                .unwrap()
                .map(|event| event.map(|event| event.payload))
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .await
        .expect("capacity-one nested vision must not starve")
    });
    assert_completed(&events);

    let permission_requests = prompter.requests();
    assert_eq!(permission_requests.len(), 1);
    assert_eq!(
        permission_requests[0].capability,
        Capability::Vision {
            paths: vec!["nested/PRIVATE_PATH_SENTINEL.png".to_owned()],
            target,
        }
    );

    let requests = transport.requests();
    assert_eq!(requests.len(), 3);
    assert_exact_native_tool_catalog(&body(&requests[0]));
    let nested_request = body(&requests[1]);
    assert_eq!(nested_request["tools"], json!([]));
    assert_eq!(
        nested_request["prompt"][1]["content"][1]["data"],
        encoded_image
    );
    let nested_serialized = serde_json::to_string(&nested_request).unwrap();
    assert!(!nested_serialized.contains("PRIVATE_PATH_SENTINEL"));
    assert!(!nested_serialized.contains("PRIVATE_IMAGE_BYTES_SENTINEL"));

    let outer_continuation = body(&requests[2]);
    assert_exact_native_tool_catalog(&outer_continuation);
    let structured_output = decoded_tool_output(&outer_continuation, 2);
    assert_eq!(
        structured_output,
        json!({
            "content": {
                "images": [{
                    "image_id": 1,
                    "status": "ok",
                    "summary": "The status indicator is ready.",
                    "visible_text": ["READY"],
                    "details": ["The indicator is green."]
                }]
            },
            "is_error": false
        })
    );
    let output_serialized = serde_json::to_string(&structured_output).unwrap();
    assert!(!output_serialized.contains("PRIVATE_PATH_SENTINEL"));
    assert!(!output_serialized.contains("PRIVATE_IMAGE_BYTES_SENTINEL"));
    assert!(!output_serialized.contains(&encoded_image));
    let outer_serialized = serde_json::to_string(&outer_continuation).unwrap();
    assert!(!outer_serialized.contains("PRIVATE_IMAGE_BYTES_SENTINEL"));
    assert!(!outer_serialized.contains(&encoded_image));

    let durable = futures_executor::block_on(host.engine().load_session(session_id))
        .unwrap()
        .expect("vision turn is durable");
    let durable_record = durable.record();
    let [ContentBlock::ToolResult { output, .. }] = durable_record.messages[2].content.as_slice()
    else {
        panic!("vision result is retained as one structured tool result")
    };
    assert_eq!(output.content, structured_output["content"]);
    let durable_result = serde_json::to_string(output).unwrap();
    assert!(!durable_result.contains("PRIVATE_PATH_SENTINEL"));
    assert!(!durable_result.contains("PRIVATE_IMAGE_BYTES_SENTINEL"));
    assert!(!durable_result.contains(&encoded_image));
    let durable_serialized = serde_json::to_string(&durable_record).unwrap();
    assert!(!durable_serialized.contains("PRIVATE_IMAGE_BYTES_SENTINEL"));
    assert!(!durable_serialized.contains(&encoded_image));
}

#[test]
fn composed_host_routes_rootless_questions_without_permission_policy() {
    let temporary = TemporaryDirectory::new("composed-question");
    let (workspace, sessions) = roots(temporary.path());
    let transport = ScriptedTransport::new(
        "COMPOSED_QUESTION_TRANSPORT_SENTINEL",
        ask_user_question_round_responses(),
    );
    let permission_prompter = AllowingPrompter::default();
    let question_prompter = AnsweringQuestionPrompter::default();
    let host = NativeReferenceHost::compose_with_ai_gateway_transport(
        built_in_config(),
        Arc::new(transport.clone()),
        production_gateway_target(),
        &workspace,
        &sessions,
        Arc::new(permission_prompter.clone()),
        Arc::new(question_prompter.clone()),
        never_deadline(),
    )
    .unwrap();

    let (_, events) = collect_turn(&host, "composed-question");
    assert!(events.iter().all(|event| !matches!(
        event,
        TurnEvent::PermissionRequested { .. } | TurnEvent::PermissionResolved { .. }
    )));
    assert!(permission_prompter.requests().is_empty());
    assert_eq!(
        question_prompter.questions.lock().unwrap().as_slice(),
        [vec!["Which path?".to_owned()]]
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_exact_native_tool_catalog(&body(&requests[0]));
    let result = decoded_tool_output(&body(&requests[1]), 2);
    assert_eq!(
        result,
        json!({
            "content": [{
                "answer": "a free-form answer",
                "question": "Which path?"
            }],
            "is_error": false
        })
    );
}

#[test]
fn v1_projection_composes_without_migrating_observable_loaded_schema() {
    let temporary = TemporaryDirectory::new("v1-projection");
    let (workspace, sessions) = roots(temporary.path());
    let loaded = load_v1_config(temporary.path());
    let transport = ScriptedTransport::new("V1_FACTORY_SENTINEL", Vec::<Vec<u8>>::new());
    let prompter = AllowingPrompter::default();

    let host = compose_with_transport(
        loaded,
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();

    assert_eq!(host.loaded_config().origin(), ConfigOrigin::File);
    assert_eq!(host.loaded_config().config().schema_version(), 1);
    assert_eq!(
        host.loaded_config().config().model(),
        AI_GATEWAY_DEFAULT_MODEL
    );
    assert_eq!(host.credential_source(), None);
    assert!(transport.requests().is_empty());
    assert!(prompter.requests().is_empty());
    assert!(directory_is_empty(&sessions));
    assert_eq!(host.engine().provider().name(), "vercel_ai_gateway");
    let engine = host.into_engine();
    assert_eq!(engine.provider().name(), "vercel_ai_gateway");
}

#[test]
fn composed_terminal_inspect_reads_exact_record_without_permission_or_supervisor_start() {
    let temporary = TemporaryDirectory::new("terminal-inspect");
    let (workspace, sessions) = roots(temporary.path());
    let workspace = fs::canonicalize(workspace).unwrap();
    write_background_record(
        &sessions,
        &workspace,
        17,
        20,
        "exited",
        None,
        Some(0),
        "PRIVATE_REFERENCE_HOST_COMMAND",
    );

    let transport = ScriptedTransport::new(
        "TERMINAL_INSPECT_FACTORY_SENTINEL",
        terminal_inspect_round_responses(),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();
    let retained_sessions = temporary.path().join("retained-sessions");
    fs::rename(&sessions, &retained_sessions).unwrap();
    fs::create_dir(&sessions).unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    write_background_record(
        &sessions,
        &workspace,
        17,
        30,
        "failed",
        Some(999),
        Some(7),
        "REPLACEMENT_COMMAND",
    );
    let (_, events) = collect_turn(&host, "composed-terminal-inspect");

    assert_completed(&events);
    assert!(prompter.requests().is_empty());
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        decoded_tool_output(&body(&requests[1]), 2),
        json!({
            "content": {
                "action": "inspect",
                "background_id": 17,
                "recorded_state": "exited",
                "started_at_ms": 10,
                "updated_at_ms": 20,
                "pid": null,
                "exit_code": 0
            },
            "is_error": false
        })
    );
    assert!(
        !body(&requests[1])
            .to_string()
            .contains("PRIVATE_REFERENCE_HOST_COMMAND")
    );
}

#[test]
fn composed_terminal_list_reads_ordered_private_free_records_from_retained_state_root() {
    let temporary = TemporaryDirectory::new("terminal-list");
    let (workspace, sessions) = roots(temporary.path());
    let workspace = fs::canonicalize(workspace).unwrap();
    write_background_record(
        &sessions,
        &workspace,
        17,
        20,
        "exited",
        None,
        Some(0),
        "PRIVATE_OLDER_LIST_COMMAND",
    );
    write_background_record(
        &sessions,
        &workspace,
        19,
        30,
        "running",
        Some(1234),
        None,
        "PRIVATE_NEWER_LIST_COMMAND",
    );

    let transport = ScriptedTransport::new(
        "TERMINAL_LIST_FACTORY_SENTINEL",
        terminal_list_round_responses(),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();
    let retained_sessions = temporary.path().join("retained-list-sessions");
    fs::rename(&sessions, &retained_sessions).unwrap();
    fs::create_dir(&sessions).unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    write_background_record(
        &sessions,
        &workspace,
        23,
        40,
        "failed",
        Some(999),
        Some(7),
        "PRIVATE_REPLACEMENT_LIST_COMMAND",
    );

    let (_, events) = collect_turn(&host, "composed-terminal-list");

    assert_completed(&events);
    assert!(prompter.requests().is_empty());
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_exact_native_tool_catalog(&body(&requests[0]));
    assert_eq!(
        decoded_tool_output(&body(&requests[1]), 2),
        json!({
            "content": {
                "action": "list",
                "count": 2,
                "truncated": false,
                "records": [
                    {
                        "background_id": 19,
                        "recorded_state": "running",
                        "updated_at_ms": 30
                    },
                    {
                        "background_id": 17,
                        "recorded_state": "exited",
                        "updated_at_ms": 20
                    }
                ]
            },
            "is_error": false
        })
    );
    let request = body(&requests[1]).to_string();
    assert!(!request.contains("PRIVATE_OLDER_LIST_COMMAND"));
    assert!(!request.contains("PRIVATE_NEWER_LIST_COMMAND"));
    assert!(!request.contains("PRIVATE_REPLACEMENT_LIST_COMMAND"));
}

#[test]
fn composed_terminal_list_treats_missing_history_as_an_empty_complete_result() {
    let temporary = TemporaryDirectory::new("terminal-list-empty");
    let (workspace, sessions) = roots(temporary.path());
    let workspace = fs::canonicalize(workspace).unwrap();
    let transport = ScriptedTransport::new(
        "TERMINAL_LIST_EMPTY_FACTORY_SENTINEL",
        terminal_list_round_responses(),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();

    let (_, events) = collect_turn(&host, "composed-terminal-list-empty");

    assert_completed(&events);
    assert!(prompter.requests().is_empty());
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        decoded_tool_output(&body(&requests[1]), 2),
        json!({
            "content": {
                "action": "list",
                "count": 0,
                "truncated": false,
                "records": []
            },
            "is_error": false
        })
    );
}

#[test]
fn composed_terminal_list_rejects_a_zero_id_persisted_record_without_disclosure() {
    let temporary = TemporaryDirectory::new("terminal-list-zero-id");
    let (workspace, sessions) = roots(temporary.path());
    let workspace = fs::canonicalize(workspace).unwrap();
    write_background_record(
        &sessions,
        &workspace,
        0,
        20,
        "exited",
        None,
        Some(0),
        "PRIVATE_ZERO_ID_REFERENCE_HOST_COMMAND",
    );
    let transport = ScriptedTransport::new(
        "TERMINAL_LIST_ZERO_ID_FACTORY_SENTINEL",
        terminal_list_round_responses(),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();

    let (_, events) = collect_turn(&host, "composed-terminal-list-zero-id");

    assert_completed(&events);
    assert!(prompter.requests().is_empty());
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        decoded_tool_output(&body(&requests[1]), 2),
        json!({
            "content": {
                "code": "tool_error",
                "message": "tool execution failed",
                "retryable": false
            },
            "is_error": true
        })
    );
    assert!(
        !body(&requests[1])
            .to_string()
            .contains("PRIVATE_ZERO_ID_REFERENCE_HOST_COMMAND")
    );
}

#[test]
fn composed_terminal_wait_reads_retained_terminal_record_without_permission_or_supervisor_start() {
    let temporary = TemporaryDirectory::new("terminal-wait-complete");
    let (workspace, sessions) = roots(temporary.path());
    let workspace = fs::canonicalize(workspace).unwrap();
    write_background_record(
        &sessions,
        &workspace,
        17,
        20,
        "exited",
        None,
        Some(0),
        "PRIVATE_WAIT_COMMAND",
    );

    let transport = ScriptedTransport::new(
        "TERMINAL_WAIT_FACTORY_SENTINEL",
        terminal_wait_round_responses(17, 5_000),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();
    let retained_sessions = temporary.path().join("retained-wait-sessions");
    fs::rename(&sessions, &retained_sessions).unwrap();
    fs::create_dir(&sessions).unwrap();
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    write_background_record(
        &sessions,
        &workspace,
        17,
        30,
        "failed",
        Some(999),
        Some(7),
        "REPLACEMENT_WAIT_COMMAND",
    );

    let (_, events) = collect_turn(&host, "composed-terminal-wait-complete");

    assert_completed(&events);
    assert!(prompter.requests().is_empty());
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_exact_native_tool_catalog(&body(&requests[0]));
    assert_eq!(
        decoded_tool_output(&body(&requests[1]), 2),
        json!({
            "content": {
                "action": "wait",
                "background_id": 17,
                "outcome": {"exited": 0},
                "recorded_state": "exited",
                "started_at_ms": 10,
                "updated_at_ms": 20,
                "pid": null,
                "exit_code": 0
            },
            "is_error": false
        })
    );
    let request = body(&requests[1]).to_string();
    assert!(!request.contains("PRIVATE_WAIT_COMMAND"));
    assert!(!request.contains("REPLACEMENT_WAIT_COMMAND"));
}

#[test]
fn composed_terminal_wait_reaches_safety_ceiling_without_reconciling_running_record() {
    let temporary = TemporaryDirectory::new("terminal-wait-ceiling");
    let (workspace, sessions) = roots(temporary.path());
    let workspace = fs::canonicalize(workspace).unwrap();
    write_background_record(
        &sessions,
        &workspace,
        18,
        20,
        "running",
        Some(1234),
        None,
        "PRIVATE_RUNNING_COMMAND",
    );

    let transport = ScriptedTransport::new(
        "TERMINAL_WAIT_CEILING_FACTORY_SENTINEL",
        terminal_wait_round_responses(18, 50),
    );
    let prompter = AllowingPrompter::default();
    let deadline = SleepingDeadline::default();
    let host = compose_with_transport_and_deadline(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
        Arc::new(deadline.clone()),
    )
    .unwrap();

    let (_, events) = collect_turn(&host, "composed-terminal-wait-ceiling");

    assert_completed(&events);
    assert!(prompter.requests().is_empty());
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        decoded_tool_output(&body(&requests[1]), 2),
        json!({
            "content": {
                "action": "wait",
                "background_id": 18,
                "outcome": {"safety_ceiling": {}},
                "recorded_state": "running",
                "started_at_ms": 10,
                "updated_at_ms": 20,
                "pid": 1234,
                "exit_code": null
            },
            "is_error": false
        })
    );
    assert_eq!(read_background_state(&sessions, &workspace, 18), "running");
    assert!(deadline.calls() >= 1);
    assert!(
        !body(&requests[1])
            .to_string()
            .contains("PRIVATE_RUNNING_COMMAND")
    );
}

#[test]
fn production_http_constructor_selects_oidc_then_api_key_without_runtime_effects() {
    let temporary = TemporaryDirectory::new("production-credentials");
    let (workspace, sessions) = roots(temporary.path());
    let prompter = AllowingPrompter::default();
    let oidc = "OIDC_CONSTRUCTOR_TOKEN_SENTINEL";
    let ignored_api_key = "IGNORED_API_KEY_TOKEN_SENTINEL";
    let host = NativeReferenceHost::compose_ai_gateway_http(
        built_in_config(),
        AiGatewayCredentialEnvironment::new(
            Some(OsString::from(oidc)),
            Some(OsString::from(ignored_api_key)),
        ),
        &workspace,
        &sessions,
        Arc::new(prompter.clone()),
        inert_question_prompter(),
        never_deadline(),
    )
    .unwrap();
    assert_eq!(
        host.credential_source(),
        Some(AiGatewayCredentialSource::VercelOidcToken)
    );
    let debug = format!("{host:?}");
    assert!(!debug.contains(oidc));
    assert!(!debug.contains(ignored_api_key));
    assert!(prompter.requests().is_empty());
    assert!(directory_is_empty(&sessions));
    drop(host);

    let fallback = "FALLBACK_API_KEY_TOKEN_SENTINEL";
    let host = NativeReferenceHost::compose_ai_gateway_http(
        built_in_config(),
        AiGatewayCredentialEnvironment::new(Some(OsString::new()), Some(OsString::from(fallback))),
        &workspace,
        &sessions,
        Arc::new(prompter.clone()),
        inert_question_prompter(),
        never_deadline(),
    )
    .unwrap();
    assert_eq!(
        host.credential_source(),
        Some(AiGatewayCredentialSource::AiGatewayApiKey)
    );
    assert!(!format!("{host:?}").contains(fallback));
    assert!(prompter.requests().is_empty());
    assert!(directory_is_empty(&sessions));
}

#[test]
fn missing_or_invalid_selected_credential_fails_after_valid_roots_without_fallback() {
    let temporary = TemporaryDirectory::new("credential-errors");
    let (workspace, sessions) = roots(temporary.path());
    let prompter = AllowingPrompter::default();

    let missing = build_error(NativeReferenceHost::compose_ai_gateway_http(
        built_in_config(),
        AiGatewayCredentialEnvironment::new(None, None),
        &workspace,
        &sessions,
        Arc::new(prompter.clone()),
        inert_question_prompter(),
        never_deadline(),
    ));
    assert_eq!(
        missing.kind(),
        NativeReferenceHostBuildErrorKind::Credential
    );

    let invalid_token_marker = "INVALID_SELECTED_OIDC_TOKEN_SENTINEL";
    let selected_invalid = format!("{invalid_token_marker}\n");
    let valid_fallback = "VALID_FALLBACK_TOKEN_SENTINEL";
    let invalid = build_error(NativeReferenceHost::compose_ai_gateway_http(
        built_in_config(),
        AiGatewayCredentialEnvironment::new(
            Some(OsString::from(selected_invalid.as_str())),
            Some(OsString::from(valid_fallback)),
        ),
        &workspace,
        &sessions,
        Arc::new(prompter.clone()),
        inert_question_prompter(),
        never_deadline(),
    ));
    assert_eq!(
        invalid.kind(),
        NativeReferenceHostBuildErrorKind::Credential
    );
    assert_eq!(invalid.to_string(), missing.to_string());
    assert_stage_debug(invalid, NativeReferenceHostBuildErrorKind::Credential);
    assert_redacted(invalid, &[invalid_token_marker, valid_fallback]);

    let invalid_os_marker = "NON_UNICODE_OS_CREDENTIAL_SENTINEL";
    let mut invalid_os = invalid_os_marker.as_bytes().to_vec();
    invalid_os.push(0xff);
    let invalid_environment = build_error(NativeReferenceHost::compose_ai_gateway_http(
        built_in_config(),
        AiGatewayCredentialEnvironment::new(
            Some(OsString::from_vec(invalid_os)),
            Some(OsString::from(valid_fallback)),
        ),
        &workspace,
        &sessions,
        Arc::new(prompter.clone()),
        inert_question_prompter(),
        never_deadline(),
    ));
    assert_eq!(
        invalid_environment.kind(),
        NativeReferenceHostBuildErrorKind::Credential
    );
    assert_eq!(invalid_environment.to_string(), missing.to_string());
    assert_stage_debug(
        invalid_environment,
        NativeReferenceHostBuildErrorKind::Credential,
    );
    assert_redacted(invalid_environment, &[invalid_os_marker, valid_fallback]);
    assert!(prompter.requests().is_empty());
    assert!(directory_is_empty(&sessions));
}

#[test]
fn public_session_root_is_rejected_as_background_configuration_without_effects() {
    let temporary = TemporaryDirectory::new("public-background-root");
    let (workspace, sessions) = roots(temporary.path());
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o755)).unwrap();
    let transport = ScriptedTransport::new(
        "PUBLIC_BACKGROUND_ROOT_TRANSPORT_SENTINEL",
        Vec::<Vec<u8>>::new(),
    );
    let prompter = AllowingPrompter::default();

    let error = build_error(compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    ));

    assert_eq!(
        error.kind(),
        NativeReferenceHostBuildErrorKind::BackgroundConfig
    );
    assert_stage_debug(error, NativeReferenceHostBuildErrorKind::BackgroundConfig);
    assert!(transport.requests().is_empty());
    assert!(prompter.requests().is_empty());
    assert!(directory_is_empty(&sessions));
}

#[cfg(target_os = "macos")]
#[test]
fn session_root_acl_is_rejected_as_background_configuration_without_effects() {
    let temporary = TemporaryDirectory::new("acl-background-root");
    let (workspace, sessions) = roots(temporary.path());
    let status = Command::new("/bin/chmod")
        .args(["+a", "everyone allow search"])
        .arg(&sessions)
        .status()
        .expect("macOS chmod executable is available");
    assert!(status.success(), "failed to install ACL fixture: {status}");
    let transport = ScriptedTransport::new(
        "ACL_BACKGROUND_ROOT_TRANSPORT_SENTINEL",
        Vec::<Vec<u8>>::new(),
    );
    let prompter = AllowingPrompter::default();

    let error = build_error(compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    ));

    assert_eq!(
        error.kind(),
        NativeReferenceHostBuildErrorKind::BackgroundConfig
    );
    assert_stage_debug(error, NativeReferenceHostBuildErrorKind::BackgroundConfig);
    assert!(transport.requests().is_empty());
    assert!(prompter.requests().is_empty());
    assert!(directory_is_empty(&sessions));
    let cleanup = Command::new("/bin/chmod")
        .arg("-N")
        .arg(&sessions)
        .status()
        .expect("macOS chmod executable is available");
    assert!(cleanup.success(), "failed to clear ACL fixture: {cleanup}");
}

#[test]
fn every_invalid_root_shape_fails_at_its_stage_before_credential_handoff() {
    let temporary = TemporaryDirectory::new("root-errors");
    let valid_workspace = temporary.path().join("valid-workspace");
    let valid_sessions = temporary.path().join("valid-sessions");
    fs::create_dir(&valid_workspace).unwrap();
    fs::create_dir(&valid_sessions).unwrap();

    let missing_workspace = temporary.path().join("MISSING_WORKSPACE_ROOT_SENTINEL");
    let workspace_file = temporary.path().join("WORKSPACE_FILE_ROOT_SENTINEL");
    fs::write(&workspace_file, b"not a directory").unwrap();
    let workspace_link = temporary.path().join("WORKSPACE_SYMLINK_ROOT_SENTINEL");
    symlink(&valid_workspace, &workspace_link).unwrap();
    let workspace_cases = [
        PathBuf::from("RELATIVE_WORKSPACE_ROOT_SENTINEL"),
        missing_workspace,
        workspace_file,
        workspace_link,
    ];
    let mut workspace_message = None;
    for path in &workspace_cases {
        let error = build_error(NativeReferenceHost::compose_ai_gateway_http(
            built_in_config(),
            AiGatewayCredentialEnvironment::new(None, None),
            path,
            &valid_sessions,
            Arc::new(AllowingPrompter::default()),
            inert_question_prompter(),
            never_deadline(),
        ));
        assert_eq!(
            error.kind(),
            NativeReferenceHostBuildErrorKind::WorkspaceRoot
        );
        assert_stage_debug(error, NativeReferenceHostBuildErrorKind::WorkspaceRoot);
        if let Some(expected) = &workspace_message {
            assert_eq!(&error.to_string(), expected);
        } else {
            workspace_message = Some(error.to_string());
        }
        let path_sentinel = path.to_string_lossy();
        assert_redacted(error, &[path_sentinel.as_ref()]);
    }

    let missing_sessions = temporary.path().join("MISSING_SESSION_ROOT_SENTINEL");
    let session_file = temporary.path().join("SESSION_FILE_ROOT_SENTINEL");
    fs::write(&session_file, b"not a directory").unwrap();
    let session_link = temporary.path().join("SESSION_SYMLINK_ROOT_SENTINEL");
    symlink(&valid_sessions, &session_link).unwrap();
    let session_cases = [
        PathBuf::from("RELATIVE_SESSION_ROOT_SENTINEL"),
        missing_sessions,
        session_file,
        session_link,
    ];
    let mut session_message = None;
    for path in &session_cases {
        let error = build_error(NativeReferenceHost::compose_ai_gateway_http(
            built_in_config(),
            AiGatewayCredentialEnvironment::new(None, None),
            &valid_workspace,
            path,
            Arc::new(AllowingPrompter::default()),
            inert_question_prompter(),
            never_deadline(),
        ));
        assert_eq!(
            error.kind(),
            NativeReferenceHostBuildErrorKind::SessionStore
        );
        assert_stage_debug(error, NativeReferenceHostBuildErrorKind::SessionStore);
        if let Some(expected) = &session_message {
            assert_eq!(&error.to_string(), expected);
        } else {
            session_message = Some(error.to_string());
        }
        let path_sentinel = path.to_string_lossy();
        assert_redacted(error, &[path_sentinel.as_ref()]);
    }
}

#[test]
fn injected_construction_is_inert_and_host_debug_redacts_owned_inputs() {
    let temporary = TemporaryDirectory::new("CONSTRUCTION_ROOT_SENTINEL");
    let (workspace, sessions) = roots(temporary.path());
    let model = "CONSTRUCTION_MODEL_SENTINEL";
    let factory = "CONSTRUCTION_FACTORY_SENTINEL";
    let transport = ScriptedTransport::new(factory, Vec::<Vec<u8>>::new());
    let prompter = AllowingPrompter::default();

    let host = compose_with_transport(
        load_v2_config(temporary.path(), model),
        transport.clone(),
        &workspace,
        &sessions,
        prompter.clone(),
    )
    .unwrap();

    assert!(transport.requests().is_empty());
    assert!(prompter.requests().is_empty());
    assert!(directory_is_empty(&sessions));
    let debug = format!("{host:?}");
    for sentinel in ["CONSTRUCTION_ROOT_SENTINEL", model, factory] {
        assert!(
            !debug.contains(sentinel),
            "host Debug leaked {sentinel:?}: {debug:?}"
        );
    }
}

fn assert_replacement_sentinels_are_redacted(request: &Value) {
    let serialized = serde_json::to_string(request).unwrap();
    assert!(!serialized.contains("REPLACEMENT_FILE_CONTENT_SENTINEL"));
    assert!(!serialized.contains("replacement-only.txt"));
    assert!(!serialized.contains("REPLACEMENT_DELETE_DECOY_SENTINEL"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn replacing_original_workspace_path_cannot_redirect_any_registered_tool() {
    let temporary = TemporaryDirectory::new("retained-workspace");
    let (workspace, sessions) = roots(temporary.path());
    let nested = workspace.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("note.txt"), "RETAINED_FILE_CONTENT_SENTINEL").unwrap();
    fs::write(nested.join("retained-only.txt"), b"retained").unwrap();
    let transport = ScriptedTransport::new(
        "RETAINED_FACTORY_SENTINEL",
        tool_round_responses("retained identity complete"),
    );
    let prompter = AllowingPrompter::default();
    let host = compose_with_transport(
        built_in_config(),
        transport.clone(),
        &workspace,
        &sessions,
        prompter,
    )
    .unwrap();

    let retained = temporary.path().join("renamed-retained-workspace");
    fs::rename(&workspace, &retained).unwrap();
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("nested")).unwrap();
    fs::write(
        workspace.join("nested/note.txt"),
        "REPLACEMENT_FILE_CONTENT_SENTINEL",
    )
    .unwrap();
    fs::write(workspace.join("nested/replacement-only.txt"), b"decoy").unwrap();
    fs::write(
        workspace.join("nested/generated.txt"),
        b"REPLACEMENT_DELETE_DECOY_SENTINEL",
    )
    .unwrap();

    let (_, events) = collect_turn(&host, "retained-root-identity");
    assert_completed(&events);
    let requests = transport.requests();
    assert_eq!(requests.len(), 5);
    let final_request = body(&requests[4]);
    let list_output = decoded_tool_output(&final_request, 2);
    let read_output = decoded_tool_output(&final_request, 3);
    let info_output = decoded_tool_output(&final_request, 4);
    let glob_output = decoded_tool_output(&final_request, 5);
    let grep_output = decoded_tool_output(&final_request, 7);
    let write_output = decoded_tool_output(&final_request, 8);
    let edit_output = decoded_tool_output(&final_request, 9);
    let rename_output = decoded_tool_output(&final_request, 10);
    let copy_output = decoded_tool_output(&final_request, 12);
    let create_output = decoded_tool_output(&final_request, 13);
    let delete_copy_output = decoded_tool_output(&final_request, 15);
    let delete_source_output = decoded_tool_output(&final_request, 16);
    assert_eq!(
        list_output["content"]["entries"],
        json!([
            {"name": "note.txt", "kind": "file"},
            {"name": "retained-only.txt", "kind": "file"}
        ])
    );
    assert_eq!(
        read_output["content"]["content"],
        "RETAINED_FILE_CONTENT_SENTINEL"
    );
    assert_eq!(info_output["content"]["path"], "nested/note.txt");
    assert_eq!(info_output["content"]["kind"], "file");
    assert_eq!(
        info_output["content"]["size_bytes"],
        "RETAINED_FILE_CONTENT_SENTINEL".len()
    );
    assert_eq!(
        glob_output["content"]["matches"],
        json!(["nested/note.txt", "nested/retained-only.txt"])
    );
    assert_eq!(grep_output["content"]["matching_lines"], 1);
    assert_eq!(grep_output["content"]["matching_files"], 1);
    assert_eq!(write_output["content"]["path"], "nested/generated.txt");
    assert_eq!(
        write_output["content"]["bytes_written"],
        "generated retained content".len()
    );
    assert_eq!(edit_output["content"]["path"], "nested/generated.txt");
    assert_eq!(
        edit_output["content"]["bytes_written"],
        "generated edited content".len()
    );
    assert_eq!(
        rename_output["content"],
        json!({
            "old_path": "nested/generated.txt",
            "new_path": "nested/renamed.txt"
        })
    );
    assert_eq!(
        copy_output["content"],
        json!({
            "source": "nested/renamed.txt",
            "destination": "nested/copied.txt",
            "bytes_copied": "generated edited content".len()
        })
    );
    assert_eq!(create_output["content"], json!({"path": "nested/created"}));
    assert_eq!(
        delete_copy_output["content"],
        json!({"path": "nested/copied.txt"})
    );
    assert_eq!(
        delete_source_output["content"],
        json!({"path": "nested/renamed.txt"})
    );
    assert!(!retained.join("nested/generated.txt").exists());
    assert!(!retained.join("nested/copied.txt").exists());
    assert!(!retained.join("nested/renamed.txt").exists());
    assert!(retained.join("nested/created").is_dir());
    assert!(!workspace.join("nested/created").exists());
    assert_eq!(
        fs::read(workspace.join("nested/generated.txt")).unwrap(),
        b"REPLACEMENT_DELETE_DECOY_SENTINEL"
    );
    assert_replacement_sentinels_are_redacted(&final_request);
}
