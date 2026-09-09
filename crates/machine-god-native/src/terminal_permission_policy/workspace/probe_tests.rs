// Included inside terminal_probe_effects::tests to exercise the real grant and executor.
#[test]
fn workspace_probe_grant_outlives_source_turn_but_stale_grant_creation_is_denied() {
    let fixture = Fixture::new();
    let permission = crate::terminal_permission_policy::tests::Fixture::new(
        crate::PermissionMode::Ask, crate::NativeSandboxMode::None,
    );
    let contexts = Arc::new(crate::NativeWorkspaceContexts::new());
    let workspace_owner = contexts.register(&permission.session).unwrap();
    let authority = crate::NativeWorkspaceAuthority::open_blocking(
        directory(&fixture.root), fixture.root.clone(), None, fixture.root.with_extension("state"), vec![], false,
    ).unwrap();
    let (turn, _permission_registration, key) = permission.turn();
    let registration = workspace_owner.begin(&turn, authority.snapshot().unwrap()).unwrap();
    let policy = crate::NativeTerminalPermissionPolicy::new(vec![], None).unwrap().with_workspace_contexts(contexts);
    policy.bind_controller(&permission.controller).unwrap();
    let launch = policy.capture_on_worker(&key, Instant::now()+Duration::from_secs(5), &CancellationToken::new()).unwrap();
    let context = Arc::new(TerminalProbeCustomContext::new(
        TerminalShell::from_executable("/bin/bash".as_ref(), true).unwrap().with_sandbox(launch.clone()),
        vec![("PATH".into(), "/usr/bin:/bin".into())],
    ).unwrap());
    let candidate = || TerminalProbeAuthority::Custom {
        command: "printf continued > after-turn; printf done".into(), cwd: ".".into(),
        canonical_cwd: fixture.root.clone(), directory: directory(&fixture.root), context: context.clone(),
    };
    let installed = grant(candidate());
    drop(registration);
    assert_eq!(launch.revalidate(Instant::now()+Duration::from_secs(5), &CancellationToken::new()).unwrap_err(), crate::NativeSandboxError::Unavailable);
    assert!(matches!(TerminalProbeGrant::new(owner(), TerminalSessionId::new("terminal-1").unwrap(), TerminalMonitorId::new("monitor-2").unwrap(), 1, candidate()), Err(TerminalProbeFailure::Denied)));
    let evidence = fixture.run(installed.clone());
    assert!(evidence.result.is_ok());
    assert_eq!(std::fs::read(fixture.root.join("after-turn")).unwrap(), b"continued");
    let queued = bind(installed.clone());
    installed.revoke();
    assert!(futures_executor::block_on(fixture.executor.execute(queued, CancellationToken::new())).result.is_err());
}
