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
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use futures_util::{StreamExt, stream};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ContentBlock, FilesystemAccess, NetworkTarget,
    PermissionRequest, Role, SessionId, SessionIncarnationId, StopReason, TurnEvent,
};
use machine_god_native::{
    AI_GATEWAY_DEFAULT_MODEL, AiGatewayByteStream, AiGatewayCredentialEnvironment,
    AiGatewayCredentialSource, AiGatewayTransport, AiGatewayTransportRequest, COPY_FILE_TOOL_NAME,
    CREATE_FOLDER_TOOL_NAME, ConfigOrigin, DELETE_FILE_TOOL_NAME, EDIT_FILE_TOOL_NAME,
    FILE_INFO_TOOL_NAME, GLOB_FILES_TOOL_NAME, GREP_FILES_TOOL_NAME, LIST_FILES_TOOL_NAME,
    LoadedNativeConfig, NativeEnvironment, NativeReferenceHost, NativeReferenceHostBuildError,
    NativeReferenceHostBuildErrorKind, OPEN_FILE_TOOL_NAME, PermissionPromptDecision,
    PermissionPromptError, PermissionPrompter, READ_FILE_TOOL_NAME, RENAME_FILE_TOOL_NAME,
    TERMINAL_TOOL_NAME, WEB_FETCH_TOOL_NAME, WEB_SEARCH_TOOL_NAME, WRITE_FILE_TOOL_NAME,
    load_native_config,
};
use serde_json::{Value, json};
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

fn compose_with_transport(
    loaded: LoadedNativeConfig,
    transport: ScriptedTransport,
    workspace: &Path,
    sessions: &Path,
    prompter: AllowingPrompter,
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
        never_deadline(),
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
    assert_eq!(tools.len(), 15);
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            COPY_FILE_TOOL_NAME,
            CREATE_FOLDER_TOOL_NAME,
            DELETE_FILE_TOOL_NAME,
            EDIT_FILE_TOOL_NAME,
            FILE_INFO_TOOL_NAME,
            GLOB_FILES_TOOL_NAME,
            GREP_FILES_TOOL_NAME,
            LIST_FILES_TOOL_NAME,
            OPEN_FILE_TOOL_NAME,
            READ_FILE_TOOL_NAME,
            RENAME_FILE_TOOL_NAME,
            TERMINAL_TOOL_NAME,
            WEB_FETCH_TOOL_NAME,
            WEB_SEARCH_TOOL_NAME,
            WRITE_FILE_TOOL_NAME
        ]
    );
    assert!(tools.iter().all(|tool| tool["type"] == "function"));
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
