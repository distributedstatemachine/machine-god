use super::*;
use machine_god_core::{
    PermissionInvocation, PermissionRequestId, PermissionRisk, TerminalActionRequest,
    TerminalActionResult, ToolContext, ToolError, TurnId,
};
use std::path::Path;

struct Executor;
impl TerminalActionExecutor for Executor {
    fn execute(
        &self,
        _: ToolContext,
        _: TerminalActionInvocation,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
        panic!("identity preparation never executes a command")
    }
}
struct Resolver {
    directory: PathBuf,
    environment: char,
}
impl NativePermissionTerminalResolver for Resolver {
    fn resolve(
        &self,
        invocation: TerminalActionInvocation,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionTerminalResolution, machine_god_core::PermissionError>>
    {
        Box::pin(async move {
            let mut cwd = None;
            let action = invocation
                .resolve_cwd(|raw| {
                    let path = self.directory.join(raw).canonicalize().unwrap();
                    assert!(path.starts_with(&self.directory));
                    cwd = Some(File::open(&path).unwrap());
                    Ok(path.to_str().unwrap().to_owned())
                })
                .unwrap();
            let TerminalActionRequest::Exec { request } = &action else {
                panic!("exec fixture")
            };
            let shell = TerminalShell::from_account_shell(
                Some(Path::new("/bin/bash")),
                request.profile,
                None,
            )
            .unwrap();
            NativePermissionTerminalResolution::new(
                action,
                cwd,
                Some(shell),
                self.environment.to_string().repeat(64),
                "b".repeat(64),
            )
        })
    }
}
fn prepare(
    directory: &Directory,
    call_id: &str,
    command: &str,
    profile: &str,
    environment: char,
) -> NativePreparedPermissionTargets {
    let tool = Arc::new(
        TerminalActionTool::new(
            Arc::new(Executor),
            TerminalActionHostIdentity {
                workspace: directory.0.to_str().unwrap().into(),
                default_cwd: directory.0.to_str().unwrap().into(),
                environment_sha256: environment.to_string().repeat(64),
                shell_selection_sha256: "b".repeat(64),
            },
        )
        .unwrap()
        .with_permission_resolver(Arc::new(Resolver {
            directory: directory.0.clone(),
            environment,
        })),
    );
    let authority = NativePermissionTargetAuthority::new(
        File::open(&directory.0).unwrap(),
        directory.0.to_str().unwrap().into(),
        vec![NativePermissionTargetTool::Terminal(tool.clone())],
    )
    .unwrap();
    let mut raw = call(
        "terminal",
        json!({"action":"exec", "command":command, "profile":profile}),
    );
    raw.id = ToolCallId::new(call_id).unwrap();
    let prepared = tool.prepare(raw.clone()).unwrap();
    let request = PermissionRequest {
        id: PermissionRequestId::new("request").unwrap(),
        session_id: SessionId::new("session").unwrap(),
        session_incarnation_id: SessionIncarnationId::new("life").unwrap(),
        turn_id: TurnId::new("turn").unwrap(),
        capability: prepared.capability().unwrap().clone(),
        risk: PermissionRisk::High,
        reason: "fixture".into(),
    };
    block_on(authority.prepare(
        &request,
        PermissionInvocation {
            tool_name: &raw.name,
            call_id: &raw.id,
            arguments: prepared.arguments(),
        },
        CancellationToken::new(),
    ))
    .unwrap()
}
fn key(
    targets: &NativePreparedPermissionTargets,
    sandbox: NativeSandboxMode,
) -> Option<NativePermissionRuleKey> {
    identities::saved(
        targets,
        None,
        &NativePermissionPolicySnapshot::new(PermissionMode::Ask, Arc::default())
            .with_sandbox_mode(sandbox),
        &targets.identity_arguments_json().unwrap(),
    )
}

#[test]
fn exact_command_keys_exclude_call_stamps_but_bind_actual_shell_environment_and_sandbox() {
    let directory = Directory::new();
    let first = prepare(&directory, "one", "printf example", "user", 'a');
    let repeated = prepare(&directory, "two", "printf example", "user", 'a');
    assert_ne!(first.arguments_json(), repeated.arguments_json());
    assert_eq!(
        key(&first, NativeSandboxMode::None),
        key(&repeated, NativeSandboxMode::None)
    );
    assert_ne!(
        key(&first, NativeSandboxMode::None),
        key(&first, NativeSandboxMode::Os)
    );
    let clean = prepare(&directory, "one", "printf example", "clean", 'a');
    assert_ne!(
        key(&first, NativeSandboxMode::None),
        key(&clean, NativeSandboxMode::None)
    );
    let changed = prepare(&directory, "one", "printf example", "user", 'c');
    assert_ne!(
        key(&first, NativeSandboxMode::None),
        key(&changed, NativeSandboxMode::None)
    );
    let other = Directory::new();
    let changed = prepare(&other, "one", "printf example", "user", 'a');
    assert_ne!(
        key(&first, NativeSandboxMode::None),
        key(&changed, NativeSandboxMode::None)
    );
}

#[test]
fn oversized_saved_key_does_not_reject_or_truncate_complete_command_evidence() {
    let directory = Directory::new();
    let command = "x".repeat(8 * 1024);
    let prepared = prepare(&directory, "one", &command, "user", 'a');
    assert!(key(&prepared, NativeSandboxMode::None).is_none());
    assert!(
        prepared
            .identity_arguments_json()
            .unwrap()
            .contains(&command)
    );
    assert!(!prepared.allows_without_review(PermissionMode::Ask));
    assert!(!prepared.allows_without_review(PermissionMode::Auto));
}
