use super::*;

mod idle;

#[test]
fn required_readiness_observes_only_selected_stdio_lifetime_and_connection() {
    struct FixedTimer(Instant);
    impl McpPeerTimer for FixedTimer {
        fn now(&self) -> Instant {
            self.0
        }
        fn sleep_until(&self, _: Instant) -> BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }
    let now = Instant::now();
    let connection = McpStdioConnection::inert_for_test();
    let cancellation = CancellationToken::new();
    let mut observation = McpStdioPeerReadiness {
        connection: connection.readiness(),
        cancellation: cancellation.clone(),
        lifetime: McpPeerLifetime::Until(now + Duration::from_secs(1)),
        timer: Arc::new(FixedTimer(now)),
    };
    assert!(observation.is_ready());
    observation.timer = Arc::new(FixedTimer(now + Duration::from_secs(1)));
    assert!(!observation.is_ready());
    observation.lifetime = McpPeerLifetime::OwnerControlled;
    assert!(observation.is_ready());
    cancellation.cancel();
    assert!(!observation.is_ready());
}
use crate::mcp::protocol::{ProtocolVersion, WireLimits, parse_envelope};
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Timer;
impl McpPeerTimer for Timer {
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move { tokio::time::sleep_until(deadline.into()).await })
    }
}
struct Fixture {
    directory: PathBuf,
    host: NativeOwnedWorkerScope,
    runtime: tokio::runtime::Runtime,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mg-mcp-peer-{}-{}-{}",
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
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap(),
        }
    }
    fn launch(&self, script: &str) -> McpStdioLaunch {
        let helper = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY")
            .expect("fresh explicit helper required");
        let launch = McpStdioLaunch::new(
            helper.into(),
            vec![crate::TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
            "/bin/sh".into(),
            vec!["-c".into(), script.into()],
            vec![],
            None,
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
    fn connect(&self, scripts: &[&str], timeout: Duration) -> (Result<McpStdioPeer>, usize) {
        let mut launches = scripts
            .iter()
            .map(|script| self.launch(script))
            .collect::<VecDeque<_>>();
        let mut attempts = 0;
        let mut factory = || {
            attempts += 1;
            launches.pop_front().ok_or(McpStdioError::Invalid)
        };
        let result = self.runtime.block_on(McpStdioPeer::connect(
            &mut factory,
            self.host.clone(),
            Arc::new(Timer),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(20),
            timeout,
        ));
        (result, attempts)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.close();
        self.host.completion().wait_on_worker().unwrap();
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

const MODERN: &str = r#"
IFS= read -r line
case "$line" in *server/discover*) : ;; *) exit 4 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{},"resources":{},"prompts":{}}}}'
while IFS= read -r line; do :; done
"#;
const LEGACY: &str = r#"
IFS= read -r line
case "$line" in *initialize*) : ;; *) exit 4 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"protocolVersion":"2024-11-05","capabilities":{}}}'
IFS= read -r line
case "$line" in *notifications/initialized*) : ;; *) exit 5 ;; esac
while IFS= read -r line; do :; done
"#;

#[test]
fn actual_typed_feature_preserves_raw_result_and_honors_selected_stdio_bounds() {
    use crate::mcp::control::{
        McpFeatureOperationOptions, McpFeatureReply,
        tests::{catalogs, human, request},
    };
    let fixture = Fixture::new();
    let script = r#"
IFS= read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"resources":{}}}}'
IFS= read -r line
case "$line" in *resources/read*) : ;; *) exit 4 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","contents":[{"uri":"test://fixed","text":"hello"}],"raw":123456789012345678901234567890}}'
while IFS= read -r line; do :; done
"#;
    let (peer, _) = fixture.connect(&[script], Duration::from_secs(2));
    let mut peer = peer.unwrap();
    let request = request("resource read srv test://fixed");
    let catalogs = catalogs();
    let until = Instant::now() + Duration::from_secs(5);
    let full = fixture.runtime.block_on(peer.feature(
        &request,
        "srv",
        &catalogs,
        human(CancellationToken::new()),
        McpFeatureOperationOptions::new(Instant::now()),
        until,
    ));
    assert!(matches!(full, Err(McpPeerError::Capacity)));
    assert_eq!(peer.next_id, Some(2));
    let mut options = McpFeatureOperationOptions::new(Instant::now());
    options.codec.max_response_bytes = WireLimits::default().max_frame_bytes;
    options.codec.max_nodes = WireLimits::default().max_nodes;
    let result = fixture
        .runtime
        .block_on(peer.feature(
            &request,
            "srv",
            &catalogs,
            human(CancellationToken::new()),
            options,
            until,
        ))
        .unwrap();
    let McpFeatureReply::Response(result) = result else {
        panic!()
    };
    assert!(
        result
            .raw_json()
            .get()
            .contains("123456789012345678901234567890")
    );
    peer.close();
    peer.completion().wait_on_worker().unwrap();
}

#[test]
fn actual_modern_negotiation_and_lossless_catalog_route_unsolicited_messages() {
    let fixture = Fixture::new();
    let script = r#"
IFS= read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{"listChanged":true}}}}'
IFS= read -r line
printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}' '{"jsonrpc":"2.0","id":"server-id","method":"sampling/createMessage","params":{}}' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"first","inputSchema":{"const":9007199254740993.0}}],"nextCursor":""}}'
IFS= read -r reply
case "$reply" in *-32601*) : ;; *) exit 6 ;; esac
IFS= read -r line
case "$line" in *'"cursor":""'*) : ;; *) exit 7 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"second","inputSchema":{}}]}}'
while IFS= read -r line; do :; done
"#;
    let (peer, attempts) = fixture.connect(&[script], Duration::from_secs(2));
    let mut peer = peer.unwrap();
    assert_eq!(attempts, 1);
    assert_eq!(peer.protocol().version, ProtocolVersion::Modern);
    assert!(peer.capabilities().tools_list_changed());
    let catalog = fixture
        .runtime
        .block_on(peer.catalog(
            McpCatalogKind::Tools,
            McpCatalogLimits::default(),
            Instant::now(),
            Instant::now() + Duration::from_secs(5),
        ))
        .unwrap();
    let items = catalog.items().collect::<Vec<_>>();
    assert_eq!(items.len(), 2);
    assert!(items[0].1.get().contains("9007199254740993.0"));
    assert_eq!(
        peer.take_notification().unwrap().method(),
        Some("notifications/tools/list_changed")
    );
    assert!(peer.take_notification().is_none());
    let id = peer.reserve_tool_id().unwrap();
    assert_eq!(id, RpcId::Integer(4));
    assert_eq!(peer.reserve_tool_id(), Err(McpPeerError::Capacity));
    peer.discard_tool_id();
    assert_eq!(peer.reserve_tool_id().unwrap(), RpcId::Integer(5));
    peer.discard_tool_id();
    let lease = peer.reserve_tool().unwrap();
    assert_eq!(lease.rpc_id(), &RpcId::Integer(6));
    assert!(peer.reserve_tool_id().is_err());
    drop(lease);
    let lease = peer.reserve_tool().unwrap();
    assert_eq!(lease.rpc_id(), &RpcId::Integer(7));
    peer.close();
    drop(lease);
    assert!(peer.reserve_tool().is_err());
}

#[test]
fn ordinary_discovery_error_restarts_only_after_old_child_settles() {
    let fixture = Fixture::new();
    let first = r#"IFS= read -r line; printf '%s\n' '{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no"}}'; while IFS= read -r line; do :; done"#;
    let (peer, attempts) = fixture.connect(&[first, LEGACY], Duration::from_secs(2));
    let mut peer = peer.unwrap();
    assert_eq!(attempts, 2);
    assert_eq!(peer.protocol().version, ProtocolVersion::Legacy20241105);
    peer.close();
}

#[test]
fn observed_fallback_retains_each_completion_before_launch_and_refreshes_only_attempt() {
    let fixture = Fixture::new();
    let first = r#"IFS= read -r line; printf '%s\n' '{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no"}}'; while IFS= read -r line; do :; done"#;
    for allow_fallback in [false, true] {
        let mut launches = VecDeque::from([fixture.launch(first), fixture.launch(LEGACY)]);
        let observed = Arc::new(std::sync::Mutex::new(
            Vec::<NativeOwnedWorkerCompletion>::new(),
        ));
        let capture = observed.clone();
        let admit = Arc::new(move |completion: NativeOwnedWorkerCompletion| {
            let mut values = capture.lock().unwrap();
            assert!(values.iter().all(NativeOwnedWorkerCompletion::is_complete));
            let admitted = values.is_empty() || allow_fallback;
            values.push(completion);
            admitted
        });
        let start = Instant::now();
        let timeout = Duration::from_secs(2);
        let result = fixture.runtime.block_on(McpStdioPeer::connect_observed(
            &mut || launches.pop_front().ok_or(McpStdioError::Invalid),
            fixture.host.clone(),
            Arc::new(Timer),
            CancellationToken::new(),
            start + Duration::from_secs(20),
            timeout,
            admit,
        ));
        if allow_fallback {
            let (mut peer, attempt) = result.unwrap();
            assert!(attempt > start + timeout);
            assert!(attempt <= Instant::now() + timeout);
            peer.close();
            let completion = peer.completion();
            completion.wait_on_worker().unwrap();
        } else {
            assert!(matches!(
                result,
                Err(McpPeerError::Transport(McpStdioError::Capacity))
            ));
        }
        let values = observed.lock().unwrap();
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(NativeOwnedWorkerCompletion::is_complete));
    }
}

#[test]
fn discovery_timeout_snapshot_restarts_but_partial_or_malformed_output_does_not() {
    let fixture = Fixture::new();
    let silent = "while IFS= read -r line; do :; done";
    let (peer, attempts) = fixture.connect(&[silent, LEGACY], Duration::from_millis(100));
    let mut peer = peer.unwrap();
    assert_eq!(attempts, 2);
    assert_eq!(peer.protocol().version, ProtocolVersion::Legacy20241105);
    peer.close();
    drop(peer);
    for script in [
        "IFS= read -r line; printf '{'; while IFS= read -r line; do :; done",
        "IFS= read -r line; printf 'invalid\\n'; while IFS= read -r line; do :; done",
    ] {
        let (result, attempts) = fixture.connect(&[script, LEGACY], Duration::from_millis(100));
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }
}

#[test]
fn malformed_success_and_foreign_response_never_authorize_restart() {
    let fixture = Fixture::new();
    for response in [
        r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{"listChanged":"yes"}}}}"#,
        r#"{"jsonrpc":"2.0","id":99,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{}}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"input_required","supportedVersions":["2026-07-28"],"capabilities":{}}}"#,
    ] {
        let script = format!(
            "IFS= read -r line; printf '%s\\n' '{response}'; while IFS= read -r line; do :; done"
        );
        let (result, attempts) = fixture.connect(&[&script, LEGACY], Duration::from_secs(2));
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }
}

#[test]
fn startup_future_and_cancelled_startup_do_not_call_launch_factory() {
    let fixture = Fixture::new();
    let cancellation = CancellationToken::new();
    let mut count = 0;
    let mut factory = || {
        count += 1;
        Ok(fixture.launch(MODERN))
    };
    drop(McpStdioPeer::connect(
        &mut factory,
        fixture.host.clone(),
        Arc::new(Timer),
        cancellation.clone(),
        Instant::now() + Duration::from_secs(5),
        Duration::from_secs(1),
    ));
    cancellation.cancel();
    assert!(
        fixture
            .runtime
            .block_on(McpStdioPeer::connect(
                &mut factory,
                fixture.host.clone(),
                Arc::new(Timer),
                cancellation,
                Instant::now() + Duration::from_secs(5),
                Duration::from_secs(1)
            ))
            .is_err()
    );
    assert_eq!(count, 0);
}

#[test]
fn capability_shapes_follow_pinned_optional_legacy_flags() {
    for (result, valid) in [
        (json!({"protocolVersion":"2024-11-05"}), true),
        (
            json!({"capabilities":{"resources":{"subscribe":true},"completions":{}}}),
            true,
        ),
        (
            json!({"capabilities":{"resources":{"subscribe":null}}}),
            false,
        ),
        (json!({"capabilities":{"prompts":false}}), false),
        (json!({"capabilities":null}), false),
    ] {
        let bytes = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"result":result})).unwrap();
        let response = parse_envelope(&bytes, WireLimits::default()).unwrap();
        assert_eq!(
            McpPeerCapabilities::admit(&response, ProtocolVersion::Legacy20241105).is_ok(),
            valid
        );
    }
}

use serde_json::json;
