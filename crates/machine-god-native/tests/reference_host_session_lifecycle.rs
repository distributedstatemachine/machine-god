#![cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use machine_god_core::{BoxFuture, CancellationToken, PermissionRequest, SessionId, SessionStore};
use machine_god_native::{
    AiGatewayByteStream, AiGatewayTransport, AiGatewayTransportRequest, NativeEnvironment,
    NativeReferenceHost, PermissionPromptDecision, PermissionPromptError, PermissionPrompter,
    QuestionPromptError, QuestionPromptOutcome, QuestionPromptRequest, QuestionPrompter,
    load_native_config,
};

mod web_search_support;

use web_search_support::{never_deadline, production_gateway_target};

static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> Self {
        for _ in 0..1_000 {
            let identifier = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mg-reference-host-session-lifecycle-{}-{identifier}",
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

#[derive(Debug)]
struct InertTransport;

impl AiGatewayTransport for InertTransport {
    fn stream(
        &self,
        _request: AiGatewayTransportRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
        panic!("session lifecycle construction and creation must not call the model transport")
    }
}

#[derive(Debug)]
struct InertPrompter;

impl PermissionPrompter for InertPrompter {
    fn prompt(
        &self,
        _request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        panic!("session lifecycle construction and creation must not prompt for permission")
    }
}

#[derive(Debug)]
struct InertQuestionPrompter;

impl QuestionPrompter for InertQuestionPrompter {
    fn prompt(
        &self,
        _request: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        panic!("session lifecycle construction and creation must not ask user questions")
    }
}

#[test]
fn reference_host_lifecycle_engine_and_public_store_share_one_exact_store() {
    let temporary = TemporaryDirectory::new();
    let workspace = temporary.path().join("workspace");
    let sessions = temporary.path().join("sessions");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&sessions).unwrap();
    let loaded = load_native_config(&NativeEnvironment::new(None, None, None)).unwrap();
    let transport: Arc<dyn AiGatewayTransport> = Arc::new(InertTransport);
    let prompter: Arc<dyn PermissionPrompter> = Arc::new(InertPrompter);
    let host = NativeReferenceHost::compose_with_ai_gateway_transport(
        loaded,
        transport,
        production_gateway_target(),
        &workspace,
        &sessions,
        prompter,
        Arc::new(InertQuestionPrompter),
        never_deadline(),
    )
    .unwrap();

    assert!(Arc::ptr_eq(
        host.session_store(),
        host.session_lifecycle().session_store()
    ));
    assert!(std::ptr::addr_eq(
        host.engine().session_store(),
        host.session_store().as_ref() as &dyn SessionStore,
    ));

    let session_id = SessionId::new("reference-host-lifecycle").unwrap();
    let created =
        futures_executor::block_on(host.session_lifecycle().create(session_id.clone())).unwrap();
    let loaded = futures_executor::block_on(host.engine().load_session(session_id))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.record(), created.record());
    assert!(!created.has_active_turn());
    assert_eq!(
        fs::read_dir(sessions).unwrap().count(),
        2,
        "durable create publishes one data record and one permanent lock"
    );
}
