use super::*;
use crate::terminal_host_authority::TerminalHostAccountShell;
use machine_god_core::{SessionId, Tool, ToolCall, ToolCallId, ToolName, TurnId};
use rustix::fs::{Mode, OFlags};
use serde_json::{Value, json};
use std::num::NonZeroU32;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[path = "../terminal_host_lifecycle/host_tests.rs"]
mod lifecycle;
#[path = "../terminal_permission_policy/host_tests.rs"]
mod permission_policy;
#[path = "../terminal_host_authority/workspace_tests.rs"]
mod workspace;

struct Fixture {
    context: ToolContext,
    root: PathBuf,
    tool: Arc<TerminalActionTool>,
    resource: Option<NativeTerminalHostResource>,
    completion: NativeOwnedWorkerCompletion,
}
fn open(path: &Path) -> OwnedFd {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap()
}

fn write_cli_fixture_helper(root: &Path) -> PathBuf {
    let script = root.join("helper");
    let executable = std::env::current_exe().unwrap();
    let quoted = executable.to_str().unwrap().replace('\'', "'\\''");
    #[cfg(target_os = "macos")]
    let inventory = format!(
        "if [ \"$1\" = '{}' ]; then\nexec '{quoted}' --exact process_inventory_helper::tests::helper_entry --nocapture 2>&1 1>/dev/null\nfi\nif [ \"$1\" = '{}' ]; then\nexec '{quoted}' --exact process_inventory_protocol::tests::service_entry --nocapture 2>&1 1>/dev/null\nfi\n",
        crate::PROCESS_INVENTORY_HELPER_ARGUMENT,
        crate::PROCESS_INVENTORY_SERVICE_ARGUMENT,
    );
    #[cfg(not(target_os = "macos"))]
    let inventory = "";
    // Inventory is a raw pipe protocol: only the registered entrypoint's
    // stderr reaches the collector, never libtest's stdout framing. Exec
    // preserves direct-child ownership and the inherited original deadline.
    std::fs::write(&script, format!("#!/bin/sh\n[ \"$#\" -eq 1 ] || exit 125\n{inventory}export MACHINE_GOD_TEST_HOST_HELPER=\"$1\"\nexec '{quoted}' --exact terminal_host::tests::helper_child --ignored --nocapture --quiet\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    script
}

impl Fixture {
    fn new() -> Self {
        Self::with_permission(|_| None)
    }
    fn with_permission(
        permission: impl FnOnce(&Path) -> Option<Arc<NativeTerminalPermissionPolicy>>,
    ) -> Self {
        Self::with_workspace(None, None, permission)
    }
    fn with_workspace(
        contexts: Option<Arc<crate::NativeWorkspaceContexts>>,
        tmux: Option<PathBuf>,
        permission: impl FnOnce(&Path) -> Option<Arc<NativeTerminalPermissionPolicy>>,
    ) -> Self {
        let mut nonce = [0_u8; 8];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("mg-h-{:016x}", u64::from_le_bytes(nonce)));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        for child in ["workspace", "state", "artifacts"] {
            let path = root.join(child);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cli = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY")
            .map_or_else(|| write_cli_fixture_helper(&root), PathBuf::from);
        let workspace = root.join("workspace");
        // Exercise the composed host beyond both sockaddr_un and one
        // canonical terminal input line, including shell-sensitive paths.
        let mut artifacts = root.join("artifacts");
        for _ in 0..6 {
            artifacts.push(format!("日本語's-{}", "nested".repeat(15)));
            std::fs::create_dir(&artifacts).unwrap();
            std::fs::set_permissions(&artifacts, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let permission = permission(&workspace);
        let inputs = TerminalHostAuthorityInputs {
            workspace: open(&workspace),
            workspace_path: workspace.clone(),
            default_cwd: workspace,
            environment: vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), root.as_os_str().to_owned()),
            ],
            account_shell: TerminalHostAccountShell::Explicit(Some("/bin/bash".into())),
            cli_executable: cli,
            tmux_executable: tmux,
            artifacts: open(&artifacts),
            artifact_path: artifacts,
        };
        let state = open(&root.join("state"));
        let identity = SessionIncarnationId::new("host-test").unwrap();
        let (tool, resource) = if let Some(contexts) = contexts {
            NativeTerminalHost::compose_with_workspace_on_worker(
                inputs, state, identity, contexts, permission,
            )
        } else {
            match permission {
                Some(permission) => NativeTerminalHost::compose_with_permission_on_worker(
                    inputs, state, identity, permission,
                ),
                None => NativeTerminalHost::compose_on_worker(inputs, state, identity),
            }
        }
        .unwrap();
        Self {
            context: Self::context(),
            root,
            tool: Arc::new(tool),
            completion: resource.completion(),
            resource: Some(resource),
        }
    }
    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("owner").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("turn").unwrap(),
            call_id: ToolCallId::new("call").unwrap(),
        }
    }
    fn future(
        &self,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
        self.future_with_expected_error(arguments, cancellation, false)
    }
    fn future_with_expected_error(
        &self,
        arguments: Value,
        cancellation: CancellationToken,
        expected_error: bool,
    ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
        let call = ToolCall {
            id: ToolCallId::new("call").unwrap(),
            name: ToolName::new("terminal").unwrap(),
            arguments,
        };
        let action = call.arguments["action"].as_str().unwrap().to_owned();
        let prepared = self
            .tool
            .prepare(call)
            .unwrap_or_else(|error| panic!("prepare {action}: {error:?}"));
        let execution = self.tool.execute(
            self.context.clone(),
            prepared.arguments().clone(),
            cancellation,
        );
        Box::pin(async move {
            let output = execution.await?;
            assert_eq!(
                output.is_error, expected_error,
                "{action}: tool failure flag"
            );
            Ok(serde_json::from_value(output.content).unwrap())
        })
    }
    fn action(&self, arguments: Value) -> TerminalActionResult {
        let action = arguments["action"].as_str().unwrap().to_owned();
        self.try_action(arguments)
            .unwrap_or_else(|error| panic!("{action}: {error:?}"))
    }
    fn try_action(&self, arguments: Value) -> Result<TerminalActionResult, ToolError> {
        futures_executor::block_on(self.future(arguments, CancellationToken::new()))
    }
    fn close(&self, id: &TerminalSessionId) -> machine_god_core::TerminalSessionFacts {
        // A bounded native inventory may fail under host load. Observe the
        // retained failure before issuing another explicit close for this
        // exact session. Never replay start, write, or arbitrary errors.
        for attempt in 0..4 {
            match self.try_action(
                json!({"action":"close","session_id":id.as_str(),"close_policy":"force"}),
            ) {
                Ok(TerminalActionResult::Close { session, .. }) => {
                    assert_eq!(&session.session_id, id);
                    assert_eq!(
                        session.lifecycle,
                        machine_god_core::TerminalLifecycle::Closed
                    );
                    return session;
                }
                Ok(_) => panic!("close receipt"),
                Err(error) => {
                    assert_native_cleanup_failure(&error);
                    self.assert_retained_close_failure(id);
                    assert!(
                        attempt < 3,
                        "bounded explicit close recovery exhausted: {error:?}"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        unreachable!("the last attempt returns or fails")
    }
    fn assert_retained_close_failure(&self, id: &TerminalSessionId) {
        let TerminalActionResult::Inspect { session, .. } =
            self.action(json!({"action":"inspect","session_id":id.as_str()}))
        else {
            panic!("retained inspection receipt");
        };
        assert_eq!(&session.session_id, id);
        assert_eq!(session.lifecycle, machine_god_core::TerminalLifecycle::Lost);
        assert!(matches!(
            session.screen_recovery,
            machine_god_core::TerminalScreenRecovery::Unavailable {
                reason: machine_god_core::TerminalScreenUnavailableReason::RawGap
            }
        ));
    }
}
fn assert_native_cleanup_failure(error: &ToolError) {
    assert_eq!(
        error,
        &diagnostic(
            "terminal_registry",
            crate::terminal_registry::TerminalRegistryError::Session(
                crate::terminal_session::TerminalSessionError::Native
            )
        ),
        "only the exact retained native-close failure admits fixture recovery"
    );
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.resource.take());
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.completion.is_complete() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if !self.completion.is_complete() && std::thread::panicking() {
            // Preserve the failed fixture while cleanup still owns it;
            // neither destroy active state nor abort on a second panic.
            return;
        }
        assert!(
            self.completion.is_complete(),
            "host workers and transferred child cleanup joined"
        );
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
#[ignore = "private host helper subprocess"]
fn helper_child() {
    let status = match std::env::var("MACHINE_GOD_TEST_HOST_HELPER").as_deref() {
        Ok(crate::TERMINAL_CAPTURED_HELPER_ARGUMENT) => {
            crate::run_terminal_captured_helper().is_ok()
        }
        Ok(crate::TERMINAL_PTY_HELPER_ARGUMENT) => crate::run_terminal_pty_helper().is_ok(),
        Ok(crate::TERMINAL_STARTUP_MARKER_ARGUMENT) => crate::run_terminal_startup_marker().is_ok(),
        _ => false,
    };
    std::process::exit(if status { 0 } else { 125 });
}

#[cfg(target_os = "macos")]
#[test]
fn full_host_fixture_inventory_dispatch_is_exact_raw_and_deadline_bound() {
    use crate::background_process::inventory_helper_reports_current_process_for_test;
    use crate::process_inventory_helper::ProcessInventoryHelper;

    let mut nonce = [0_u8; 8];
    getrandom::fill(&mut nonce).unwrap();
    let root = std::env::temp_dir().join(format!(
        "mg-host-inventory-{:016x}",
        u64::from_le_bytes(nonce)
    ));
    std::fs::create_dir(&root).unwrap();
    let helper = write_cli_fixture_helper(&root);
    let flag = crate::PROCESS_INVENTORY_HELPER_ARGUMENT;
    let collect = |mut arguments: Vec<std::ffi::OsString>| {
        // A new script's first direct execution can spend the inventory
        // budget before shell entry. Explicit interpretation exercises the
        // same script without warming it or changing the collector bound.
        arguments.insert(0, helper.as_os_str().to_owned());
        inventory_helper_reports_current_process_for_test(
            &ProcessInventoryHelper::new("/bin/sh".into(), arguments).unwrap(),
        )
    };
    // This is the actual fixed CLI wrapper, before constructing a full
    // host. The production collector requires canonical output, EOF and
    // successful reap under its original 250 ms bound.
    let valid = collect(vec![flag.into()]);
    let missing = collect(vec![]);
    let extra = collect(vec![flag.into(), "extra".into()]);
    // Override only the transferred stamp at exec, proving the wrapper
    // does not replace an already-expired deadline with a fresh budget.
    let expired = inventory_helper_reports_current_process_for_test(
        &ProcessInventoryHelper::new(
            "/usr/bin/env".into(),
            vec![
                "MACHINE_GOD_PROCESS_INVENTORY_DEADLINE=0:0".into(),
                "/bin/sh".into(),
                helper.into_os_string(),
                flag.into(),
            ],
        )
        .unwrap(),
    );
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        valid,
        Ok(true),
        "exact helper must emit only canonical PID lines"
    );
    assert!(missing.is_err(), "missing private flag must fail");
    assert!(extra.is_err(), "extra private arguments must fail");
    assert!(expired.is_err(), "expired transferred deadline must fail");
}

#[test]
fn full_host_exec_and_unpolled_start_share_inert_adapter() {
    let fixture = Fixture::new();
    assert!(!fixture.root.join("state/terminal-v1").exists());
    drop(fixture.future(
        json!({"action":"start","command":"printf forbidden > forbidden"}),
        CancellationToken::new(),
    ));
    assert!(!fixture.root.join("workspace/forbidden").exists());
    let TerminalActionResult::Exec { result } =
        futures_executor::block_on(fixture.future_with_expected_error(
            json!({"action":"exec","profile":"clean","command":"printf foreground; exit 7"}),
            CancellationToken::new(),
            true,
        ))
        .unwrap()
    else {
        panic!("exec receipt");
    };
    assert_eq!(
        result.status,
        machine_god_core::TerminalExecStatus::Exited { exit_code: 7 }
    );
    assert!(result.stdout.bytes.ends_with(b"foreground"));
}

#[test]
fn full_host_start_read_screen_resize_inspect_and_close() {
    let fixture = Fixture::new();
    let TerminalActionResult::Start { session, .. } = fixture.action(json!({"action":"start","profile":"clean","command":"printf host-ready; exec /bin/sleep 30"})) else { panic!("start receipt"); };
    let id = session.session_id;
    let wait = fixture.action(json!({"action":"wait","session_id":id.as_str(),"return_when":{"kind":"match","pattern":"host-ready"},"wait_ceiling_ms":5000}));
    assert!(matches!(
        wait,
        TerminalActionResult::Wait {
            outcome: machine_god_core::TerminalReturnOutcome::ConditionMet {},
            ..
        }
    ));
    let TerminalActionResult::Read { output, .. } =
        fixture.action(json!({"action":"read","session_id":id.as_str(),"cursor_segment":1}))
    else {
        panic!("read receipt");
    };
    assert!(
        output
            .windows(b"host-ready".len())
            .any(|bytes| bytes == b"host-ready")
    );
    assert!(matches!(
        fixture.action(json!({"action":"screen","session_id":id.as_str()})),
        TerminalActionResult::Screen { .. }
    ));
    assert!(matches!(
        fixture.action(json!({"action":"resize","session_id":id.as_str(),"rows":31,"columns":97})),
        TerminalActionResult::Resize { .. }
    ));
    let TerminalActionResult::Inspect { cwd, .. } =
        fixture.action(json!({"action":"inspect","session_id":id.as_str()}))
    else {
        panic!("inspect receipt");
    };
    assert_eq!(cwd, fixture.root.join("workspace").to_str().unwrap());
    fixture.close(&id);
    let TerminalActionResult::List { sessions } = fixture.action(json!({"action":"list"})) else {
        panic!("list receipt");
    };
    assert!(sessions.iter().any(|session| session.session_id == id));
}

#[test]
fn full_host_list_resolves_workspace_filters_without_expanding_owner_authority() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    std::fs::create_dir_all(workspace.join("real/deep")).unwrap();
    std::os::unix::fs::symlink("real/deep", workspace.join("link")).unwrap();
    let TerminalActionResult::Start { session, .. } = fixture.action(json!({
        "action":"start", "profile":"clean", "command":"exec /bin/sleep 30"
    })) else {
        panic!("start receipt");
    };
    fixture.close(&session.session_id);
    let ids = |arguments| {
        let TerminalActionResult::List { sessions } = fixture.action(arguments) else {
            panic!("list receipt");
        };
        sessions
            .into_iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>()
    };
    let expected = ids(json!({"action":"list"}));
    assert_eq!(expected, vec![session.session_id]);
    for root in [
        ".".to_owned(),
        workspace.display().to_string(),
        format!("{}/.", workspace.display()),
        "link/../..".to_owned(),
        " \t.\r\n".to_owned(),
        "~/workspace".to_owned(),
        "~//workspace".to_owned(),
    ] {
        assert_eq!(
            ids(json!({"action":"list","workspace_root":root})),
            expected,
            "{root}"
        );
    }
    // Native link/.. is real, not the workspace produced by lexical removal.
    for root in [
        "link/..".to_owned(),
        fixture.root.display().to_string(),
        "..".to_owned(),
    ] {
        assert!(ids(json!({"action":"list","workspace_root":root})).is_empty());
    }
    assert!(ids(json!({"action":"list","workspace_root":".","task_id":"other-owner"})).is_empty());
    let mut foreign = Fixture::context();
    foreign.session_id = SessionId::new("foreign-owner").unwrap();
    let prepared = fixture
        .tool
        .prepare(ToolCall {
            id: foreign.call_id.clone(),
            name: ToolName::new("terminal").unwrap(),
            arguments: json!({"action":"list","workspace_root":".","task_id":"owner"}),
        })
        .unwrap();
    let output = futures_executor::block_on(fixture.tool.execute(
        foreign,
        prepared.arguments().clone(),
        CancellationToken::new(),
    ))
    .unwrap();
    assert_eq!(output.content["sessions"], json!([]));
    for root in ["missing", "missing/..", " \t ", "~other"] {
        assert!(
            futures_executor::block_on(fixture.future(
                json!({"action":"list","workspace_root":root}),
                CancellationToken::new()
            ))
            .is_err(),
            "{root}"
        );
    }
}

#[test]
fn full_host_filtered_list_is_inert_until_polled_and_cancelled_before_submission() {
    let fixture = Fixture::new();
    let arguments = json!({"action":"list","workspace_root":"."});
    drop(fixture.future(arguments.clone(), CancellationToken::new()));
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(futures_executor::block_on(fixture.future(arguments, cancelled)).is_err());
    assert!(!fixture.root.join("state/terminal-v1").exists());
}

#[test]
fn full_host_shutdown_joins_foreground_work_with_an_unpolled_receipt() {
    use std::task::{Context, Poll};
    let mut fixture = Fixture::new();
    let resource = fixture.resource.take().unwrap();
    let completion = resource.completion();
    let mut execution = fixture.future(
        json!({"action":"exec","profile":"clean","command":"printf ready > exec-ready; exec /bin/sleep 30"}),
        CancellationToken::new(),
    );
    let mut context = Context::from_waker(std::task::Waker::noop());
    assert!(matches!(
        execution.as_mut().poll(&mut context),
        Poll::Pending
    ));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !fixture.root.join("workspace/exec-ready").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(fixture.root.join("workspace/exec-ready").exists());
    assert!(!completion.is_complete());
    drop(resource);
    let deadline = Instant::now() + Duration::from_secs(15);
    while !completion.is_complete() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        completion.is_complete(),
        "native cleanup must not require polling the response"
    );
    assert!(futures_executor::block_on(execution).is_err());
}

#[test]
fn full_host_dormant_shutdown_does_not_need_to_initialize_the_owner() {
    let mut fixture = Fixture::new();
    let resource = fixture.resource.take().unwrap();
    let completion = resource.completion();
    let unpolled = fixture.future(json!({"action":"start"}), CancellationToken::new());
    assert!(!completion.is_complete());
    drop(resource);
    assert!(completion.is_complete());
    assert!(!fixture.root.join("state/terminal-v1").exists());
    assert!(futures_executor::block_on(unpolled).is_err());
}

#[test]
fn full_host_interactive_write_monitor_probe_and_signal() {
    let fixture = Fixture::new();
    let TerminalActionResult::Start { session, .. } =
        fixture.action(json!({"action":"start","profile":"clean"}))
    else {
        panic!("start receipt");
    };
    let id = session.session_id;
    fixture.action(json!({"action":"write","session_id":id.as_str(),"lease":"acquire"}));
    let TerminalActionResult::Write { accepted_bytes, .. } = fixture.action(json!({"action":"write","session_id":id.as_str(),"lease":"use","write":{"kind":"text","text":"printf write-ready\\n\n"}})) else { panic!("write receipt"); };
    assert!(accepted_bytes > 0);
    fixture.action(json!({"action":"wait","session_id":id.as_str(),"return_when":{"kind":"match","pattern":"write-ready"},"wait_ceiling_ms":5000}));
    let definition = json!({"condition":{"kind":"custom_probe","command":"printf probe-ran > probe-ran", "cwd":"."},"check_interval_ms":10,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_session_end"}});
    let TerminalActionResult::Monitor { monitor_id: Some(monitor), .. } = fixture.action(json!({"action":"monitor","session_id":id.as_str(),"monitor":{"kind":"add","definition":definition}})) else { panic!("monitor receipt"); };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.root.join("workspace/probe-ran").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        fixture.root.join("workspace/probe-ran").exists(),
        "ordinary owner pump executes authorized grant"
    );
    for operation in ["pause", "resume", "remove"] {
        fixture.action(json!({"action":"monitor","session_id":id.as_str(),"monitor":{"kind":operation,"monitor_id":monitor.as_str()}}));
    }
    fixture.action(json!({"action":"signal","session_id":id.as_str(),"signal":"terminate"}));
    fixture.close(&id);
}

#[test]
fn full_host_failed_close_retains_history_and_explicit_cleanup_authority() {
    struct InjectedSnapshotFailure(NonZeroU32);
    impl Drop for InjectedSnapshotFailure {
        fn drop(&mut self) {
            crate::background_process::inject_group_snapshot_spawn_failures_for_test(self.0, 0);
        }
    }
    let _guard = crate::background_process::GROUP_SNAPSHOT_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = Fixture::new();
    let TerminalActionResult::Start { session, .. } = fixture.action(json!({
        "action":"start", "profile":"clean",
        "command":"printf '%s\\n' \"$$\" > close-owner.pid; printf 'once\\n' >> close-count; printf CLOSE_RETAINED; exec /bin/sleep 30"
    })) else {
        panic!("start receipt");
    };
    let id = session.session_id;
    assert!(matches!(
        fixture.action(json!({"action":"wait","session_id":id.as_str(),"return_when":{"kind":"match","pattern":"CLOSE_RETAINED"},"wait_ceiling_ms":5000})),
        TerminalActionResult::Wait {
            outcome: machine_god_core::TerminalReturnOutcome::ConditionMet {},
            ..
        }
    ));
    // The owned command's PID only selects a test fault; production close
    // still obtains all authority through the exact retained session.
    let pid = std::fs::read_to_string(fixture.root.join("workspace/close-owner.pid"))
        .unwrap()
        .trim()
        .parse::<NonZeroU32>()
        .unwrap();
    let injection = InjectedSnapshotFailure(pid);
    crate::background_process::inject_group_snapshot_spawn_failures_for_test(pid, 1);
    let failed = fixture
        .try_action(json!({"action":"close","session_id":id.as_str(),"close_policy":"force"}));
    drop(injection);
    assert_native_cleanup_failure(&failed.unwrap_err());
    fixture.assert_retained_close_failure(&id);
    let closed = fixture.close(&id);
    assert!(matches!(
        closed.screen_recovery,
        machine_god_core::TerminalScreenRecovery::Unavailable {
            reason: machine_god_core::TerminalScreenUnavailableReason::RawGap
        }
    ));
    let TerminalActionResult::Read { output, .. } =
        fixture.action(json!({"action":"read","session_id":id.as_str(),"cursor_segment":1}))
    else {
        panic!("retained output receipt");
    };
    assert!(
        output
            .windows(b"CLOSE_RETAINED".len())
            .any(|bytes| bytes == b"CLOSE_RETAINED")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("workspace/close-count")).unwrap(),
        "once\n"
    );
}
