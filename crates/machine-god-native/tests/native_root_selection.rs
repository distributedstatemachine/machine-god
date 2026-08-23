#![cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{StreamExt, stream};
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionRequest, SessionId, SessionIncarnationId, StopReason,
    TurnEvent,
};
use machine_god_native::{
    AiGatewayByteStream, AiGatewayCredentialEnvironment, AiGatewayCredentialSource,
    AiGatewayTransport, AiGatewayTransportRequest, NativeEnvironment, NativeReferenceHost,
    NativeReferenceHostBuildError, NativeReferenceHostBuildErrorKind, NativeRootSelection,
    PermissionPromptDecision, PermissionPromptError, PermissionPrompter, PreparedNativeRoots,
    load_native_config,
};
use serde_json::Value;

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-native-roots-{label}-{}-{identifier}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    set_mode(&path, 0o700);
                    return Self { path };
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("failed to create temporary directory: {error}"),
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
            Err(error) => panic!("failed to remove temporary directory: {error}"),
        }
    }
}

fn set_mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}

fn create_private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    set_mode(path, 0o700);
}

struct TransportState {
    responses: VecDeque<Vec<u8>>,
    request_bodies: Vec<Vec<u8>>,
}

#[derive(Clone)]
struct ScriptedTransport {
    state: Arc<Mutex<TransportState>>,
}

impl ScriptedTransport {
    fn new(responses: impl IntoIterator<Item = impl Into<Vec<u8>>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(TransportState {
                responses: responses.into_iter().map(Into::into).collect(),
                request_bodies: Vec::new(),
            })),
        }
    }

    fn request_bodies(&self) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().request_bodies.clone()
    }
}

impl AiGatewayTransport for ScriptedTransport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
        let (_, body) = request.into_parts();
        let response = {
            let mut state = self.state.lock().unwrap();
            state.request_bodies.push(body);
            state.responses.pop_front().expect("scripted response")
        };
        Box::pin(async move { Ok(Box::pin(stream::iter([Ok(response)])) as AiGatewayByteStream) })
    }
}

#[derive(Clone, Default)]
struct RecordingPrompter {
    requests: Arc<Mutex<Vec<PermissionRequest>>>,
}

impl RecordingPrompter {
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl PermissionPrompter for RecordingPrompter {
    fn prompt(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async { Ok(PermissionPromptDecision::AllowOnce) })
    }
}

fn built_in_config() -> machine_god_native::LoadedNativeConfig {
    load_native_config(&NativeEnvironment::new(None, None, None)).unwrap()
}

fn prepared_roots(base: &Path) -> (PreparedNativeRoots, PathBuf, PathBuf) {
    let workspace = base.join("workspace");
    let state_base = base.join("state-base");
    create_private_dir(&workspace);
    create_private_dir(&state_base);
    let selection = NativeRootSelection::from_environment(
        &NativeEnvironment::new(None, Some(state_base.into_os_string()), None),
        &workspace,
    )
    .unwrap();
    let state_root = selection.state_root().to_path_buf();
    let prepared = PreparedNativeRoots::prepare(selection).unwrap();
    (prepared, workspace, state_root)
}

fn directory_is_empty(path: &Path) -> bool {
    fs::read_dir(path).unwrap().next().is_none()
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

fn collect_turn(host: &NativeReferenceHost) -> Vec<TurnEvent> {
    let session = host
        .engine()
        .create_session(
            SessionId::new("retained-native-roots").unwrap(),
            SessionIncarnationId::new("retained-native-roots-incarnation").unwrap(),
        )
        .unwrap();
    futures_executor::block_on(async {
        session
            .prompt("read the retained file")
            .await
            .unwrap()
            .map(|event| event.map(|event| event.payload))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
}

fn retained_root_responses() -> [Vec<u8>; 3] {
    let first = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"read-call\",\"toolName\":\"read_file\",\"input\":{\"path\":\"note.txt\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"info-call\",\"toolName\":\"file_info\",\"input\":{\"path\":\"note.txt\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"glob-call\",\"toolName\":\"glob_files\",\"input\":{\"pattern\":\"*.txt\"}}\n\n",
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"grep-call\",\"toolName\":\"grep_files\",\"input\":{\"pattern\":\"RETAINED_WORKSPACE_CONTENT_SENTINEL\",\"path\":\"note.txt\",\"mode\":\"count\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let second = concat!(
        "data: {\"type\":\"tool-call\",\"toolCallId\":\"write-call\",\"toolName\":\"write_file\",\"input\":{\"path\":\"generated.txt\",\"content\":\"RETAINED_GENERATED_CONTENT\"}}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"tool-calls\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    let third = concat!(
        "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"done\"}\n\n",
        "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n"
    )
    .as_bytes()
    .to_vec();
    [first, second, third]
}

#[test]
#[allow(clippy::too_many_lines)]
fn prepared_constructor_consumes_retained_workspace_and_state_identities() {
    let temporary = TemporaryDirectory::new("retained-identities");
    let (prepared, workspace, state_root) = prepared_roots(temporary.path());
    fs::write(
        workspace.join("note.txt"),
        "RETAINED_WORKSPACE_CONTENT_SENTINEL",
    )
    .unwrap();

    let retained_workspace = temporary.path().join("retained-workspace");
    fs::rename(&workspace, &retained_workspace).unwrap();
    create_private_dir(&workspace);
    fs::write(
        workspace.join("note.txt"),
        "REPLACEMENT_WORKSPACE_CONTENT_SENTINEL",
    )
    .unwrap();

    let retained_state = temporary.path().join("retained-state");
    fs::rename(&state_root, &retained_state).unwrap();
    create_private_dir(&state_root);

    let transport = ScriptedTransport::new(retained_root_responses());
    let prompter = RecordingPrompter::default();
    let host = NativeReferenceHost::compose_with_ai_gateway_transport_and_prepared_roots(
        built_in_config(),
        Arc::new(transport.clone()),
        prepared,
        Arc::new(prompter.clone()),
    )
    .unwrap();

    assert!(transport.request_bodies().is_empty());
    assert_eq!(prompter.count(), 0);
    assert!(directory_is_empty(&retained_state));
    assert!(directory_is_empty(&state_root));
    for forbidden in [
        "retained-identities",
        "RETAINED_WORKSPACE_CONTENT_SENTINEL",
        "REPLACEMENT_WORKSPACE_CONTENT_SENTINEL",
    ] {
        assert!(!format!("{host:?}").contains(forbidden));
    }

    let events = collect_turn(&host);
    assert!(matches!(
        events.last(),
        Some(TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        })
    ));
    assert_eq!(prompter.count(), 5);
    let requests = transport.request_bodies();
    assert_eq!(requests.len(), 3);
    let second_request = String::from_utf8(requests[1].clone()).unwrap();
    assert!(second_request.contains("RETAINED_WORKSPACE_CONTENT_SENTINEL"));
    assert!(!second_request.contains("REPLACEMENT_WORKSPACE_CONTENT_SENTINEL"));
    let second_request: Value = serde_json::from_slice(&requests[1]).unwrap();
    let encoded_info = second_request["prompt"][3]["content"][0]["output"]["value"]
        .as_str()
        .expect("file_info output is encoded as text");
    let info_output: Value = serde_json::from_str(encoded_info).unwrap();
    assert_eq!(info_output["content"]["path"], "note.txt");
    assert_eq!(info_output["content"]["kind"], "file");
    assert_eq!(
        info_output["content"]["size_bytes"],
        "RETAINED_WORKSPACE_CONTENT_SENTINEL".len()
    );
    let encoded_glob = second_request["prompt"][4]["content"][0]["output"]["value"]
        .as_str()
        .expect("glob_files output is encoded as text");
    let glob_output: Value = serde_json::from_str(encoded_glob).unwrap();
    assert_eq!(
        glob_output["content"],
        serde_json::json!({
            "path": ".",
            "pattern": "*.txt",
            "mode": "matches",
            "matches": ["note.txt"],
            "truncated": false
        })
    );
    let encoded_grep = second_request["prompt"][5]["content"][0]["output"]["value"]
        .as_str()
        .expect("grep_files output is encoded as text");
    let grep_output: Value = serde_json::from_str(encoded_grep).unwrap();
    assert_eq!(
        grep_output["content"],
        serde_json::json!({
            "pattern": "RETAINED_WORKSPACE_CONTENT_SENTINEL",
            "path": "note.txt",
            "include": null,
            "case_insensitive": false,
            "mode": "count",
            "head_limit": 100,
            "offset": 0,
            "context_lines": 0,
            "candidate_files": 1,
            "searched_files": 1,
            "skipped_oversized_files": 0,
            "skipped_non_text_files": 0,
            "matching_lines": 1,
            "matching_files": 1
        })
    );
    let third_request: Value = serde_json::from_slice(&requests[2]).unwrap();
    let encoded_write = third_request["prompt"][7]["content"][0]["output"]["value"]
        .as_str()
        .expect("write_file output is encoded as text");
    let write_output: Value = serde_json::from_str(encoded_write).unwrap();
    assert_eq!(
        write_output["content"],
        serde_json::json!({
            "path": "generated.txt",
            "bytes_written": "RETAINED_GENERATED_CONTENT".len()
        })
    );
    assert_eq!(
        fs::read(retained_workspace.join("generated.txt")).unwrap(),
        b"RETAINED_GENERATED_CONTENT"
    );
    assert!(!workspace.join("generated.txt").exists());
    assert!(!directory_is_empty(&retained_state));
    assert!(directory_is_empty(&state_root));
}

#[test]
fn prepared_production_constructor_discovers_credentials_only_after_preparation() {
    let temporary = TemporaryDirectory::new("prepared-production");
    let (prepared, _, state_root) = prepared_roots(temporary.path());
    let prompter = RecordingPrompter::default();

    let error = build_error(
        NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots(
            built_in_config(),
            AiGatewayCredentialEnvironment::new(None, None),
            prepared,
            Arc::new(prompter.clone()),
        ),
    );
    assert_eq!(error.kind(), NativeReferenceHostBuildErrorKind::Credential);
    assert_eq!(prompter.count(), 0);
    assert!(directory_is_empty(&state_root));

    let second = temporary.path().join("second");
    create_private_dir(&second);
    let (prepared, _, state_root) = prepared_roots(&second);
    let token = "PREPARED_PRODUCTION_TOKEN_SENTINEL";
    let host = NativeReferenceHost::compose_ai_gateway_http_with_prepared_roots(
        built_in_config(),
        AiGatewayCredentialEnvironment::new(Some(OsString::from(token)), None),
        prepared,
        Arc::new(prompter.clone()),
    )
    .unwrap();
    assert_eq!(
        host.credential_source(),
        Some(AiGatewayCredentialSource::VercelOidcToken)
    );
    assert!(!format!("{host:?}").contains(token));
    assert_eq!(prompter.count(), 0);
    assert!(directory_is_empty(&state_root));
}

#[test]
fn existing_path_constructors_remain_no_create_and_keep_root_before_credential_order() {
    let temporary = TemporaryDirectory::new("legacy-paths");
    let workspace = temporary.path().join("workspace");
    let sessions = temporary.path().join("sessions");
    create_private_dir(&workspace);
    create_private_dir(&sessions);
    let transport = ScriptedTransport::new(Vec::<Vec<u8>>::new());
    let prompter = RecordingPrompter::default();

    let host = NativeReferenceHost::compose_with_ai_gateway_transport(
        built_in_config(),
        Arc::new(transport.clone()),
        &workspace,
        &sessions,
        Arc::new(prompter.clone()),
    )
    .unwrap();
    assert!(transport.request_bodies().is_empty());
    assert_eq!(prompter.count(), 0);
    assert!(directory_is_empty(&sessions));
    drop(host);

    let missing_workspace = temporary.path().join("MISSING_WORKSPACE_SENTINEL");
    let workspace_error = build_error(NativeReferenceHost::compose_ai_gateway_http(
        built_in_config(),
        AiGatewayCredentialEnvironment::new(None, None),
        &missing_workspace,
        &sessions,
        Arc::new(prompter.clone()),
    ));
    assert_eq!(
        workspace_error.kind(),
        NativeReferenceHostBuildErrorKind::WorkspaceRoot
    );
    assert!(!missing_workspace.exists());
    assert!(directory_is_empty(&sessions));

    let missing_sessions = temporary.path().join("MISSING_SESSION_ROOT_SENTINEL");
    let session_error = build_error(NativeReferenceHost::compose_ai_gateway_http(
        built_in_config(),
        AiGatewayCredentialEnvironment::new(None, None),
        &workspace,
        &missing_sessions,
        Arc::new(prompter),
    ));
    assert_eq!(
        session_error.kind(),
        NativeReferenceHostBuildErrorKind::SessionStore
    );
    assert!(!missing_sessions.exists());
}
