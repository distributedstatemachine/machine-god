// Nested under terminal_host::tests to exercise the actual complete host.
use super::*;
use crate::terminal_permission_policy::tests::Fixture as PolicyFixture;
use crate::workspace_context::{WorkspaceContextRegistration, WorkspaceContextSession};
use crate::{
    NativeSandboxMode, NativeWorkspaceAuthority, NativeWorkspaceContexts, NativeWorkspaceEntrySpec,
    NativeWorkspaceSource, PermissionMode,
};
use machine_god_core::Turn;

struct Scoped {
    host: Fixture,
    policy: PolicyFixture,
    turn: Turn,
    _permission: crate::NativePermissionTurn,
    owner: Arc<WorkspaceContextSession>,
    registration: Option<WorkspaceContextRegistration>,
    authority: NativeWorkspaceAuthority,
    contexts: Arc<NativeWorkspaceContexts>,
}

impl Scoped {
    fn new(mode: Option<(PermissionMode, NativeSandboxMode)>, tmux: Option<PathBuf>) -> Self {
        let (permission_mode, sandbox_mode) =
            mode.unwrap_or((PermissionMode::Ask, NativeSandboxMode::None));
        let policy = PolicyFixture::new(permission_mode, sandbox_mode);
        let (turn, permission, context) = policy.turn();
        let contexts = Arc::new(NativeWorkspaceContexts::new());
        let owner = contexts.register(&policy.session).unwrap();
        let mut authority = None;
        let mut registration = None;
        let mut host = Fixture::with_workspace(Some(contexts.clone()), tmux, |workspace| {
            let base = workspace.parent().unwrap();
            let extra = base.join("extra");
            std::fs::create_dir(&extra).unwrap();
            std::fs::create_dir(extra.join("child")).unwrap();
            let scope = NativeWorkspaceAuthority::open_blocking(
                open(workspace),
                workspace.to_owned(),
                Some(open(&base.join("state"))),
                base.join("state"),
                vec![
                    NativeWorkspaceEntrySpec::new(
                        NativeWorkspaceSource::new(extra.clone(), extra, true).unwrap(),
                        false,
                        true,
                    )
                    .unwrap(),
                ],
                false,
            )
            .unwrap();
            registration = Some(owner.begin(&turn, scope.snapshot().unwrap()).unwrap());
            authority = Some(scope);
            mode.map(|_| {
                let executable =
                    if cfg!(target_os = "macos") && sandbox_mode == NativeSandboxMode::Os {
                        Some(std::fs::File::open(crate::NATIVE_SANDBOX_EXECUTABLE).unwrap())
                    } else {
                        None
                    };
                let selected = Arc::new(
                    NativeTerminalPermissionPolicy::new(vec![], executable)
                        .unwrap()
                        .with_workspace_contexts(contexts.clone()),
                );
                selected.bind_controller(&policy.controller).unwrap();
                selected
            })
        });
        host.context = context;
        Self {
            host,
            policy,
            turn,
            _permission: permission,
            owner,
            registration,
            authority: authority.unwrap(),
            contexts,
        }
    }

    fn extra(&self) -> PathBuf {
        self.host.root.join("extra")
    }

    fn reregister(&mut self) {
        self.registration.take();
        self.registration = Some(
            self.owner
                .begin(&self.turn, self.authority.snapshot().unwrap())
                .unwrap(),
        );
    }

    fn prepared(&self, arguments: Value) -> machine_god_core::PreparedToolCall {
        self.host
            .tool
            .prepare(ToolCall {
                id: self.host.context.call_id.clone(),
                name: ToolName::new("terminal").unwrap(),
                arguments,
            })
            .unwrap()
    }
}

fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "command marker missing: {path:?}");
}

#[test]
fn workspace_cwd_exec_preserves_native_symlink_parent_and_old_snapshot() {
    let fixture = Scoped::new(None, None);
    std::os::unix::fs::symlink(
        fixture.extra().join("child"),
        fixture.host.root.join("workspace/link"),
    )
    .unwrap();
    for cwd in [fixture.extra().to_str().unwrap(), "link/.."] {
        fixture.host.action(json!({"action":"exec","profile":"clean","cwd":cwd,"command":"printf retained > native-parent"}));
        assert_eq!(
            std::fs::read(fixture.extra().join("native-parent")).unwrap(),
            b"retained"
        );
    }
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    fixture.host.action(json!({"action":"exec","profile":"clean","cwd":fixture.extra(),"command":"printf old > old-snapshot"}));
    assert_eq!(
        std::fs::read(fixture.extra().join("old-snapshot")).unwrap(),
        b"old"
    );
    for cwd in [fixture.host.root.join("state"), fixture.host.root.clone()] {
        assert!(fixture.host.try_action(json!({"action":"exec","profile":"clean","cwd":cwd,"command":"printf denied > must-not-run"})).is_err());
        assert!(!cwd.join("must-not-run").exists());
    }
}

#[test]
fn workspace_cwd_rejects_retained_state_moved_into_an_allowed_root() {
    let fixture = Scoped::new(None, None);
    let state = fixture.host.root.join("state");
    std::fs::create_dir(state.join("nested")).unwrap();
    let moved = fixture.extra().join("moved-state");
    std::fs::rename(&state, &moved).unwrap();
    std::fs::create_dir(&state).unwrap();
    for cwd in [&moved, &moved.join("nested")] {
        for action in ["exec", "start"] {
            assert!(fixture.host.try_action(json!({"action":action,"profile":"clean","cwd":cwd,"command":"printf forbidden > forbidden"})).is_err());
            assert!(!cwd.join("forbidden").exists());
        }
    }
}

#[test]
fn workspace_cwd_unpolled_entrypoints_cannot_rebind_reused_registration() {
    for for_turn in [false, true] {
        let mut fixture = Scoped::new(None, None);
        let prepared = fixture.prepared(json!({"action":"exec","profile":"clean","cwd":fixture.extra(),"command":"printf forbidden > late"}));
        // Construct both actual Tool entrypoints before replacing the exact registration.
        if for_turn {
            let future = fixture.host.tool.execute_for_turn(
                fixture.host.context.clone(),
                prepared.arguments().clone(),
                CancellationToken::new(),
            );
            fixture.registration.take();
            fixture.registration = Some(
                fixture
                    .owner
                    .begin(&fixture.turn, fixture.authority.snapshot().unwrap())
                    .unwrap(),
            );
            assert!(futures_executor::block_on(future).is_err());
        } else {
            let future = fixture.host.tool.execute(
                fixture.host.context.clone(),
                prepared.arguments().clone(),
                CancellationToken::new(),
            );
            fixture.registration.take();
            fixture.registration = Some(
                fixture
                    .owner
                    .begin(&fixture.turn, fixture.authority.snapshot().unwrap())
                    .unwrap(),
            );
            assert!(futures_executor::block_on(future).is_err());
        }
        assert!(!fixture.extra().join("late").exists());
        assert!(!fixture.host.root.join("state/terminal-v1").exists());
        fixture.host.action(json!({"action":"exec","profile":"clean","cwd":fixture.extra(),"command":"printf current > current"}));
        assert!(fixture.extra().join("current").exists());
    }
}

#[test]
fn workspace_cwd_governed_wrapper_keeps_both_acceptance_stamps() {
    for for_turn in [false, true] {
        let mut fixture = Scoped::new(None, None);
        let tool = crate::NativePermissionGovernedTool::new(
            fixture.host.tool.clone(),
            machine_god_core::EngineLimits::default(),
        );
        let prepared = tool.prepare(ToolCall { id:fixture.host.context.call_id.clone(),name:ToolName::new("terminal").unwrap(),arguments:json!({"action":"exec","profile":"clean","cwd":fixture.extra(),"command":"printf no > wrapper"}) }).unwrap();
        if for_turn {
            let future = tool.execute_for_turn(
                fixture.host.context.clone(),
                prepared.arguments().clone(),
                CancellationToken::new(),
            );
            fixture.reregister();
            assert!(futures_executor::block_on(future).is_err());
        } else {
            let future = tool.execute(
                fixture.host.context.clone(),
                prepared.arguments().clone(),
                CancellationToken::new(),
            );
            fixture.reregister();
            assert!(futures_executor::block_on(future).is_err());
        }
        assert!(!fixture.extra().join("wrapper").exists());
    }
}

#[test]
fn workspace_cwd_permission_evidence_uses_same_scope_and_rejects_late_registration() {
    use machine_god_core::{
        PermissionInvocation, PermissionRequest, PermissionRequestId, PermissionRisk,
    };
    let mut fixture = Scoped::new(None, None);
    let primary = fixture.host.root.join("workspace");
    let authority = crate::NativePermissionTargetAuthority::new(
        std::fs::File::open(&primary).unwrap(),
        primary.to_str().unwrap().into(),
        vec![crate::NativePermissionTargetTool::Terminal(
            fixture.host.tool.clone(),
        )],
    )
    .unwrap()
    .with_workspace_contexts(fixture.contexts.clone());
    let prepared = fixture.prepared(json!({"action":"exec","profile":"clean","cwd":fixture.extra(),"command":"printf no > permission-must-not-run"}));
    let request = PermissionRequest {
        id: PermissionRequestId::new("cwd").unwrap(),
        session_id: fixture.host.context.session_id.clone(),
        session_incarnation_id: fixture.host.context.session_incarnation_id.clone(),
        turn_id: fixture.host.context.turn_id.clone(),
        capability: prepared.capability().unwrap().clone(),
        risk: PermissionRisk::Critical,
        reason: "test".into(),
    };
    let name = ToolName::new("terminal").unwrap();
    let call_id = fixture.host.context.call_id.clone();
    let invocation = || PermissionInvocation {
        tool_name: &name,
        call_id: &call_id,
        arguments: prepared.arguments(),
    };
    let old = authority.prepare(&request, invocation(), CancellationToken::new());
    fixture.reregister();
    assert!(futures_executor::block_on(old).is_err());
    let evidence = futures_executor::block_on(authority.prepare(
        &request,
        invocation(),
        CancellationToken::new(),
    ))
    .unwrap();
    let Some(TerminalActionRequest::Exec { request }) = evidence.terminal_action() else {
        panic!("exec evidence");
    };
    assert_eq!(request.cwd, fixture.extra().to_str().unwrap());
    evidence.revalidate().unwrap();
    assert!(!fixture.extra().join("permission-must-not-run").exists());
    let foreign = Scoped::new(None, None);
    let scope = Arc::new(
        foreign
            .contexts
            .snapshot_for_tool(&foreign.host.context)
            .unwrap(),
    );
    let invocation = serde_json::from_value(prepared.arguments()["invocation"].clone()).unwrap();
    let resolver = fixture.host.tool.permission_resolver().unwrap();
    assert!(
        futures_executor::block_on(resolver.resolve_with_workspace_scope(
            invocation,
            Some(scope),
            CancellationToken::new()
        ))
        .is_err()
    );
    fixture.reregister();
    assert!(evidence.revalidate().is_err());
}

#[test]
fn workspace_cwd_missing_foreign_removed_replaced_and_cancelled_scopes_fail_closed() {
    let mut fixture = Scoped::new(None, None);
    let prepared = fixture.prepared(json!({"action":"exec","profile":"clean","cwd":fixture.extra(),"command":"printf no > forbidden"}));
    let mut wrong = fixture.host.context.clone();
    wrong.session_incarnation_id = SessionIncarnationId::new("foreign").unwrap();
    assert!(
        futures_executor::block_on(fixture.host.tool.execute(
            wrong,
            prepared.arguments().clone(),
            CancellationToken::new()
        ))
        .is_err()
    );
    fixture.registration.take();
    let absent = fixture.host.tool.execute(
        fixture.host.context.clone(),
        prepared.arguments().clone(),
        CancellationToken::new(),
    );
    fixture.registration = Some(
        fixture
            .owner
            .begin(&fixture.turn, fixture.authority.snapshot().unwrap())
            .unwrap(),
    );
    assert!(futures_executor::block_on(absent).is_err());
    std::fs::rename(fixture.extra(), fixture.host.root.join("old-extra")).unwrap();
    std::fs::create_dir(fixture.extra()).unwrap();
    assert!(
        futures_executor::block_on(fixture.host.tool.execute(
            fixture.host.context.clone(),
            prepared.arguments().clone(),
            CancellationToken::new()
        ))
        .is_err()
    );
    fixture
        .authority
        .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
        .unwrap();
    fixture.reregister();
    assert!(
        futures_executor::block_on(fixture.host.tool.execute(
            fixture.host.context.clone(),
            prepared.arguments().clone(),
            CancellationToken::new()
        ))
        .is_err()
    );
    let _ = fixture.turn.handle().cancel();
    assert!(
        fixture
            .host
            .try_action(
                json!({"action":"exec","profile":"clean","command":"printf no > forbidden"})
            )
            .is_err()
    );
    assert!(!fixture.extra().join("forbidden").exists());
    assert!(!fixture.host.root.join("workspace/forbidden").exists());
}

#[test]
fn workspace_cwd_pty_and_monitors_use_additional_cwd_without_permission_options() {
    let mut fixture = Scoped::new(None, None);
    let probe = json!({"condition":{"kind":"custom_probe","command":"printf probe > custom-probe","cwd":"."},"check_interval_ms":10,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_session_end"}});
    let path = json!({"condition":{"kind":"path_exists","path":"pty"},"check_interval_ms":10,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_session_end"}});
    let TerminalActionResult::Start {session,..} = fixture.host.action(json!({"action":"start","profile":"clean","cwd":fixture.extra(),"command":"printf pty > pty; exec /bin/sleep 30","initial_monitors":[probe,path]})) else { panic!("start"); };
    wait_file(&fixture.extra().join("pty"));
    wait_file(&fixture.extra().join("custom-probe"));
    fixture.host.action(json!({"action":"monitor","session_id":session.session_id,"monitor":{"kind":"add","definition":{"condition":{"kind":"custom_probe","command":"printf added > added-probe","cwd":"."},"check_interval_ms":10,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_session_end"}}}}));
    fixture.registration.take();
    // Installed monitor authority and already committed PTY lifetime are independent.
    wait_file(&fixture.extra().join("added-probe"));
    fixture.reregister();
    fixture.host.close(&session.session_id);
}

#[test]
fn workspace_cwd_none_yolo_and_os_keep_distinct_launch_policy() {
    let mut modes = vec![
        (PermissionMode::Ask, NativeSandboxMode::None),
        (PermissionMode::Yolo, NativeSandboxMode::Os),
    ];
    if cfg!(target_os = "macos") {
        modes.push((PermissionMode::Auto, NativeSandboxMode::Os));
    }
    for mode in modes {
        let fixture = Scoped::new(Some(mode), None);
        fixture
            .authority
            .install(fixture.authority.prepare_blocking(vec![], false).unwrap())
            .unwrap();
        fixture.host.action(json!({"action":"exec","profile":"clean","cwd":fixture.extra(),"command":"printf allowed > mode"}));
        assert_eq!(
            std::fs::read(fixture.extra().join("mode")).unwrap(),
            b"allowed"
        );
    }
}

#[test]
fn workspace_cwd_launch_capture_cannot_replace_a_retired_scope() {
    let mut fixture = Scoped::new(None, None);
    let policy = NativeTerminalPermissionPolicy::new(vec![], None)
        .unwrap()
        .with_workspace_contexts(fixture.contexts.clone());
    policy.bind_controller(&fixture.policy.controller).unwrap();
    let scope = Arc::new(
        fixture
            .contexts
            .snapshot_for_tool(&fixture.host.context)
            .unwrap(),
    );
    fixture.reregister();
    assert!(
        policy
            .capture_on_worker(
                &fixture.host.context,
                Instant::now() + Duration::from_secs(2),
                &CancellationToken::new()
            )
            .is_ok(),
        "replacement registration is genuinely live"
    );
    assert!(
        capture_sandbox(
            Some(&policy),
            Some(&scope),
            &fixture.host.context,
            Instant::now() + Duration::from_secs(2),
            &CancellationToken::new()
        )
        .is_err()
    );
    assert!(
        policy
            .capture_with_shared_workspace_scope(
                &fixture.host.context,
                Some(scope),
                Instant::now() + Duration::from_secs(2),
                &CancellationToken::new()
            )
            .is_err()
    );
}

#[test]
fn workspace_cwd_tmux_launch_uses_additional_directory() {
    let explicit = std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY");
    let executable = explicit.clone().map(PathBuf::from).or_else(|| {
        [
            "/opt/homebrew/bin/tmux",
            "/usr/bin/tmux",
            "/usr/local/bin/tmux",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
    });
    let Some(executable) = executable else {
        assert!(explicit.is_none());
        return;
    };
    let fixture = Scoped::new(None, Some(executable));
    let TerminalActionResult::Start {session,..} = fixture.host.action(json!({"action":"start","backend":"tmux","profile":"clean","cwd":fixture.extra(),"command":"printf tmux > tmux; exec /bin/sleep 30"})) else { panic!("start"); };
    wait_file(&fixture.extra().join("tmux"));
    fixture.host.close(&session.session_id);
}
