#![cfg(any(target_os = "linux", target_os = "macos"))]

#[path = "../src/permission_preparer/identity.rs"]
mod identities;
#[path = "permission_preparer/terminal_identity.rs"]
mod terminal_identity;

use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{
    BoxFuture, CancellationToken, Engine, EngineLimits, ModelEvent, PermissionRequest, SessionId,
    SessionIncarnationId, StopReason, Tool, ToolCall, ToolCallId, ToolName,
};
use machine_god_native::*;
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs::{self, File},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "machine-god-preparer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct Clock(Instant);
impl NativePermissionReviewClock for Clock {
    fn now(&self) -> Instant {
        self.0
    }
    fn wait_until(&self, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(std::future::pending())
    }
}
struct Transport {
    decision: Option<&'static str>,
    wire: Mutex<Vec<Value>>,
    pending_drops: AtomicU64,
}
struct PendingReview<'a>(&'a AtomicU64);
impl Drop for PendingReview<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl AiGatewayTransport for Transport {
    fn stream(
        &self,
        request: AiGatewayTransportRequest,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<AiGatewayByteStream, machine_god_core::ProviderError>> {
        Box::pin(async move {
            self.wire
                .lock()
                .unwrap()
                .push(serde_json::from_slice(request.body()).unwrap());
            let Some(decision) = self.decision else {
                return Err(machine_god_core::ProviderError::new(
                    machine_god_core::ProviderErrorKind::Transport,
                    "fixture",
                    "fixture",
                    false,
                ));
            };
            if decision == "pending" {
                let _guard = PendingReview(&self.pending_drops);
                return std::future::pending().await;
            }
            let answer = json!({"type":"tool-call","toolCallId":"assessment", "toolName":"permission_decision",
                "input":{"risk":"critical","authorization":"unknown","decision":decision,"rationale":"Requested development task."}});
            let finish = json!({"type":"finish","finishReason":{"unified":"tool-calls"}});
            let bytes = format!("data: {answer}\n\ndata: {finish}\n\n").into_bytes();
            Ok(Box::pin(futures_util::stream::iter([Ok(bytes)])) as AiGatewayByteStream)
        })
    }
}
struct Prompt {
    decisions: Mutex<VecDeque<PermissionPromptDecision>>,
    calls: Mutex<usize>,
}
impl PermissionPrompter for Prompt {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async move {
            *self.calls.lock().unwrap() += 1;
            Ok(self
                .decisions
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(PermissionPromptDecision::Deny))
        })
    }
}
fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        name: ToolName::new(name).unwrap(),
        id: ToolCallId::new("call").unwrap(),
        arguments,
    }
}
fn round(call: ToolCall) -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::ToolCall { call },
        ModelEvent::Stop {
            reason: StopReason::ToolCalls,
        },
    ])
}
fn done() -> ModelProviderStep {
    ModelProviderStep::events([ModelEvent::Stop {
        reason: StopReason::Completed,
    }])
}
struct Harness {
    runtime: NativeConversationRuntime,
    transport: Arc<Transport>,
    prompt: Arc<Prompt>,
    workers: NativeOwnedWorkerScope,
    _preparer: Arc<NativeToolPermissionPreparer>,
    _engine: Engine,
}
impl Harness {
    fn new(
        directory: &Directory,
        mode: PermissionMode,
        rules: NativeConfiguredPermissionRules,
        decision: Option<&'static str>,
        prompts: Vec<PermissionPromptDecision>,
        calls: Vec<ToolCall>,
    ) -> Self {
        let registry = Arc::new(NativeFileApprovalRegistry::new());
        let (targets, tools) = native_tools(directory, &registry);
        let contexts = Arc::new(NativePermissionContexts::new());
        let transport = Arc::new(Transport {
            decision,
            wire: Mutex::default(),
            pending_drops: AtomicU64::new(0),
        });
        let reviewer = Arc::new(AiGatewayPermissionReviewer::new(
            transport.clone(),
            Arc::new(Clock(Instant::now())),
        ));
        let workers = NativeOwnedWorkerScope::new();
        let preparer = Arc::new(NativeToolPermissionPreparer::new(
            targets,
            Arc::new(
                NativeFileApprovalAuthority::from_directory(File::open(&directory.0).unwrap())
                    .unwrap(),
            ),
            registry,
            contexts.clone(),
            reviewer,
            workers.clone(),
        ));
        let prompt = Arc::new(Prompt {
            decisions: Mutex::new(prompts.into()),
            calls: Mutex::new(0),
        });
        let controller = Arc::new(NativePermissionController::new(
            preparer.clone(),
            prompt.clone(),
        ));
        preparer.bind_controller(&controller).unwrap();
        let mut builder = Engine::builder()
            .session_store(InMemorySessionStore::default())
            .provider(ScriptedModelProvider::new(
                "fixture",
                calls.into_iter().flat_map(|call| [round(call), done()]),
            ))
            .shared_permission_handler(controller.clone());
        for tool in tools {
            builder = builder.tool(NativePermissionGovernedTool::new(
                tool,
                EngineLimits::default(),
            ));
        }
        let engine = builder.build().unwrap();
        let session = engine
            .create_session(
                SessionId::new("native").unwrap(),
                SessionIncarnationId::new("life").unwrap(),
            )
            .unwrap();
        let conversation = NativeConversation::from_session(session)
            .unwrap()
            .with_permission_controller(
                &controller,
                NativePermissionPolicySnapshot::new(mode, Arc::new(rules)),
            )
            .unwrap()
            .with_permission_contexts(&contexts)
            .unwrap();
        let runtime = NativeConversationRuntime::new(
            conversation,
            NativeModelPreferences::new("fixture/model", NativeReasoningEffort::default(), false)
                .unwrap(),
            None,
        )
        .unwrap();
        Self {
            runtime,
            transport,
            prompt,
            workers,
            _preparer: preparer,
            _engine: engine,
        }
    }
    fn run(&self) {
        self.runtime
            .enqueue("Implement this requested change".into())
            .unwrap();
        let turn = block_on(self.runtime.start_next(1)).unwrap().unwrap();
        let events = block_on(turn.collect::<Vec<_>>());
        assert!(events.iter().all(Result::is_ok), "{events:?}");
        assert!(
            events.iter().any(|event| matches!(
                event.as_ref().unwrap().payload,
                machine_god_core::TurnEvent::Completed { .. }
            )),
            "{events:?}"
        );
    }
}

fn native_tools(
    directory: &Directory,
    registry: &Arc<NativeFileApprovalRegistry>,
) -> (Arc<NativePermissionTargetAuthority>, Vec<Arc<dyn Tool>>) {
    let read = Arc::new(ReadFileTool::open(&directory.0).unwrap());
    let grep = Arc::new(GrepFilesTool::open(&directory.0).unwrap());
    let targets = Arc::new(
        NativePermissionTargetAuthority::new(
            File::open(&directory.0).unwrap(),
            directory.0.to_str().unwrap().into(),
            vec![
                NativePermissionTargetTool::Ordinary(read.clone()),
                NativePermissionTargetTool::Grep(grep.clone()),
            ],
        )
        .unwrap(),
    );
    let tools: Vec<Arc<dyn Tool>> = vec![
        read,
        grep,
        Arc::new(
            WriteFileTool::open(&directory.0)
                .unwrap()
                .with_file_approvals(registry.clone()),
        ),
        Arc::new(
            EditFileTool::open(&directory.0)
                .unwrap()
                .with_file_approvals(registry.clone()),
        ),
        Arc::new(
            DeleteFileTool::open(&directory.0)
                .unwrap()
                .with_file_approvals(registry.clone()),
        ),
        Arc::new(
            CopyFileTool::open(&directory.0)
                .unwrap()
                .with_file_approvals(registry.clone()),
        ),
        Arc::new(
            RenameFileTool::open(&directory.0)
                .unwrap()
                .with_file_approvals(registry.clone()),
        ),
    ];
    (targets, tools)
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.workers.close();
        self.workers.completion().wait_on_worker().unwrap();
    }
}

#[test]
fn canonical_grep_bypass_and_owned_file_write_execute_through_real_controller() {
    let directory = Directory::new();
    fs::write(directory.0.join("file"), "before").unwrap();
    let harness = Harness::new(
        &directory,
        PermissionMode::Auto,
        NativeConfiguredPermissionRules::default(),
        None,
        vec![],
        vec![
            call("grep_files", json!({"pattern":"before"})),
            call("write_file", json!({"path":"file","content":"after"})),
        ],
    );
    harness.run();
    harness.run();
    assert_eq!(
        fs::read_to_string(directory.0.join("file")).unwrap(),
        "after"
    );
    assert_eq!(*harness.prompt.calls.lock().unwrap(), 0);
    assert!(harness.transport.wire.lock().unwrap().is_empty());
}

#[test]
fn actual_reviewer_allow_ask_and_failure_have_no_human_fallback() {
    for decision in [Some("allow"), Some("ask"), None] {
        let directory = Directory::new();
        fs::write(directory.0.join(".profile"), "before").unwrap();
        let harness = Harness::new(
            &directory,
            PermissionMode::Auto,
            NativeConfiguredPermissionRules::default(),
            decision,
            vec![],
            vec![call(
                "write_file",
                json!({"path":".profile","content":"after"}),
            )],
        );
        harness.run();
        assert_eq!(
            fs::read_to_string(directory.0.join(".profile")).unwrap(),
            if decision == Some("allow") {
                "after"
            } else {
                "before"
            }
        );
        assert_eq!(*harness.prompt.calls.lock().unwrap(), 0);
        let wire = harness.transport.wire.lock().unwrap();
        assert_eq!(wire.len(), 1);
        let body = wire[0].to_string();
        assert!(body.contains("Implement this requested change"));
        assert!(body.contains("before"));
        assert!(body.contains("after"));
    }
}

#[test]
fn read_session_grant_satisfies_write_disclosure_but_reset_retires_it() {
    let directory = Directory::new();
    fs::write(directory.0.join("file"), "before").unwrap();
    let rules = NativeConfiguredPermissionRules::new(vec![
        NativeConfiguredPermissionRule::new(
            "read_file",
            "*",
            NativeConfiguredPermissionDecision::Ask,
        )
        .unwrap(),
    ])
    .unwrap();
    let harness = Harness::new(
        &directory,
        PermissionMode::Auto,
        rules,
        None,
        vec![
            PermissionPromptDecision::AllowSession,
            PermissionPromptDecision::Deny,
        ],
        vec![
            call("read_file", json!({"path":"file"})),
            call("write_file", json!({"path":"file","content":"after"})),
            call("write_file", json!({"path":"file","content":"blocked"})),
        ],
    );
    harness.run();
    harness.run();
    assert_eq!(*harness.prompt.calls.lock().unwrap(), 1);
    assert_eq!(
        fs::read_to_string(directory.0.join("file")).unwrap(),
        "after"
    );
    harness.runtime.permissions().unwrap().reset().unwrap();
    harness
        .runtime
        .permissions()
        .unwrap()
        .set_mode(PermissionMode::Auto);
    harness.run();
    assert_eq!(*harness.prompt.calls.lock().unwrap(), 2);
    assert_eq!(
        fs::read_to_string(directory.0.join("file")).unwrap(),
        "after"
    );
    assert!(harness.transport.wire.lock().unwrap().is_empty());
}

#[test]
fn missing_targets_replan_without_fatal_error_or_prompt() {
    let directory = Directory::new();
    let harness = Harness::new(
        &directory,
        PermissionMode::Ask,
        NativeConfiguredPermissionRules::default(),
        None,
        vec![],
        vec![call("read_file", json!({"path":"missing"}))],
    );
    harness.run();
    assert_eq!(*harness.prompt.calls.lock().unwrap(), 0);
    assert!(harness.transport.wire.lock().unwrap().is_empty());
}

#[test]
fn all_five_file_mutations_claim_exact_owned_approvals_and_release_capacity() {
    let directory = Directory::new();
    let calls = vec![
        call("write_file", json!({"path":"file","content":"before"})),
        call(
            "edit_file",
            json!({"path":"file","old_string":"before","new_string":"after"}),
        ),
        call("copy_file", json!({"source":"file","destination":"copied"})),
        call(
            "rename_file",
            json!({"old_path":"copied","new_path":"renamed"}),
        ),
        call("delete_file", json!({"path":"renamed"})),
    ];
    let harness = Harness::new(
        &directory,
        PermissionMode::Yolo,
        NativeConfiguredPermissionRules::default(),
        None,
        vec![],
        calls,
    );
    harness.run();
    assert_eq!(
        fs::read_to_string(directory.0.join("file")).unwrap(),
        "before"
    );
    harness.run();
    assert_eq!(
        fs::read_to_string(directory.0.join("file")).unwrap(),
        "after"
    );
    harness.run();
    assert_eq!(
        fs::read_to_string(directory.0.join("copied")).unwrap(),
        "after"
    );
    harness.run();
    assert!(!directory.0.join("copied").exists());
    assert_eq!(
        fs::read_to_string(directory.0.join("renamed")).unwrap(),
        "after"
    );
    harness.run();
    assert!(!directory.0.join("renamed").exists());
    assert_eq!(*harness.prompt.calls.lock().unwrap(), 0);
}

#[test]
fn dropped_unpolled_turn_starts_no_preparation_effects_or_review() {
    let directory = Directory::new();
    let harness = Harness::new(
        &directory,
        PermissionMode::Auto,
        NativeConfiguredPermissionRules::default(),
        Some("allow"),
        vec![],
        vec![call(
            "write_file",
            json!({"path":".profile","content":"after"}),
        )],
    );
    harness.runtime.enqueue("requested change".into()).unwrap();
    let turn = block_on(harness.runtime.start_next(1)).unwrap().unwrap();
    drop(turn);
    assert!(!directory.0.join(".profile").exists());
    assert_eq!(*harness.prompt.calls.lock().unwrap(), 0);
    assert!(harness.transport.wire.lock().unwrap().is_empty());
}

#[test]
fn cancellation_drops_actual_pending_review_and_retires_unclaimed_file_evidence() {
    use futures_core::Stream;
    use std::{pin::Pin, task::Poll};
    let directory = Directory::new();
    let harness = Harness::new(
        &directory,
        PermissionMode::Auto,
        NativeConfiguredPermissionRules::default(),
        Some("pending"),
        vec![],
        vec![call(
            "write_file",
            json!({"path":".profile","content":"blocked"}),
        )],
    );
    harness.runtime.enqueue("requested change".into()).unwrap();
    let mut turn = block_on(harness.runtime.start_next(1)).unwrap().unwrap();
    block_on(std::future::poll_fn(|cx| {
        loop {
            if !harness.transport.wire.lock().unwrap().is_empty() {
                return Poll::Ready(());
            }
            match Pin::new(&mut turn).poll_next(cx) {
                Poll::Ready(Some(event)) => {
                    event.unwrap();
                }
                Poll::Ready(None) => panic!("review was not reached"),
                Poll::Pending if harness.transport.wire.lock().unwrap().is_empty() => {
                    return Poll::Pending;
                }
                Poll::Pending => return Poll::Ready(()),
            }
        }
    }));
    assert!(turn.handle().unwrap().cancel());
    block_on(turn.collect::<Vec<_>>());
    assert_eq!(harness.transport.pending_drops.load(Ordering::SeqCst), 1);
    assert!(!directory.0.join(".profile").exists());
    assert_eq!(*harness.prompt.calls.lock().unwrap(), 0);
}

#[test]
fn execution_bounds_remain_distinct_from_the_smaller_reviewer_packet() {
    let directory = Directory::new();
    let content = "x".repeat(24 * 1024);
    let harness = Harness::new(
        &directory,
        PermissionMode::Auto,
        NativeConfiguredPermissionRules::default(),
        Some("allow"),
        vec![],
        vec![
            call("write_file", json!({"path":"file","content":content})),
            call("write_file", json!({"path":".profile","content":content})),
        ],
    );
    harness.run();
    assert_eq!(
        fs::read_to_string(directory.0.join("file")).unwrap(),
        content
    );
    harness.run();
    assert!(!directory.0.join(".profile").exists());
    assert!(harness.transport.wire.lock().unwrap().is_empty());
    assert_eq!(*harness.prompt.calls.lock().unwrap(), 0);
}
