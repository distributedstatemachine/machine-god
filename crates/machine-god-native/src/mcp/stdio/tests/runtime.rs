//! Process fixtures require the freshly built helper selected by the gate.
use super::*;
use crate::mcp::protocol::RpcId;
use futures_executor::block_on;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

struct Fixture {
    directory: PathBuf,
    host: NativeOwnedWorkerScope,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mg-mcp-stdio-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        Self {
            directory,
            host: NativeOwnedWorkerScope::new(),
        }
    }
    fn launch(&self, command: &str, arguments: Vec<String>) -> McpStdioLaunch {
        let helper = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY")
            .expect("stdio runtime fixtures require a fresh explicit helper");
        let launch = McpStdioLaunch::new(
            helper.into(),
            vec![crate::TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
            command.into(),
            arguments,
            vec![("MCP_SELECTED".into(), "selected".into())],
            Some("/bin:/usr/bin".into()),
            Arc::new(File::open(&self.directory).unwrap()),
            WireLimits::default(),
        )
        .unwrap();
        #[cfg(target_os = "macos")]
        let launch = launch
            .with_process_inventory_service(
                std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY")
                    .unwrap()
                    .into(),
                vec![crate::PROCESS_INVENTORY_SERVICE_ARGUMENT.into()],
            )
            .unwrap();
        launch
    }
    fn connect(&self, script: &str, keepalive: Box<dyn Send>) -> McpStdioConnection {
        block_on(self.launch("sh", vec!["-c".into(), script.into()]).connect(
            self.host.clone(),
            Instant::now() + Duration::from_secs(10),
            CancellationToken::new(),
            keepalive,
        ))
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.close();
        self.host.completion().wait_on_worker().unwrap();
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
struct Lease(Arc<AtomicUsize>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

#[test]
fn discovery_list_echo_and_close_retain_exact_owned_lifecycle() {
    let fixture = Fixture::new();
    let released = Arc::new(AtomicUsize::new(0));
    let connection = fixture.connect(r#"
test "$MCP_SELECTED" = selected || exit 7
i=0; while test "$i" -lt 1000; do printf 'ignored stderr bytes\n' >&2; i=$((i+1)); done
while IFS= read -r line; do
case "$line" in
  *server/discover*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2026-07-28","capabilities":{}}}' ;;
  *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}' ;;
  *tools/call*) printf '%s\n' '{"jsonrpc":"2.0","id":"rpc-secret","result":{"echo":true}}' ;;
  *notifications/initialized*) : ;;
  *) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"echo":true}}' ;;
esac
done
"#, Box::new(Lease(released.clone())));
    for (id, method) in [
        (1, "server/discover"),
        (2, "tools/list"),
        (3, "prompts/list"),
    ] {
        let bytes = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}"}}"#);
        let receipt = block_on(connection.control(
            McpStdioControl::discovery(bytes.as_bytes()).unwrap(),
            Instant::now() + Duration::from_secs(5),
        ))
        .unwrap();
        assert_eq!(receipt.outcome, Ok(()));
        assert_eq!(receipt.acknowledged_bytes, bytes.len() + 1);
        let envelope = block_on(connection.receive()).unwrap();
        assert_eq!(envelope.id(), Some(&RpcId::Integer(id)));
    }
    let permission = crate::mcp::submission::tests::Fixture::new();
    permission.ready("echo");
    let submission = block_on(permission.claim("echo", CancellationToken::new())).unwrap();
    connection
        .admit_runtimes(vec![permission.runtime.clone()])
        .unwrap();
    let receipt =
        block_on(connection.submit(submission, Instant::now() + Duration::from_secs(5))).unwrap();
    assert_eq!(receipt.outcome, Ok(()));
    assert_eq!(
        block_on(connection.receive()).unwrap().id(),
        Some(&RpcId::String("rpc-secret".into()))
    );
    assert_eq!(released.load(Ordering::Acquire), 0);
    let completion = connection.completion();
    connection.close();
    completion.wait_on_worker().unwrap();
    assert_eq!(released.load(Ordering::Acquire), 1);
    assert!(!fixture.host.completion().is_complete());
}

#[test]
fn missing_program_and_cancelled_startup_release_owned_admission() {
    let fixture = Fixture::new();
    let released = Arc::new(AtomicUsize::new(0));
    let result = block_on(
        fixture
            .launch("/definitely-missing-mcp-server", vec![])
            .connect(
                fixture.host.clone(),
                Instant::now() + Duration::from_secs(5),
                CancellationToken::new(),
                Box::new(Lease(released.clone())),
            ),
    );
    assert!(matches!(result, Err(McpStdioError::Process)));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let result = block_on(fixture.launch("sh", vec![]).connect(
        fixture.host.clone(),
        Instant::now() + Duration::from_secs(5),
        cancellation,
        Box::new(Lease(released.clone())),
    ));
    assert!(matches!(result, Err(McpStdioError::Cancelled)));
    fixture.host.close();
    fixture.host.completion().wait_on_worker().unwrap();
    assert_eq!(released.load(Ordering::Acquire), 2);
}

#[test]
fn incomplete_stdout_fails_protocol_and_collects_child() {
    let fixture = Fixture::new();
    let connection = fixture.connect("printf '{'; exit 0", Box::new(()));
    assert!(matches!(
        block_on(connection.receive()),
        Err(McpStdioError::Protocol)
    ));
    connection.completion().wait_on_worker().unwrap();
    let observation = connection.close_observation().unwrap();
    assert_eq!(observation.read_end, McpStdioReadEnd::IncompleteEof);
    assert!(observation.buffered_partial_frame);
}

#[test]
fn close_observation_requires_drain_validation_and_real_eof() {
    let fixture = Fixture::new();
    for (script, valid) in [
        (
            r#"printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'"#,
            true,
        ),
        ("printf 'not-json\\n'", false),
    ] {
        let connection = fixture.connect(script, Box::new(()));
        connection.completion().wait_on_worker().unwrap();
        let observation = connection.close_observation().unwrap();
        assert_eq!(observation.read_end, McpStdioReadEnd::CleanEof);
        assert_eq!(observation.unconsumed_complete_frames, 1);
        assert_eq!(block_on(connection.receive()).is_ok(), valid);
        let observation = connection.close_observation().unwrap();
        assert_eq!(observation.unconsumed_complete_frames, 0);
        assert_eq!(
            observation.reason,
            if valid {
                McpStdioError::Closed
            } else {
                McpStdioError::Protocol
            }
        );
    }
    let connection = fixture.connect("exit 0", Box::new(()));
    connection.completion().wait_on_worker().unwrap();
    let observation = connection.close_observation().unwrap();
    assert_eq!(observation.read_end, McpStdioReadEnd::CleanEof);
    assert_eq!(observation.unconsumed_complete_frames, 0);
}

#[test]
fn cancellation_during_startup_collects_helper_and_retained_lease() {
    stopped_startup_collects(false);
}

#[test]
fn abandonment_during_startup_collects_helper_and_retained_lease() {
    stopped_startup_collects(true);
}

fn stopped_startup_collects(abandon: bool) {
    let fixture = Fixture::new();
    let started = fixture.directory.join("started");
    let launch = fixture.launch("sh", vec![]).with_test_helper(
        crate::terminal_helper::TerminalPtyHelper::new(
            "/bin/sh".into(),
            vec![
                "-c".into(),
                format!(
                    "printf started > '{}'; exec /bin/sleep 30",
                    started.display()
                )
                .into(),
            ],
        )
        .unwrap()
        .with_test_inventory_helper(),
    );
    let cancellation = CancellationToken::new();
    let released = Arc::new(AtomicUsize::new(0));
    let mut future = launch.connect(
        fixture.host.clone(),
        Instant::now() + Duration::from_secs(10),
        cancellation.clone(),
        Box::new(Lease(released.clone())),
    );
    let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(future.as_mut().poll(&mut cx).is_pending());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() && Instant::now() < deadline {
        assert!(future.as_mut().poll(&mut cx).is_pending());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(started.exists());
    if abandon {
        drop(future);
    } else {
        cancellation.cancel();
        assert!(matches!(block_on(future), Err(McpStdioError::Cancelled)));
    }
    fixture.host.close();
    fixture.host.completion().wait_on_worker().unwrap();
    assert_eq!(released.load(Ordering::Acquire), 1);
}

#[test]
fn retained_renamed_cwd_resolves_relative_executable_and_exec_failure() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::new();
    let script = fixture.directory.join("server");
    std::fs::write(
        &script,
        b"#!/bin/sh\nprintf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{}}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let launch = fixture.launch("./server", vec![]);
    let renamed = fixture.directory.with_extension("renamed");
    std::fs::rename(&fixture.directory, &renamed).unwrap();
    fixture.directory = renamed;
    let connection = block_on(launch.connect(
        fixture.host.clone(),
        Instant::now() + Duration::from_secs(10),
        CancellationToken::new(),
        Box::new(()),
    ))
    .unwrap();
    assert_eq!(
        block_on(connection.receive()).unwrap().id(),
        Some(&RpcId::Integer(9))
    );
    connection.completion().wait_on_worker().unwrap();
    let bad = fixture.directory.join("bad");
    std::fs::write(&bad, b"#!/definitely-missing-interpreter\n").unwrap();
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = block_on(fixture.launch("./bad", vec![]).connect(
        fixture.host.clone(),
        Instant::now() + Duration::from_secs(10),
        CancellationToken::new(),
        Box::new(()),
    ));
    assert!(matches!(result, Err(McpStdioError::Process)));
}

#[test]
fn unpolled_connect_and_closed_host_never_launch() {
    let fixture = Fixture::new();
    let released = Arc::new(AtomicUsize::new(0));
    drop(fixture.launch("sh", vec![]).connect(
        fixture.host.clone(),
        Instant::now() + Duration::from_secs(5),
        CancellationToken::new(),
        Box::new(Lease(released.clone())),
    ));
    fixture.host.close();
    assert!(fixture.host.completion().is_complete());
    assert_eq!(released.load(Ordering::Acquire), 1);
    let result = block_on(fixture.launch("sh", vec![]).connect(
        fixture.host.clone(),
        Instant::now() + Duration::from_secs(5),
        CancellationToken::new(),
        Box::new(Lease(released.clone())),
    ));
    assert!(matches!(result, Err(McpStdioError::Capacity)));
    assert!(fixture.host.completion().is_complete());
    assert_eq!(released.load(Ordering::Acquire), 2);
}
