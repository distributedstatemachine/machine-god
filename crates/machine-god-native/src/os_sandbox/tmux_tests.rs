#[cfg(target_os = "macos")]
#[test]
fn sandbox_real_tmux_shell_inherits_os_write_policy() {
    let _serial = crate::os_sandbox::NATIVE_TESTS.lock().unwrap();
    let directory = Directory::new();
    let extra = Directory::new();
    let outside = Directory::new();
    let outside_path = outside.0.canonicalize().unwrap();
    assert!(!outside_path.starts_with("/private/tmp") && !outside_path.starts_with("/tmp"));
    let Some(mut request) = request(
        &directory,
        "printf allowed > ok; printf extra > \"$EXTRA/ok\"; printf denied > \"$OUTSIDE/denied\"; /bin/sh -c 'printf denied > \"$OUTSIDE/descendant\"'; printf SANDBOX_DONE; exec /bin/sleep 30",
    ) else {
        assert!(
            std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY").is_none(),
            "configured tmux must exist"
        );
        return;
    };
    request
        .environment
        .push(("OUTSIDE".into(), outside_path.as_os_str().to_owned()));
    request.environment.push(("EXTRA".into(), extra.0.as_os_str().to_owned()));
    request.sandbox = Some(std::sync::Arc::new(
        crate::NativeSandboxLaunch::capture(
            crate::NativeSandboxMode::Os,
            crate::PermissionMode::Ask,
            [&directory.0, &extra.0]
                .into_iter()
                .map(|path| crate::NativeSandboxRoot::new(
                    std::fs::File::open(path).unwrap(), path.canonicalize().unwrap(),
                ).unwrap())
                .collect(),
            Some(std::fs::File::open(crate::NATIVE_SANDBOX_EXECUTABLE).unwrap()),
            false,
            Instant::now() + Duration::from_secs(2),
            &CancellationToken::new(),
        ).unwrap(),
    ));
    let prepared = PreparedTerminalTmuxLaunch::prepare(request, &CancellationToken::new()).unwrap();
    assert!(!directory.0.join("ok").exists());
    let mut backend = prepared.commit_owned(&CancellationToken::new()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    while !output.windows(12).any(|bytes| bytes == b"SANDBOX_DONE") {
        assert!(
            Instant::now() < deadline,
            "sandbox tmux marker missing: {output:?}"
        );
        let mut bytes = [0; 4096];
        let read = backend.read(&mut bytes).unwrap();
        output.extend_from_slice(&bytes[..read.bytes_read]);
        std::thread::sleep(PAUSE);
    }
    assert_eq!(std::fs::read(directory.0.join("ok")).unwrap(), b"allowed");
    assert_eq!(std::fs::read(extra.0.join("ok")).unwrap(), b"extra");
    assert!(!outside.0.join("denied").exists());
    assert!(!outside.0.join("descendant").exists());
    backend.close(true, &mut |_| {}).unwrap();
}
