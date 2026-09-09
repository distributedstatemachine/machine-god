#![cfg(any(target_os = "linux", target_os = "macos"))]

use futures_executor::block_on;
use machine_god_core::*;
use machine_god_native::*;
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mg-permission-targets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn authority(&self, tools: Vec<NativePermissionTargetTool>) -> NativePermissionTargetAuthority {
        NativePermissionTargetAuthority::new(
            File::open(&self.0).unwrap(),
            self.0.to_str().unwrap().into(),
            tools,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        name: ToolName::new(name).unwrap(),
        id: ToolCallId::new("call").unwrap(),
        arguments,
    }
}
fn request(capability: Capability) -> PermissionRequest {
    PermissionRequest {
        id: PermissionRequestId::new("permission").unwrap(),
        session_id: SessionId::new("session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
        turn_id: TurnId::new("turn").unwrap(),
        capability,
        risk: PermissionRisk::Critical,
        reason: "untrusted hint".into(),
    }
}
fn prepare(
    authority: &NativePermissionTargetAuthority,
    tool: &dyn Tool,
    raw: ToolCall,
) -> Result<NativePreparedPermissionTargets, PermissionError> {
    let ToolCall {
        name,
        id,
        arguments,
    } = raw;
    let prepared = tool
        .prepare(ToolCall {
            name: name.clone(),
            id: id.clone(),
            arguments,
        })
        .unwrap();
    let capability = prepared
        .capability()
        .cloned()
        .unwrap_or_else(|| Capability::Tool {
            name: name.clone(),
            call_id: id.clone(),
            arguments: prepared.arguments().clone(),
        });
    block_on(authority.prepare(
        &request(capability),
        PermissionInvocation {
            tool_name: &name,
            call_id: &id,
            arguments: prepared.arguments(),
        },
        CancellationToken::new(),
    ))
}
fn rule(
    permission: &str,
    pattern: &str,
    decision: NativeConfiguredPermissionDecision,
) -> NativeConfiguredPermissionRule {
    NativeConfiguredPermissionRule::new(permission, pattern, decision).unwrap()
}

#[test]
fn real_read_defaults_ignore_risk_and_still_evaluate_configured_policy() {
    let f = Fixture::new();
    fs::write(f.0.join("a"), "body").unwrap();
    let tool = Arc::new(ReadFileTool::open(&f.0).unwrap());
    let authority = f.authority(vec![NativePermissionTargetTool::Ordinary(tool.clone())]);
    let prepared = prepare(
        &authority,
        tool.as_ref(),
        call("read_file", json!({"path":"a"})),
    )
    .unwrap();
    assert!(prepared.allows_without_review(PermissionMode::Ask));
    assert!(prepared.allows_without_review(PermissionMode::Auto));
    assert_eq!(
        prepared.targets()[0].path(),
        f.0.join("a").to_str().unwrap()
    );
    let rules = NativeConfiguredPermissionRules::new(vec![rule(
        "read",
        "a",
        NativeConfiguredPermissionDecision::Deny,
    )])
    .unwrap();
    assert_eq!(
        prepared.configured_outcome(&rules).unwrap(),
        NativePermissionConfiguredOutcome::Deny
    );
}

#[test]
fn ordinary_validation_uses_the_exact_request_and_invocation_context() {
    struct ContextualRead {
        inner: ReadFileTool,
        seen: Arc<std::sync::Mutex<Vec<ToolContext>>>,
    }
    impl Tool for ContextualRead {
        fn spec(&self) -> ToolSpec {
            self.inner.spec()
        }
        fn prepare(&self, _: ToolCall) -> Result<PreparedToolCall, ToolError> {
            Err(ToolError::new(
                ToolErrorKind::InvalidInput,
                "needs_context",
                "needs context",
                false,
            ))
        }
        fn prepare_for_turn(
            &self,
            context: &ToolContext,
            call: ToolCall,
        ) -> Result<PreparedToolCall, ToolError> {
            self.seen.lock().unwrap().push(context.clone());
            self.inner.prepare(call)
        }
        fn execute(
            &self,
            context: ToolContext,
            arguments: Value,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
            self.inner.execute(context, arguments, cancellation)
        }
    }
    let fixture = Fixture::new();
    fs::write(fixture.0.join("file"), "data").unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tool = Arc::new(ContextualRead {
        inner: ReadFileTool::open(&fixture.0).unwrap(),
        seen: seen.clone(),
    });
    let authority = fixture.authority(vec![NativePermissionTargetTool::Ordinary(tool)]);
    let raw = call("read_file", json!({"path":"file"}));
    let request = request(Capability::Filesystem {
        access: FilesystemAccess::Read,
        path: "file".into(),
    });
    let targets = block_on(authority.prepare(
        &request,
        PermissionInvocation {
            tool_name: &raw.name,
            call_id: &raw.id,
            arguments: &raw.arguments,
        },
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(
        targets.targets()[0].path(),
        fixture.0.join("file").to_str().unwrap()
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].session_id, request.session_id);
    assert_eq!(
        seen[0].session_incarnation_id,
        request.session_incarnation_id
    );
    assert_eq!(seen[0].turn_id, request.turn_id);
    assert_eq!(seen[0].call_id, raw.id);
}

#[test]
fn create_folder_observes_missing_target_and_parent_without_creating_them() {
    let f = Fixture::new();
    let tool = Arc::new(CreateFolderTool::open(&f.0).unwrap());
    let authority = f.authority(vec![NativePermissionTargetTool::Ordinary(tool.clone())]);
    let prepared = prepare(
        &authority,
        tool.as_ref(),
        call("create_folder", json!({"path":"new/child"})),
    )
    .unwrap();
    assert!(!f.0.join("new").exists());
    assert_eq!(prepared.targets().len(), 2);
    assert!(!prepared.allows_without_review(PermissionMode::Ask));
    assert!(prepared.allows_without_review(PermissionMode::Auto));
    let rules = NativeConfiguredPermissionRules::new(vec![
        rule(
            "create_folder",
            "*",
            NativeConfiguredPermissionDecision::Allow,
        ),
        rule(
            "create_folder",
            "new",
            NativeConfiguredPermissionDecision::Deny,
        ),
    ])
    .unwrap();
    assert_eq!(
        prepared.configured_outcome(&rules).unwrap(),
        NativePermissionConfiguredOutcome::Deny
    );
    fs::create_dir(f.0.join("new")).unwrap();
    assert!(prepared.revalidate().is_err());
}

#[test]
fn sensitive_folder_does_not_gain_auto_bypass() {
    let f = Fixture::new();
    let tool = Arc::new(CreateFolderTool::open(&f.0).unwrap());
    let authority = f.authority(vec![NativePermissionTargetTool::Ordinary(tool.clone())]);
    for path in [".git/hooks/x", ".config/fish/config.fish", "x/.ssh/config"] {
        let prepared = prepare(
            &authority,
            tool.as_ref(),
            call("create_folder", json!({"path":path})),
        )
        .unwrap();
        assert!(!prepared.allows_without_review(PermissionMode::Auto));
    }
    assert!(
        prepare(
            &authority,
            tool.as_ref(),
            call("create_folder", json!({"path":".git/hooks-safe"}))
        )
        .unwrap()
        .allows_without_review(PermissionMode::Auto)
    );
}

#[test]
fn capability_mismatch_and_unregistered_builtin_are_rejected() {
    let f = Fixture::new();
    fs::write(f.0.join("a"), "body").unwrap();
    let tool = Arc::new(ReadFileTool::open(&f.0).unwrap());
    let authority = f.authority(vec![NativePermissionTargetTool::Ordinary(tool.clone())]);
    let raw = call("read_file", json!({"path":"a"}));
    let args = tool.prepare(raw.clone()).unwrap();
    let wrong = request(Capability::Filesystem {
        access: FilesystemAccess::Read,
        path: "b".into(),
    });
    assert!(
        block_on(authority.prepare(
            &wrong,
            PermissionInvocation {
                tool_name: &raw.name,
                call_id: &raw.id,
                arguments: args.arguments()
            },
            CancellationToken::new()
        ))
        .is_err()
    );
    let empty = f.authority(vec![]);
    assert!(prepare(&empty, tool.as_ref(), raw).is_err());
}

#[test]
fn replaced_parent_or_root_cannot_reuse_observations() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("dir")).unwrap();
    fs::write(f.0.join("dir/a"), "body").unwrap();
    let tool = Arc::new(ReadFileTool::open(&f.0).unwrap());
    let authority = f.authority(vec![NativePermissionTargetTool::Ordinary(tool.clone())]);
    let prepared = prepare(
        &authority,
        tool.as_ref(),
        call("read_file", json!({"path":"dir/a"})),
    )
    .unwrap();
    fs::rename(f.0.join("dir"), f.0.join("old")).unwrap();
    fs::create_dir(f.0.join("dir")).unwrap();
    fs::hard_link(f.0.join("old/a"), f.0.join("dir/a")).unwrap();
    assert!(prepared.revalidate().is_err());
    let other = Fixture::new();
    let mismatched = NativePermissionTargetAuthority::new(
        File::open(&other.0).unwrap(),
        f.0.to_str().unwrap().into(),
        vec![NativePermissionTargetTool::Ordinary(tool.clone())],
    )
    .unwrap();
    assert!(
        prepare(
            &mismatched,
            tool.as_ref(),
            call("read_file", json!({"path":"dir/a"}))
        )
        .is_err()
    );
}

#[test]
fn parent_symlink_never_expands_workspace_authority() {
    let f = Fixture::new();
    let other = Fixture::new();
    fs::write(other.0.join("a"), "body").unwrap();
    std::os::unix::fs::symlink(&other.0, f.0.join("link")).unwrap();
    let tool = Arc::new(ReadFileTool::open(&f.0).unwrap());
    let authority = f.authority(vec![NativePermissionTargetTool::Ordinary(tool.clone())]);
    assert!(
        prepare(
            &authority,
            tool.as_ref(),
            call("read_file", json!({"path":"link/a"}))
        )
        .is_err()
    );
}

struct Executor;
impl TerminalActionExecutor for Executor {
    fn execute(
        &self,
        _: ToolContext,
        _: TerminalActionInvocation,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
        panic!("permission preparation must not execute")
    }
}
struct Resolver(PathBuf);
impl NativePermissionTerminalResolver for Resolver {
    fn resolve(
        &self,
        invocation: TerminalActionInvocation,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionTerminalResolution, PermissionError>> {
        Box::pin(async move {
            let mut descriptor = None;
            let action = invocation
                .resolve_cwd(|raw| {
                    let path = self.0.join(raw).canonicalize().unwrap();
                    assert!(path.starts_with(&self.0));
                    descriptor = Some(File::open(&path).unwrap());
                    Ok(path.to_str().unwrap().into())
                })
                .unwrap();
            let shell = match &action {
                TerminalActionRequest::Exec { request } => Some(
                    TerminalShell::from_account_shell(
                        Some(std::path::Path::new("/bin/bash")),
                        request.profile,
                        None,
                    )
                    .unwrap(),
                ),
                _ => None,
            };
            NativePermissionTerminalResolution::new(
                action,
                descriptor,
                shell,
                "a".repeat(64),
                "b".repeat(64),
            )
        })
    }
}
fn terminal(f: &Fixture) -> Arc<TerminalActionTool> {
    Arc::new(
        TerminalActionTool::new(
            Arc::new(Executor),
            TerminalActionHostIdentity {
                workspace: f.0.to_str().unwrap().into(),
                default_cwd: f.0.to_str().unwrap().into(),
                environment_sha256: "a".repeat(64),
                shell_selection_sha256: "b".repeat(64),
            },
        )
        .unwrap()
        .with_permission_resolver(Arc::new(Resolver(f.0.clone()))),
    )
}

#[test]
fn terminal_canonical_envelope_validates_without_raw_repreparation() {
    let f = Fixture::new();
    let tool = terminal(&f);
    let authority = f.authority(vec![NativePermissionTargetTool::Terminal(tool.clone())]);
    let prepared = prepare(
        &authority,
        tool.as_ref(),
        call(
            "terminal",
            json!({"action":"exec","command":"printf hello","profile":"clean"}),
        ),
    )
    .unwrap();
    assert!(!prepared.allows_without_review(PermissionMode::Ask));
    assert!(!prepared.allows_without_review(PermissionMode::Auto));
    assert!(prepared.arguments_json().contains("shell_selection_sha256"));
    assert!(
        !prepared
            .identity_arguments_json()
            .unwrap()
            .contains("shell_selection_sha256")
    );
    let rules = NativeConfiguredPermissionRules::new(vec![rule(
        "bash",
        "printf *",
        NativeConfiguredPermissionDecision::Allow,
    )])
    .unwrap();
    assert_eq!(
        prepared.configured_outcome(&rules).unwrap(),
        NativePermissionConfiguredOutcome::Allow
    );
    assert!(
        matches!(prepared.terminal_action(),Some(TerminalActionRequest::Exec {request}) if request.cwd == f.0.to_str().unwrap())
    );
}

#[test]
fn terminal_read_bypass_is_action_specific_not_generic_low_risk() {
    let f = Fixture::new();
    let tool = terminal(&f);
    let authority = f.authority(vec![NativePermissionTargetTool::Terminal(tool.clone())]);
    for (raw, bypass) in [
        (json!({"action":"list"}), true),
        (json!({"action":"inspect","session_id":"terminal-a"}), true),
        (
            json!({"action":"inspect","session_id":"terminal-a","acknowledge_event_id":1}),
            false,
        ),
        (
            json!({"action":"wait","session_id":"terminal-a","return_when":{"kind":"exit"},"wait_ceiling_ms":1000}),
            false,
        ),
    ] {
        let prepared = prepare(&authority, tool.as_ref(), call("terminal", raw)).unwrap();
        assert!(!prepared.allows_without_review(PermissionMode::Ask));
        assert_eq!(prepared.allows_without_review(PermissionMode::Auto), bypass);
    }
}

#[test]
fn terminal_foreign_call_and_tampered_capability_reject() {
    let f = Fixture::new();
    let tool = terminal(&f);
    let authority = f.authority(vec![NativePermissionTargetTool::Terminal(tool.clone())]);
    let raw = call("terminal", json!({"action":"list"}));
    let prepared = tool.prepare(raw.clone()).unwrap();
    let mut req = request(prepared.capability().unwrap().clone());
    if let Capability::Custom { details, .. } = &mut req.capability {
        details["request_sha256"] = json!("0".repeat(64));
    }
    assert!(
        block_on(authority.prepare(
            &req,
            PermissionInvocation {
                tool_name: &raw.name,
                call_id: &raw.id,
                arguments: prepared.arguments()
            },
            CancellationToken::new()
        ))
        .is_err()
    );
    let other = ToolCallId::new("other").unwrap();
    let req = request(prepared.capability().unwrap().clone());
    assert!(
        block_on(authority.prepare(
            &req,
            PermissionInvocation {
                tool_name: &raw.name,
                call_id: &other,
                arguments: prepared.arguments()
            },
            CancellationToken::new()
        ))
        .is_err()
    );
}

#[test]
fn five_file_projections_reuse_owned_evidence_without_mutating() {
    let f = Fixture::new();
    fs::write(f.0.join("source"), "before").unwrap();
    let registry = Arc::new(NativeFileApprovalRegistry::new());
    let files = NativeFileApprovalAuthority::from_directory(File::open(&f.0).unwrap()).unwrap();
    let targets = f.authority(vec![]);
    let cases: Vec<(Box<dyn Tool>, Value, usize)> = vec![
        (
            Box::new(WriteFileTool::open(&f.0).unwrap()),
            json!({"path":"new","content":"after"}),
            2,
        ),
        (
            Box::new(EditFileTool::open(&f.0).unwrap()),
            json!({"path":"source","old_string":"before","new_string":"after"}),
            2,
        ),
        (
            Box::new(DeleteFileTool::open(&f.0).unwrap()),
            json!({"path":"source"}),
            1,
        ),
        (
            Box::new(CopyFileTool::open(&f.0).unwrap()),
            json!({"source":"source","destination":"new"}),
            2,
        ),
        (
            Box::new(RenameFileTool::open(&f.0).unwrap()),
            json!({"old_path":"source","new_path":"new"}),
            2,
        ),
    ];
    for (tool, args, count) in cases {
        let raw = call(tool.spec().name.as_str(), args);
        let prepared = tool.prepare(raw.clone()).unwrap();
        let req = request(prepared.capability().unwrap().clone());
        let invocation = PermissionInvocation {
            tool_name: &raw.name,
            call_id: &raw.id,
            arguments: prepared.arguments(),
        };
        let file =
            block_on(registry.prepare(&files, &req, invocation, CancellationToken::new())).unwrap();
        let projection = targets
            .from_file(&file, invocation, &CancellationToken::new())
            .unwrap();
        assert_eq!(projection.targets().len(), count);
        assert!(!projection.allows_without_review(PermissionMode::Auto));
        if matches!(raw.name.as_str(), "copy_file" | "rename_file") {
            assert_eq!(projection.targets()[0].role(), "source");
            assert_eq!(projection.targets()[1].role(), "destination");
        }
        assert_eq!(fs::read(f.0.join("source")).unwrap(), b"before");
        assert!(!f.0.join("new").exists());
    }
}

#[test]
fn construction_and_unpolled_preparation_do_not_observe_root_or_prompt() {
    let f = Fixture::new();
    fs::write(f.0.join("not-directory"), "body").unwrap();
    let tool = Arc::new(ReadFileTool::open(&f.0).unwrap());
    let authority = NativePermissionTargetAuthority::new(
        File::open(f.0.join("not-directory")).unwrap(),
        f.0.to_str().unwrap().into(),
        vec![NativePermissionTargetTool::Ordinary(tool.clone())],
    )
    .unwrap();
    let raw = call("read_file", json!({"path":"not-directory"}));
    let prepared = tool.prepare(raw.clone()).unwrap();
    let req = request(prepared.capability().unwrap().clone());
    let invocation = PermissionInvocation {
        tool_name: &raw.name,
        call_id: &raw.id,
        arguments: prepared.arguments(),
    };
    drop(authority.prepare(&req, invocation, CancellationToken::new()));
    assert!(block_on(authority.prepare(&req, invocation, CancellationToken::new())).is_err());
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(block_on(authority.prepare(&req, invocation, cancel)).is_err());
}

struct Questions;
impl QuestionPrompter for Questions {
    fn prompt(
        &self,
        _: QuestionPromptRequest,
    ) -> BoxFuture<'_, Result<QuestionPromptOutcome, QuestionPromptError>> {
        panic!("permission preparation must not prompt")
    }
}
#[test]
fn maximum_escaped_question_uses_prepared_not_incoming_bounds() {
    let f = Fixture::new();
    let tool = Arc::new(AskUserQuestionTool::new(Questions));
    let raw = call(
        "ask_user_question",
        json!({"questions":[{"question":"\u{1b}".repeat(MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES),"options":[{"label":"first"},{"label":"second"}]}]}),
    );
    let prepared = tool.prepare(raw.clone()).unwrap();
    assert_eq!(
        prepared.arguments()["questions"][0]["question"]
            .as_str()
            .unwrap()
            .len(),
        MAX_ASK_USER_QUESTION_RENDERED_QUESTION_BYTES
    );
    assert!(
        tool.prepare(ToolCall {
            name: raw.name.clone(),
            id: raw.id.clone(),
            arguments: prepared.arguments().clone()
        })
        .is_err()
    );
    let authority = f.authority(vec![NativePermissionTargetTool::Question(tool.clone())]);
    let result = prepare(&authority, tool.as_ref(), raw).unwrap();
    assert!(result.allows_without_review(PermissionMode::Ask));
    assert!(result.allows_without_review(PermissionMode::Auto));
    assert!(result.arguments_json().len() > MAX_ASK_USER_QUESTION_RAW_QUESTION_BYTES);
}

#[test]
fn grep_explicit_null_include_uses_actual_execution_decoder() {
    let f = Fixture::new();
    let tool = Arc::new(GrepFilesTool::open(&f.0).unwrap());
    let raw = call("grep_files", json!({"pattern":"needle"}));
    let prepared = tool.prepare(raw.clone()).unwrap();
    assert!(prepared.arguments()["include"].is_null());
    assert!(
        tool.prepare(ToolCall {
            name: raw.name.clone(),
            id: raw.id.clone(),
            arguments: prepared.arguments().clone()
        })
        .is_err()
    );
    let authority = f.authority(vec![NativePermissionTargetTool::Grep(tool.clone())]);
    let result = prepare(&authority, tool.as_ref(), raw).unwrap();
    assert!(result.allows_without_review(PermissionMode::Ask));
    assert!(result.allows_without_review(PermissionMode::Auto));
    assert_eq!(result.targets()[0].path(), f.0.to_str().unwrap());
    for include in ["*.rs", "src/**/*.rs"] {
        assert!(
            prepare(
                &authority,
                tool.as_ref(),
                call("grep_files", json!({"pattern":"needle","include":include}))
            )
            .is_ok()
        );
    }
}
