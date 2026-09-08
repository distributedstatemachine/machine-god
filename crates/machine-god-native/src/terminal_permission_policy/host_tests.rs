// Included beneath terminal_host::tests to reuse the real composed host fixture.
use super::*;
use crate::terminal_permission_policy::tests::Fixture as PolicyFixture;
use crate::{NativeSandboxMode, PermissionMode};

#[test]
fn permission_host_unpolled_is_inert_and_missing_route_never_launches() {
    let policy = Arc::new(NativeTerminalPermissionPolicy::new(vec![], None).unwrap());
    let fixture = Fixture::with_permission(|_| Some(policy));
    for arguments in [
        json!({"action":"exec","profile":"clean","command":"printf forbidden > forbidden"}),
        json!({"action":"start","profile":"clean","command":"printf forbidden > forbidden"}),
    ] {
        let future = fixture.future(arguments.clone(), CancellationToken::new());
        drop(future);
        assert!(!fixture.root.join("workspace/forbidden").exists());
        assert!(!fixture.root.join("state/terminal-v1").exists());
        assert!(fixture.try_action(arguments).is_err());
        assert!(!fixture.root.join("workspace/forbidden").exists());
    }
}

#[test]
fn permission_host_taken_os_missing_executable_never_falls_back() {
    let owner = PolicyFixture::new(PermissionMode::Auto, NativeSandboxMode::Os);
    let (_turn, _registration, context) = owner.turn();
    let policy = Arc::new(NativeTerminalPermissionPolicy::new(vec![], None).unwrap());
    policy.bind_controller(&owner.controller).unwrap();
    let mut fixture = Fixture::with_permission(|_| Some(policy));
    fixture.context = context;
    owner.owner.set_mode(PermissionMode::Yolo);
    owner.owner.set_sandbox_mode(NativeSandboxMode::None);
    owner.owner.reset().unwrap();
    for action in ["exec", "start"] {
        assert!(fixture.try_action(json!({"action":action,"profile":"clean","command":"printf forbidden > forbidden"})).is_err());
        assert!(!fixture.root.join("workspace/forbidden").exists());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn permission_host_taken_os_routes_foreground_pty_and_custom_monitors() {
    use crate::{NATIVE_SANDBOX_EXECUTABLE, NativeSandboxRoot};
    use std::fs::File;

    let owner = PolicyFixture::new(PermissionMode::Auto, NativeSandboxMode::Os);
    let (_turn, registration, context) = owner.turn();
    let mut fixture = Fixture::with_permission(|workspace| {
        // The Darwin fixture must not live under the pinned writable /tmp exception.
        assert!(!workspace.starts_with("/private/tmp") && !workspace.starts_with("/tmp"));
        let root =
            NativeSandboxRoot::new(File::open(workspace).unwrap(), workspace.to_owned()).unwrap();
        let policy = Arc::new(
            NativeTerminalPermissionPolicy::new(
                vec![root],
                Some(File::open(NATIVE_SANDBOX_EXECUTABLE).unwrap()),
            )
            .unwrap(),
        );
        policy.bind_controller(&owner.controller).unwrap();
        Some(policy)
    });
    fixture.context = context;
    owner.owner.set_mode(PermissionMode::Yolo);
    owner.owner.set_sandbox_mode(NativeSandboxMode::None);
    owner.owner.reset().unwrap();
    let outside = fixture.root.join("forbidden");
    let quoted = outside.to_str().unwrap().replace('\'', "'\\''");
    fixture.action(json!({"action":"exec","profile":"clean","command":format!("printf allowed > foreground; (printf no > '{quoted}') 2>/dev/null; printf done")}));
    assert!(fixture.root.join("workspace/foreground").exists());
    assert!(!outside.exists());
    let initial = json!({"condition":{"kind":"custom_probe","command":format!("(printf no > '{quoted}') 2>/dev/null; printf allowed > initial-probe"),"cwd":"."},"check_interval_ms":10,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_session_end"}});
    let TerminalActionResult::Start { session, .. } = fixture.action(json!({"action":"start","profile":"clean","initial_monitors":[initial],"command":format!("printf allowed > pty; (printf no > '{quoted}') 2>/dev/null; exec /bin/sleep 30")})) else { panic!("start receipt"); };
    let id = session.session_id;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.root.join("workspace/pty").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(fixture.root.join("workspace/pty").exists());
    assert!(!outside.exists());
    let definition = json!({"condition":{"kind":"custom_probe","command":format!("(printf no > '{quoted}') 2>/dev/null; printf allowed > probe"),"cwd":"."},"check_interval_ms":10,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_session_end"}});
    fixture.action(json!({"action":"monitor","session_id":id.as_str(),"monitor":{"kind":"add","definition":definition}}));
    // The monitor retains the admission-time snapshot after its source turn ends.
    drop(registration);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.root.join("workspace/probe").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(fixture.root.join("workspace/probe").exists());
    assert!(fixture.root.join("workspace/initial-probe").exists());
    assert!(!outside.exists());
    fixture.close(&id);
}
