#![cfg(any(target_os = "linux", target_os = "macos"))]

use super::*;
use std::fs::{self, File};

struct FilePreparer {
    registry: Arc<NativeFileApprovalRegistry>,
    authority: NativeFileApprovalAuthority,
}

struct FileAction {
    prepared: PreparedFileApproval,
    key: NativePermissionRuleKey,
}

impl NativePermissionActionPreparer for FilePreparer {
    fn prepare<'a>(
        &'a self,
        request: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        Box::pin(async move {
            let prepared = self
                .registry
                .prepare(&self.authority, request, invocation, cancellation)
                .await
                .map_err(|_| PermissionError::new("test_prepare", "test preparation failed"))?;
            let key = prepared.saved_rule_key().unwrap();
            Ok(Box::new(FileAction { prepared, key }) as Box<dyn NativePreparedPermissionAction>)
        })
    }
    fn close_turn(&self, session: &SessionId, incarnation: &SessionIncarnationId, turn: &TurnId) {
        self.registry.close_turn(session, incarnation, turn);
    }
}

impl NativePreparedPermissionAction for FileAction {
    fn saved_rule_key(&self) -> Option<&NativePermissionRuleKey> {
        Some(&self.key)
    }
    fn grant_key(&self) -> Option<&NativePermissionRuleKey> {
        Some(&self.key)
    }
    fn is_file_mutation(&self) -> bool {
        true
    }
    fn configured_outcome(
        &self,
        _: &NativeConfiguredPermissionRules,
    ) -> Result<NativePermissionConfiguredOutcome, PermissionError> {
        Ok(NativePermissionConfiguredOutcome::Unresolved)
    }
    fn allows_without_review(&self, _: PermissionMode) -> bool {
        false
    }
    fn automatic_review(
        &self,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionAutomaticOutcome, PermissionError>> {
        Box::pin(async { panic!("Ask-mode fixture must never request Auto review") })
    }
    fn bind_execution(
        self: Box<Self>,
        proof: NativePermissionExecutionProof,
    ) -> Result<Box<dyn PermissionExecutionAdmission>, PermissionError> {
        Ok(Box::new(self.prepared.admit(Arc::new(proof))))
    }
}

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        for _ in 0..1024 {
            let path = std::env::temp_dir().join(format!(
                "mg-controller-file-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("{error}"),
            }
        }
        panic!("test directory capacity exhausted")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        match fs::remove_file(self.0.join("output.txt")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("{error}"),
        }
        fs::remove_dir(&self.0).unwrap();
    }
}

#[test]
fn real_file_tool_composes_owned_preparation_native_policy_and_final_effect_check() {
    use futures_util::StreamExt;
    use machine_god_testkit::ModelProviderStep;
    for (decision, path, prompt_count, written) in [
        (PermissionPromptDecision::AllowOnce, "output.txt", 1, true),
        (PermissionPromptDecision::Deny, "output.txt", 1, false),
        (
            PermissionPromptDecision::AllowOnce,
            "missing/output.txt",
            0,
            false,
        ),
    ] {
        let directory = Directory::new();
        let registry = Arc::new(NativeFileApprovalRegistry::new());
        let preparer = Arc::new(FilePreparer {
            registry: registry.clone(),
            authority: NativeFileApprovalAuthority::from_directory(
                File::open(&directory.0).unwrap(),
            )
            .unwrap(),
        });
        let prompt = Arc::new(Prompter {
            calls: AtomicUsize::new(0),
            pending: AtomicBool::new(false),
            decision: Mutex::new(decision),
        });
        let controller = Arc::new(NativePermissionController::new(preparer, prompt.clone()));
        let provider = ScriptedModelProvider::new(
            "test",
            [
                ModelProviderStep::events([
                    ModelEvent::ToolCall {
                        call: ToolCall {
                            name: ToolName::new("write_file").unwrap(),
                            id: ToolCallId::new("write").unwrap(),
                            arguments: json!({"path":path,"content":"actual bytes"}),
                        },
                    },
                    ModelEvent::Stop {
                        reason: StopReason::ToolCalls,
                    },
                ]),
                ModelProviderStep::events([ModelEvent::Stop {
                    reason: StopReason::Completed,
                }]),
            ],
        );
        let engine = Engine::builder()
            .session_store(InMemorySessionStore::default())
            .provider(provider)
            .shared_permission_handler(controller.clone())
            .tool(
                WriteFileTool::open(&directory.0)
                    .unwrap()
                    .with_file_approvals(registry),
            )
            .build()
            .unwrap();
        let session = engine
            .create_session(
                SessionId::new("file-session").unwrap(),
                SessionIncarnationId::new("life").unwrap(),
            )
            .unwrap();
        let conversation = NativeConversation::from_session(session)
            .unwrap()
            .with_permission_controller(
                &controller,
                NativePermissionPolicySnapshot::new(PermissionMode::Ask, Arc::default()),
            )
            .unwrap();
        let mut turn = block_on(conversation.prompt("write the file".into(), 100)).unwrap();
        let mut completed = false;
        block_on(async {
            while let Some(event) = turn.next().await {
                let event = event.unwrap();
                assert!(
                    !matches!(event.payload, TurnEvent::Failed { .. }),
                    "{event:?}"
                );
                completed |= matches!(event.payload, TurnEvent::Completed { .. });
            }
        });
        assert!(completed);
        assert_eq!(prompt.calls.load(Ordering::SeqCst), prompt_count);
        if written {
            assert_eq!(
                fs::read(directory.0.join("output.txt")).unwrap(),
                b"actual bytes"
            );
        } else {
            assert!(!directory.0.join("output.txt").exists());
        }
    }
}
