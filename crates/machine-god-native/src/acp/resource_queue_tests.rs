//! Actual native FIFO/provider custody with deterministic stores and file authority.

use super::*;
use crate::acp::{prompt::decode_prompt_input, resources::NativeAcpResourceContextError};
use crate::conversation_resource_context::{
    NATIVE_RESOURCE_PROMPT_CONTEXT_KEY, NativeResourcePromptContext,
    NativeResourcePromptContextError,
};
use crate::{NativeOwnedWorkerScope, NativeWorkspaceAuthority, NativeWorkspaceContexts};
use futures_executor::block_on;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{Engine, Message, ModelEvent, Role, StopReason};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    base: PathBuf,
    root: PathBuf,
    authority: NativeWorkspaceAuthority,
    contexts: Arc<NativeWorkspaceContexts>,
    permission_contexts: Arc<crate::NativePermissionContexts>,
    workers: NativeOwnedWorkerScope,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-acp-fifo-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let root = base.join("workspace");
        let state = base.join("state");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&state).unwrap();
        let open = |path: &Path| {
            rustix::fs::open(
                path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .unwrap()
        };
        let authority = NativeWorkspaceAuthority::open_blocking(
            open(&root),
            root.clone(),
            Some(open(&state)),
            state,
            vec![],
            false,
        )
        .unwrap();
        Self {
            base,
            root,
            authority,
            contexts: Arc::new(NativeWorkspaceContexts::new()),
            permission_contexts: Arc::new(crate::NativePermissionContexts::new()),
            workers: NativeOwnedWorkerScope::new(),
        }
    }
    fn setup(
        &self,
        steps: impl IntoIterator<Item = ModelProviderStep>,
    ) -> (
        NativeConversationRuntime,
        Arc<InMemorySessionStore>,
        ScriptedModelProvider,
    ) {
        let provider = ScriptedModelProvider::new("acp-resources", steps);
        let store = Arc::new(InMemorySessionStore::new());
        let engine = Engine::builder()
            .provider(provider.clone())
            .shared_session_store(store.clone())
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("acp-fifo").unwrap(),
                SessionIncarnationId::new("life").unwrap(),
            )
            .unwrap();
        let conversation = NativeConversation::from_session(session)
            .unwrap()
            .with_permission_contexts(&self.permission_contexts)
            .unwrap()
            .with_workspace_contexts(self.authority.clone(), &self.contexts)
            .unwrap();
        let runtime = NativeConversationRuntime::new(
            conversation,
            NativeModelPreferences::new("model", crate::NativeReasoningEffort::default(), false)
                .unwrap(),
            None,
        )
        .unwrap();
        (runtime, store, provider)
    }
    fn enqueue(&self, runtime: &NativeConversationRuntime, text: &str) -> NativeQueuedJobId {
        let input = decode_prompt_input(&json!({"prompt":[{"type":"text","text":text}]})).unwrap();
        runtime.enqueue_acp(input, self.workers.clone()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.workers.close();
        self.workers.completion().wait_on_worker().unwrap();
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}
fn finished() -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: "answer".into(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}
fn complete(turn: NativeConversationRuntimeTurn) {
    for event in block_on(turn.collect::<Vec<_>>()) {
        event.unwrap();
    }
}

fn block_reader(
    runtime: &NativeConversationRuntime,
    id: NativeQueuedJobId,
) -> (mpsc::Receiver<()>, mpsc::SyncSender<()>) {
    let (entered, observe) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    let wait = Mutex::new(wait);
    runtime
        .state
        .lock()
        .unwrap()
        .queue
        .iter_mut()
        .find(|job| job.id == id)
        .unwrap()
        .resources
        .as_mut()
        .unwrap()
        .before_read = Some(Arc::new(move || {
        entered.send(()).unwrap();
        wait.lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    }));
    (observe, release)
}

#[cfg(feature = "ai-gateway-http")]
impl NativeConversationRuntime {
    pub(crate) fn block_next_acp_resource_read_for_test(
        &self,
    ) -> (mpsc::Receiver<()>, mpsc::SyncSender<()>) {
        let id = self.state.lock().unwrap().queue.front().unwrap().id;
        block_reader(self, id)
    }
}

#[test]
fn acp_fifo_reads_after_take_and_only_provider_gets_advisory_context() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("AGENTS.md"), "before admission").unwrap();
    std::fs::create_dir(fixture.root.join("src")).unwrap();
    std::fs::write(fixture.root.join("src/AGENTS.md"), "NESTED target marker").unwrap();
    std::fs::write(fixture.root.join("src/file.rs"), "not prompt content").unwrap();
    let (runtime, store, provider) = fixture.setup([finished(), finished()]);
    let prompt = decode_prompt_input(&json!({"prompt":[{"type":"text","text":"canonical first"},{"type":"resource","resource":{"uri":format!("file://{}", fixture.root.join("src/file.rs").display())}}]})).unwrap();
    let first = runtime
        .enqueue_acp(prompt, fixture.workers.clone())
        .unwrap();
    fixture.enqueue(&runtime, "canonical second");
    drop(runtime.start_next(10));
    assert_eq!(runtime.status().queued_jobs, 2);
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
    std::fs::write(fixture.root.join("AGENTS.md"), "ADMITTED advisory marker").unwrap();
    let turn = block_on(runtime.start_next(10)).unwrap().unwrap();
    assert_eq!(turn.queued_id(), first);
    assert_eq!(
        runtime.record().messages,
        [Message::text(Role::User, "canonical first")]
    );
    let mut provenance_record = runtime.record();
    let provenance = crate::permission_context::prepare_provenance(&mut provenance_record, None, 0)
        .unwrap()
        .unwrap();
    assert!(provenance.contains("canonical first"));
    assert!(!provenance.contains("ADMITTED advisory marker"));
    assert!(!provenance.contains("NESTED target marker"));
    complete(turn);
    assert!(
        format!("{:?}", provider.requests()[0].request.messages)
            .contains("ADMITTED advisory marker")
    );
    assert!(
        format!("{:?}", provider.requests()[0].request.messages).contains("NESTED target marker")
    );
    assert!(!format!("{:?}", runtime.record().messages).contains("ADMITTED advisory marker"));
    assert!(
        !runtime
            .record()
            .metadata
            .contains_key(NATIVE_RESOURCE_PROMPT_CONTEXT_KEY)
    );
    std::fs::write(fixture.root.join("AGENTS.md"), "SECOND snapshot marker").unwrap();
    complete(block_on(runtime.start_next(20)).unwrap().unwrap());
    let second = format!("{:?}", provider.requests()[1].request.messages);
    assert!(second.contains("SECOND snapshot marker"));
    assert!(!second.contains("ADMITTED advisory marker"));
    assert!(!second.contains("NESTED target marker"));
}

#[test]
fn saved_checkpoint_rejects_mismatched_and_overbudget_resource_context() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("AGENTS.md"), "saved instructions").unwrap();
    let (runtime, _, _) = fixture.setup([]);
    fixture.enqueue(&runtime, "canonical");
    drop(block_on(runtime.start_next(10)).unwrap().unwrap());
    let mut record = runtime.record();
    record
        .metadata
        .get_mut(NATIVE_RESOURCE_PROMPT_CONTEXT_KEY)
        .unwrap()["turn_sequence"] = json!(99);
    assert!(matches!(
        crate::conversation::validated_history(&record),
        Err(NativeConversationError::InvalidResourceContext(
            NativeResourcePromptContextError::CheckpointMismatch
        ))
    ));
    let mut record = runtime.record();
    record.metadata.insert(
        crate::NATIVE_SKILL_PROMPT_CONTEXT_KEY.to_owned(),
        crate::NativeSkillPromptContext::new(
            "s".repeat(machine_god_core::MAX_SESSION_USER_CONTEXT_BYTES),
        )
        .unwrap()
        .to_value(1, 0),
    );
    assert!(matches!(
        crate::conversation::validated_history(&record),
        Err(NativeConversationError::InvalidResourceContext(
            NativeResourcePromptContextError::ResourceLimit
        ))
    ));
}

#[test]
fn continuation_uses_exact_inert_snapshot_and_new_prompt_clears_it() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("AGENTS.md"), "saved resource bytes").unwrap();
    let (runtime, _, provider) = fixture.setup([finished(), finished()]);
    fixture.enqueue(&runtime, "canonical");
    drop(block_on(runtime.start_next(10)).unwrap().unwrap());
    let saved = runtime.record().metadata[NATIVE_RESOURCE_PROMPT_CONTEXT_KEY]["text"].clone();
    std::fs::remove_file(fixture.root.join("AGENTS.md")).unwrap();
    runtime
        .enqueue_continuation(InferenceOptions::default())
        .unwrap();
    let turn = block_on(runtime.start_next(20)).unwrap().unwrap();
    assert_eq!(
        runtime.record().metadata[NATIVE_RESOURCE_PROMPT_CONTEXT_KEY]["text"],
        saved
    );
    assert_eq!(
        runtime.record().messages,
        [Message::text(Role::User, "canonical")]
    );
    complete(turn);
    assert!(
        format!("{:?}", provider.requests()[0].request.messages).contains("saved resource bytes")
    );
    runtime.enqueue("ordinary new prompt".into()).unwrap();
    complete(block_on(runtime.start_next(30)).unwrap().unwrap());
    assert!(
        !runtime
            .record()
            .metadata
            .contains_key(NATIVE_RESOURCE_PROMPT_CONTEXT_KEY)
    );
    assert!(
        !format!("{:?}", provider.requests()[1].request.messages).contains("saved resource bytes")
    );
}

#[test]
fn queued_resource_bytes_and_acp_text_limit_are_charged_without_io() {
    let fixture = Fixture::new();
    let (runtime, store, _) = fixture.setup([]);
    let input = decode_prompt_input(&json!({"prompt":[{"type":"text","text":"x".repeat(crate::acp::prompt::MAX_ACP_PROMPT_BYTES)},{"type":"resource","resource":{"uri":"file:///outside/target"}},{"type":"resource","resource":{"uri":"https://example.test/omitted"}}]})).unwrap();
    let retained = input.retained_bytes();
    let id = runtime.enqueue_acp(input, fixture.workers.clone()).unwrap();
    assert!(runtime.status().queued_input_bytes >= retained);
    assert!(store.calls().is_empty());
    assert!(runtime.cancel_queued(id));
    assert_eq!(runtime.status().queued_input_bytes, 0);
    assert!(
        runtime
            .enqueue("x".repeat(MAX_NATIVE_QUEUED_PROMPT_BYTES + 1).into())
            .is_err()
    );
    let text = "x".repeat(crate::acp::prompt::MAX_ACP_PROMPT_BYTES);
    for _ in 0..3 {
        fixture.enqueue(&runtime, &text);
    }
    let input = decode_prompt_input(&json!({"prompt":[{"type":"text","text":text}]})).unwrap();
    assert!(matches!(
        runtime.enqueue_acp(input, fixture.workers.clone()),
        Err(NativeConversationRuntimeError::QueueLimit)
    ));
    assert_eq!(runtime.clear_queued(), 3);
}

#[test]
fn abandoned_admission_cancels_reader_and_retains_exact_runtime_lease() {
    let fixture = Fixture::new();
    let (runtime, store, provider) = fixture.setup([]);
    let id = fixture.enqueue(&runtime, "cancel me");
    let (entered, release) = block_reader(&runtime, id);
    let mut future = runtime.start_next(10);
    let waker = noop_waker();
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(future);
    assert!(runtime.status().active);
    assert!(matches!(
        block_on(runtime.start_next(11)),
        Err(NativeConversationRuntimeError::Busy)
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
    release.send(()).unwrap();
    fixture.workers.close();
    fixture.workers.completion().wait_on_worker().unwrap();
    assert!(!runtime.status().active);
    assert_eq!(runtime.status().queued_jobs, 0);
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
}

#[test]
fn cancel_active_preparation_rejects_before_provider_or_persistence() {
    let fixture = Fixture::new();
    let (runtime, store, provider) = fixture.setup([]);
    let id = fixture.enqueue(&runtime, "cancel me");
    let (entered, release) = block_reader(&runtime, id);
    let mut future = runtime.start_next(10);
    let waker = noop_waker();
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(runtime.request_active_cancel());
    release.send(()).unwrap();
    assert!(matches!(
        block_on(future),
        Err(NativeConversationRuntimeError::Resources(
            NativeAcpResourceContextError::Cancelled
        ))
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
    assert!(!runtime.status().active);
}

#[test]
fn combined_context_limit_fails_before_publication() {
    let fixture = Fixture::new();
    let (runtime, store, provider) = fixture.setup([]);
    let mut input = PendingInput::new(ConversationInput::Prompt("canonical".into()));
    input.skill_context = Some(
        crate::NativeSkillPromptContext::new(
            "s".repeat(machine_god_core::MAX_SESSION_USER_CONTEXT_BYTES - 1),
        )
        .unwrap(),
    );
    input.resource_context = Some(NativeResourcePromptContext::new("r".to_owned()).unwrap());
    let bytes = input_bytes(&input).unwrap();
    runtime.insert(input, bytes, None, None, None).unwrap();
    assert!(matches!(
        block_on(runtime.start_next(10)),
        Err(NativeConversationRuntimeError::Conversation(
            NativeConversationError::InvalidResourceContext(
                NativeResourcePromptContextError::ResourceLimit
            )
        ))
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
    assert!(runtime.record().messages.is_empty());
}

#[test]
fn workspace_changes_before_take_are_observed_without_restoring_old_root_authority() {
    let fixture = Fixture::new();
    let extra = fixture.base.join("extra");
    std::fs::create_dir(&extra).unwrap();
    std::fs::write(extra.join("AGENTS.md"), "removed root sentinel").unwrap();
    std::fs::write(extra.join("target"), "target").unwrap();
    let source = crate::NativeWorkspaceSource::new(extra.clone(), extra.clone(), true).unwrap();
    let spec = crate::NativeWorkspaceEntrySpec::new(source, false, true).unwrap();
    fixture
        .authority
        .install(
            fixture
                .authority
                .prepare_blocking(vec![spec], false)
                .unwrap(),
        )
        .unwrap();
    let (runtime, _, provider) = fixture.setup([finished()]);
    let input = decode_prompt_input(&json!({"prompt":[{"type":"text","text":"canonical"},{"type":"resource","resource":{"uri":format!("file://{}", extra.join("target").display())}}]})).unwrap();
    runtime.enqueue_acp(input, fixture.workers.clone()).unwrap();
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    complete(block_on(runtime.start_next(10)).unwrap().unwrap());
    let request = format!("{:?}", provider.requests()[0].request.messages);
    assert!(!request.contains("removed root sentinel"));
    assert!(request.contains("unsafe=1"));
}
