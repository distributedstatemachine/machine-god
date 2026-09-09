use super::*;
use machine_god_core::{SessionId, SessionIncarnationId};
use rustix::fs::{Mode, OFlags};
use std::net::TcpListener;
use std::sync::atomic::AtomicU64;

static NEXT: AtomicU64 = AtomicU64::new(0);
fn owner() -> BackgroundOutputOwner {
    BackgroundOutputOwner::new(
        SessionId::new("probe-owner").unwrap(),
        SessionIncarnationId::new("probe-incarnation").unwrap(),
    )
}
fn grant(authority: TerminalProbeAuthority) -> Arc<TerminalProbeGrant> {
    Arc::new(
        TerminalProbeGrant::new(
            owner(),
            TerminalSessionId::new("terminal-probe").unwrap(),
            TerminalMonitorId::new("monitor-1").unwrap(),
            1,
            authority,
        )
        .unwrap(),
    )
}
fn request(grant: &TerminalProbeGrant) -> TerminalProbeRequest {
    TerminalProbeRequest {
        session_id: grant.session_id().clone(),
        monitor_id: grant.monitor_id().clone(),
        generation: grant.generation(),
        request_sequence: 7,
        started_at_ms: 100,
        deadline_ms: 2100,
        output_limit_bytes: PROBE_OUTPUT_BYTES,
        target: grant.target().clone(),
    }
}
fn bind(grant: Arc<TerminalProbeGrant>) -> AuthorizedTerminalProbe {
    AuthorizedTerminalProbe::new(
        &owner(),
        request(&grant),
        grant,
        TerminalProbeClock {
            now_ms: 100,
            observed_at: Instant::now(),
        },
    )
    .unwrap()
}
fn directory(path: &std::path::Path) -> OwnedFd {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .unwrap()
}
struct Fixture {
    root: PathBuf,
    executor: NativeTerminalProbeExecutor,
    harness_bytes: usize,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "machine-god-probe-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let (program, arguments, harness_bytes) = std::env::var_os(
            "MACHINE_GOD_TERMINAL_RELEASE_BINARY",
        )
        .map_or_else(
            || {
                (
                    std::env::current_exe().unwrap(),
                    vec![
                        "--exact".into(),
                        "terminal_captured_exec::tests::captured_helper_child".into(),
                        "--ignored".into(),
                        "--nocapture".into(),
                        "--quiet".into(),
                    ],
                    b"\nrunning 1 test\n".len(),
                )
            },
            |program| {
                (
                    PathBuf::from(program),
                    vec![crate::terminal_captured_exec::TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
                    0,
                )
            },
        );
        let captured = Arc::new(
            TerminalCapturedExec::new(program, arguments, Duration::from_secs(20), 4)
                .unwrap()
                .with_test_inventory_helper(),
        );
        Self {
            root,
            executor: NativeTerminalProbeExecutor::new(captured, 2).unwrap(),
            harness_bytes,
        }
    }
    fn custom(&self, command: String) -> Arc<TerminalProbeGrant> {
        grant(TerminalProbeAuthority::Custom {
            command,
            cwd: ".".into(),
            canonical_cwd: self.root.clone(),
            directory: directory(&self.root),
            context: Arc::new(
                TerminalProbeCustomContext::new(
                    TerminalShell::from_executable("/bin/bash".as_ref(), true).unwrap(),
                    vec![("PATH".into(), "/usr/bin:/bin".into())],
                )
                .unwrap(),
            ),
        })
    }
    fn run(&self, grant: Arc<TerminalProbeGrant>) -> TerminalProbeEvidence {
        futures_executor::block_on(self.executor.execute(bind(grant), CancellationToken::new()))
    }
    fn settled(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.executor.active.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(self.executor.active.load(Ordering::Acquire), 0);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.settled();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

// Exercise the real grant and executor using the shared probe fixture.
#[test]
fn workspace_probe_grant_outlives_source_turn_but_stale_grant_creation_is_denied() {
    let fixture = Fixture::new();
    let permission = crate::terminal_permission_policy::tests::Fixture::new(
        crate::PermissionMode::Ask,
        crate::NativeSandboxMode::None,
    );
    let contexts = Arc::new(crate::NativeWorkspaceContexts::new());
    let workspace_owner = contexts.register(&permission.session).unwrap();
    let authority = crate::NativeWorkspaceAuthority::open_blocking(
        directory(&fixture.root),
        fixture.root.clone(),
        None,
        fixture.root.with_extension("state"),
        vec![],
        false,
    )
    .unwrap();
    let (turn, _permission_registration, key) = permission.turn();
    let registration = workspace_owner
        .begin(&turn, authority.snapshot().unwrap())
        .unwrap();
    let policy = crate::NativeTerminalPermissionPolicy::new(vec![], None)
        .unwrap()
        .with_workspace_contexts(contexts);
    policy.bind_controller(&permission.controller).unwrap();
    let launch = policy
        .capture_on_worker(
            &key,
            Instant::now() + Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .unwrap();
    let context = Arc::new(
        TerminalProbeCustomContext::new(
            TerminalShell::from_executable("/bin/bash".as_ref(), true)
                .unwrap()
                .with_sandbox(launch.clone()),
            vec![("PATH".into(), "/usr/bin:/bin".into())],
        )
        .unwrap(),
    );
    let candidate = || TerminalProbeAuthority::Custom {
        command: "printf continued > after-turn; printf done".into(),
        cwd: ".".into(),
        canonical_cwd: fixture.root.clone(),
        directory: directory(&fixture.root),
        context: context.clone(),
    };
    let installed = grant(candidate());
    drop(registration);
    assert_eq!(
        launch
            .revalidate(
                Instant::now() + Duration::from_secs(5),
                &CancellationToken::new()
            )
            .unwrap_err(),
        crate::NativeSandboxError::Unavailable
    );
    assert!(matches!(
        TerminalProbeGrant::new(
            owner(),
            TerminalSessionId::new("terminal-1").unwrap(),
            TerminalMonitorId::new("monitor-2").unwrap(),
            1,
            candidate()
        ),
        Err(TerminalProbeFailure::Denied)
    ));
    let evidence = fixture.run(installed.clone());
    assert!(evidence.result.is_ok());
    assert_eq!(
        std::fs::read(fixture.root.join("after-turn")).unwrap(),
        b"continued"
    );
    let queued = bind(installed.clone());
    installed.revoke();
    assert!(
        futures_executor::block_on(fixture.executor.execute(queued, CancellationToken::new()))
            .result
            .is_err()
    );
}

#[test]
fn probe_grants_bind_owner_generation_target_sequence_and_budget_without_effects() {
    let fixture = Fixture::new();
    let grant = fixture.custom("printf forbidden > forbidden".into());
    let clock = TerminalProbeClock {
        now_ms: 100,
        observed_at: Instant::now(),
    };
    for field in 0..8 {
        let mut request = request(&grant);
        match field {
            0 => request.session_id = TerminalSessionId::new("other").unwrap(),
            1 => request.monitor_id = TerminalMonitorId::new("monitor-2").unwrap(),
            2 => request.generation += 1,
            3 => {
                request.target = TerminalProbeTarget::Path {
                    path: "forbidden".into(),
                }
            }
            4 => request.request_sequence = 0,
            5 => request.deadline_ms += 1,
            6 => request.output_limit_bytes += 1,
            _ => {
                if let TerminalProbeTarget::Custom {
                    approved_cwd_sha256,
                    ..
                } = &mut request.target
                {
                    *approved_cwd_sha256 = [0; 32];
                }
            }
        }
        assert!(
            AuthorizedTerminalProbe::new(&owner(), request, Arc::clone(&grant), clock).is_err()
        );
    }
    let wrong_owner = BackgroundOutputOwner::new(
        SessionId::new("probe-owner").unwrap(),
        SessionIncarnationId::new("other").unwrap(),
    );
    assert!(matches!(
        AuthorizedTerminalProbe::new(&wrong_owner, request(&grant), Arc::clone(&grant), clock),
        Err(TerminalProbeFailure::Denied)
    ));
    assert!(!fixture.root.join("forbidden").exists());
    let first = fixture.run(Arc::clone(&grant));
    let second = fixture.run(Arc::clone(&grant));
    for evidence in [first, second] {
        assert_eq!(evidence.session_id, *grant.session_id());
        assert_eq!(evidence.monitor_id, *grant.monitor_id());
        assert_eq!(evidence.generation, grant.generation());
        assert_eq!(evidence.request_sequence, 7);
        assert!(matches!(
            evidence.result,
            Ok(TerminalProbeObservation::Custom { exit_code: 0, .. })
        ));
    }
}

#[test]
fn probe_custom_capture_exact_limit_and_one_over_do_not_weaken_public_exec() {
    let fixture = Fixture::new();
    for over in [0, 1] {
        let bytes = PROBE_OUTPUT_BYTES - fixture.harness_bytes + over;
        let evidence = fixture.run(fixture.custom(format!("/usr/bin/head -c {bytes} /dev/zero")));
        assert_eq!(evidence.output_bytes, (PROBE_OUTPUT_BYTES + over) as u64);
        assert_eq!(evidence.truncated, over != 0);
        if over == 0 {
            assert!(matches!(
                evidence.result,
                Ok(TerminalProbeObservation::Custom { exit_code: 0, .. })
            ));
        } else {
            assert!(matches!(
                evidence.result,
                Err(TerminalProbeFailure::OutputLimit)
            ));
        }
    }
    let evidence = fixture.run(fixture.custom("exit 19".into()));
    assert!(matches!(
        evidence.result,
        Ok(TerminalProbeObservation::Custom { exit_code: 19, .. })
    ));
    let evidence = fixture.run(fixture.custom("kill -KILL $$".into()));
    assert!(matches!(
        evidence.result,
        Err(TerminalProbeFailure::Unavailable)
    ));
}

#[test]
fn probe_queue_delay_and_custom_timeout_keep_original_deadline() {
    let fixture = Fixture::new();
    let grant = fixture.custom("printf forbidden > forbidden".into());
    let expired = AuthorizedTerminalProbe::new(
        &owner(),
        request(&grant),
        grant,
        TerminalProbeClock {
            now_ms: 100,
            observed_at: Instant::now().checked_sub(Duration::from_secs(3)).unwrap(),
        },
    )
    .unwrap();
    let evidence =
        futures_executor::block_on(fixture.executor.execute(expired, CancellationToken::new()));
    assert!(evidence.timed_out);
    assert!(matches!(
        evidence.result,
        Err(TerminalProbeFailure::Timeout)
    ));
    assert!(!fixture.root.join("forbidden").exists());
    let grant = fixture.custom("exec /bin/sleep 30".into());
    let short = AuthorizedTerminalProbe::new(
        &owner(),
        request(&grant),
        grant,
        TerminalProbeClock {
            now_ms: 2000,
            observed_at: Instant::now(),
        },
    )
    .unwrap();
    let started = Instant::now();
    let evidence =
        futures_executor::block_on(fixture.executor.execute(short, CancellationToken::new()));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(evidence.timed_out);
    assert!(matches!(
        evidence.result,
        Err(TerminalProbeFailure::Timeout)
    ));
}

#[test]
fn probe_path_baselines_use_retained_parent_and_do_not_follow_leaf_symlinks() {
    let fixture = Fixture::new();
    let grant = grant(TerminalProbeAuthority::Path {
        path: "ready".into(),
        parent: directory(&fixture.root),
        leaf: "ready".into(),
    });
    let absent = fixture.run(Arc::clone(&grant));
    assert!(matches!(
        absent.result,
        Ok(TerminalProbeObservation::Path {
            baseline: TerminalPathBaseline {
                exists: false,
                size: 0,
                modified_ns: 0
            }
        })
    ));
    std::fs::write(fixture.root.join("ready"), b"12345").unwrap();
    let present = fixture.run(Arc::clone(&grant));
    assert!(matches!(
        present.result,
        Ok(TerminalProbeObservation::Path {
            baseline: TerminalPathBaseline {
                exists: true,
                size: 5,
                ..
            }
        })
    ));
    std::fs::remove_file(fixture.root.join("ready")).unwrap();
    std::os::unix::fs::symlink("unavailable-destination", fixture.root.join("ready")).unwrap();
    let link = fixture.run(grant);
    assert!(matches!(
        link.result,
        Ok(TerminalProbeObservation::Path {
            baseline: TerminalPathBaseline { exists: true, .. }
        })
    ));
    for leaf in ["", "..", "../b", "a//b", "a/./b", "/a"] {
        assert!(
            TerminalProbeGrant::new(
                owner(),
                TerminalSessionId::new("terminal-probe").unwrap(),
                TerminalMonitorId::new("monitor-1").unwrap(),
                1,
                TerminalProbeAuthority::Path {
                    path: "ready".into(),
                    parent: directory(&fixture.root),
                    leaf: leaf.into()
                }
            )
            .is_err()
        );
    }
}

#[test]
fn probe_custom_directory_replacement_cannot_redirect_execution() {
    let fixture = Fixture::new();
    let grant = fixture.custom("printf forbidden > forbidden".into());
    let moved = fixture.root.with_extension("retained");
    std::fs::rename(&fixture.root, &moved).unwrap();
    std::fs::create_dir(&fixture.root).unwrap();
    let evidence = fixture.run(grant);
    assert!(matches!(evidence.result, Err(TerminalProbeFailure::Denied)));
    assert!(!fixture.root.join("forbidden").exists());
    assert!(!moved.join("forbidden").exists());
    std::fs::remove_dir_all(moved).unwrap();
}

fn http_server(response: Vec<u8>) -> (SocketAddr, std::thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            if let Ok((stream, _)) = listener.accept() {
                break stream;
            }
            assert!(Instant::now() < until, "probe never connected");
            std::thread::sleep(Duration::from_millis(5));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = Vec::new();
        let mut bytes = [0; 512];
        while !request.ends_with(b"\r\n\r\n") {
            let count = stream.read(&mut bytes).unwrap();
            assert!(count > 0 && request.len() + count < PROBE_OUTPUT_BYTES);
            request.extend_from_slice(&bytes[..count]);
        }
        let _ = stream.write_all(&response);
        request
    });
    (address, server)
}

#[test]
fn probe_native_http_uses_approved_address_exact_target_and_never_redirects() {
    let fixture = Fixture::new();
    let redirect = TcpListener::bind("127.0.0.1:0").unwrap();
    redirect.set_nonblocking(true).unwrap();
    let response = format!(
        "HTTP/1.0 302 Found\r\nLocation: http://{}/redirect\r\n\r\n",
        redirect.local_addr().unwrap()
    )
    .into_bytes();
    let (address, server) = http_server(response.clone());
    let grant = grant(TerminalProbeAuthority::Http {
        url: format!("http://{address}/ready?name=%E7%95%8C#ignored"),
        addresses: vec![address],
    });
    let evidence = fixture.run(grant);
    match evidence.result {
        Ok(TerminalProbeObservation::Http { response_prefix }) => {
            assert_eq!(response_prefix, response);
        }
        _ => panic!("HTTP probe failed"),
    }
    assert_eq!(evidence.output_bytes, response.len() as u64);
    assert!(!evidence.truncated && !evidence.timed_out);
    assert_eq!(
        server.join().unwrap(),
        b"GET /ready?name=%E7%95%8C HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    );
    assert!(matches!(redirect.accept(),Err(error) if error.kind()==io::ErrorKind::WouldBlock));
    for url in [
        "https://localhost/",
        "http://user@localhost/",
        "http://@localhost/",
    ] {
        assert!(
            TerminalProbeGrant::new(
                owner(),
                TerminalSessionId::new("terminal-probe").unwrap(),
                TerminalMonitorId::new("monitor-1").unwrap(),
                1,
                TerminalProbeAuthority::Http {
                    url: url.into(),
                    addresses: vec!["127.0.0.1:80".parse().unwrap()]
                }
            )
            .is_err()
        );
    }
}

#[test]
fn probe_http_prefix_is_bounded_and_tcp_connections_are_real() {
    let fixture = Fixture::new();
    let mut response = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&vec![b'x'; 32 * 1024]);
    let (address, server) = http_server(response);
    let evidence = fixture.run(grant(TerminalProbeAuthority::Http {
        url: format!("http://{address}"),
        addresses: vec![address],
    }));
    assert_eq!(evidence.output_bytes, HTTP_PREFIX_BYTES as u64);
    assert!(
        matches!(evidence.result,Ok(TerminalProbeObservation::Http {response_prefix}) if response_prefix.len()==HTTP_PREFIX_BYTES)
    );
    server.join().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let approved = grant(TerminalProbeAuthority::Tcp {
        host: "127.0.0.1".into(),
        port: address.port(),
        addresses: vec![address],
    });
    assert!(matches!(
        fixture.run(Arc::clone(&approved)).result,
        Ok(TerminalProbeObservation::Tcp { connected: true })
    ));
    drop(listener);
    assert!(matches!(
        fixture.run(approved).result,
        Ok(TerminalProbeObservation::Tcp { connected: false })
    ));
}

#[test]
fn scoped_probe_is_inert_and_closed_scope_rejects_native_effects() {
    let mut fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    fixture.executor = NativeTerminalProbeExecutor::new(Arc::clone(&fixture.executor.captured), 2)
        .unwrap()
        .with_worker_scope(scope.clone());
    let approved = fixture.custom("printf forbidden > forbidden".into());
    drop(
        fixture
            .executor
            .execute(bind(Arc::clone(&approved)), CancellationToken::new()),
    );
    scope.close();
    scope.completion().wait_on_worker().unwrap();
    assert!(matches!(
        fixture.run(approved).result,
        Err(TerminalProbeFailure::Unavailable)
    ));
    assert!(!fixture.root.join("forbidden").exists());
    assert_eq!(fixture.executor.active.load(Ordering::Acquire), 0);
}

#[test]
fn probe_revocation_denies_queued_clones_and_stops_unpolled_running_workers() {
    let mut fixture = Fixture::new();
    let scope = NativeOwnedWorkerScope::new();
    fixture.executor.worker_scope = Some(scope.clone());
    let approved = fixture.custom("printf forbidden > forbidden".into());
    let queued = bind(Arc::clone(&approved));
    let clone = Arc::clone(&approved);
    approved.revoke();
    assert!(clone.is_revoked());
    assert!(matches!(
        AuthorizedTerminalProbe::new(
            &owner(),
            request(&clone),
            clone,
            TerminalProbeClock {
                now_ms: 100,
                observed_at: Instant::now()
            }
        ),
        Err(TerminalProbeFailure::Denied)
    ));
    assert!(matches!(
        futures_executor::block_on(fixture.executor.execute(queued, CancellationToken::new()))
            .result,
        Err(TerminalProbeFailure::Denied)
    ));
    assert!(!fixture.root.join("forbidden").exists());

    let approved = fixture.custom("printf '%s' \"$$\" > leader; exec /bin/sleep 30".into());
    let mut future = fixture
        .executor
        .execute(bind(Arc::clone(&approved)), CancellationToken::new());
    futures_executor::block_on(async {
        assert!(futures_util::poll!(&mut future).is_pending());
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let pid = loop {
        if let Ok(text) = std::fs::read_to_string(fixture.root.join("leader"))
            && let Ok(pid) = text.parse::<i32>()
        {
            break rustix::process::Pid::from_raw(pid).unwrap();
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    scope.close();
    assert!(!scope.completion().is_complete());
    approved.revoke();
    // No more future polls: the worker observes the grant token itself.
    scope.completion().wait_on_worker().unwrap();
    assert_eq!(fixture.executor.active.load(Ordering::Acquire), 1);
    while rustix::process::test_kill_process(pid).is_ok() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(
        futures_executor::block_on(future).result,
        Err(TerminalProbeFailure::Denied)
    ));
    fixture.settled();
}

#[test]
fn probe_revocation_rejects_late_completed_evidence() {
    let fixture = Fixture::new();
    let approved = grant(TerminalProbeAuthority::Path {
        path: ".".into(),
        parent: directory(&fixture.root),
        leaf: "absent".into(),
    });
    let mut future = fixture
        .executor
        .execute(bind(Arc::clone(&approved)), CancellationToken::new());
    futures_executor::block_on(async {
        assert!(futures_util::poll!(&mut future).is_pending());
    });
    // The retained response owns a permit; do not consume the evidence until
    // after authority is retired, even if the worker already finished.
    std::thread::sleep(Duration::from_millis(100));
    approved.revoke();
    assert!(matches!(
        futures_executor::block_on(future).result,
        Err(TerminalProbeFailure::Denied)
    ));
    fixture.settled();
}

#[test]
fn probe_unpolled_cancelled_and_abandoned_calls_leave_no_process() {
    let fixture = Fixture::new();
    let grant = fixture.custom("printf '%s' \"$$\" > leader; exec /bin/sleep 30".into());
    drop(
        fixture
            .executor
            .execute(bind(Arc::clone(&grant)), CancellationToken::new()),
    );
    assert!(!fixture.root.join("leader").exists());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let evidence = futures_executor::block_on(
        fixture
            .executor
            .execute(bind(Arc::clone(&grant)), cancelled),
    );
    assert!(matches!(
        evidence.result,
        Err(TerminalProbeFailure::Unavailable)
    ));
    assert!(!fixture.root.join("leader").exists());
    for cancel in [false, true] {
        let cancellation = CancellationToken::new();
        let mut future = fixture
            .executor
            .execute(bind(Arc::clone(&grant)), cancellation.clone());
        futures_executor::block_on(async {
            assert!(futures_util::poll!(&mut future).is_pending());
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        let pid = loop {
            if let Ok(text) = std::fs::read_to_string(fixture.root.join("leader"))
                && let Ok(pid) = text.parse::<i32>()
            {
                break pid;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        if cancel {
            cancellation.cancel();
            assert!(matches!(
                futures_executor::block_on(future).result,
                Err(TerminalProbeFailure::Unavailable)
            ));
        } else {
            drop(future);
        }
        fixture.settled();
        assert_eq!(
            rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()),
            Err(rustix::io::Errno::SRCH)
        );
        std::fs::remove_file(fixture.root.join("leader")).unwrap();
    }
}
