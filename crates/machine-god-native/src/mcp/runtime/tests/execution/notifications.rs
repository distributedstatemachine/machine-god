//! Real stdio producer regression for per-exchange notification consumption.
use super::*;
use crate::mcp::{
    peer::{McpPeerTimer, McpStdioPeer},
    protocol::WireLimits,
    stdio::McpStdioLaunch,
};
use std::fs::File;

struct Timer;
impl McpPeerTimer for Timer {
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move { tokio::time::sleep_until(deadline.into()).await })
    }
}

#[test]
fn two_calls_each_with_33_progress_notifications_complete() {
    let archive = Archive::new();
    let workers = NativeOwnedWorkerScope::new();
    let io = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let helper = PathBuf::from(
        std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY").expect("fresh helper required"),
    );
    let script = r#"
IFS= read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}}}'
id=2
while IFS= read -r line; do
    case "$line" in
    *tools/list*) printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","ttlMs":600000,"tools":[{"name":"lookup","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
    *tools/call*)
        n=0
        while [ "$n" -lt 33 ]; do
            printf '{"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":%s,"progress":%s,"total":33}}\n' "$id" "$n"
            n=$((n + 1))
        done
        printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete","content":[{"type":"text","text":"done"}]}}\n' "$id"
        ;;
    *) exit 5 ;;
    esac
    id=$((id + 1))
done
"#;
    let launch = McpStdioLaunch::new(
        helper.clone(),
        vec![crate::TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
        "/bin/sh".into(),
        vec!["-c".into(), script.into()],
        vec![],
        None,
        Arc::new(File::open(&archive.path).unwrap()),
        WireLimits::default(),
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    let launch = launch
        .with_process_inventory_service(
            helper,
            vec![crate::PROCESS_INVENTORY_SERVICE_ARGUMENT.into()],
        )
        .unwrap();
    let mut launch = Some(launch);
    let mut peer = io
        .block_on(McpStdioPeer::connect(
            &mut || {
                launch
                    .take()
                    .ok_or(crate::mcp::stdio::McpStdioError::Invalid)
            },
            workers.clone(),
            Arc::new(Timer),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
            Duration::from_secs(5),
        ))
        .unwrap();
    let epoch = Instant::now();
    let raw = io
        .block_on(peer.catalog(
            McpCatalogKind::Tools,
            McpCatalogLimits::default(),
            epoch,
            Instant::now() + Duration::from_secs(5),
        ))
        .unwrap();
    let catalog = McpDescriptorCatalog::admit(raw, McpDescriptorLimits::default()).unwrap();
    let fixture = Fixture::with_executor(
        &[json!({}), json!({})],
        PermissionMode::Auto,
        archive.executor.clone(),
        archive.executor.execution_policy(),
        false,
        move |runtime, _| {
            runtime
                .prepare_candidate(
                    vec![NativeMcpServerCandidate {
                        server: Arc::from("calendar"),
                        configuration: Arc::from(&b"config"[..]),
                        authentication: Arc::from(&b"auth"[..]),
                        catalogs: vec![catalog],
                        refresh: None,
                        catalog_epoch: epoch,
                        peer: NativeMcpOwnedPeer::Stdio(peer),
                        operation_timeout: Duration::from_secs(5),
                        authority_cancellations: Arc::from([]),
                    }],
                    &[MCP_SELECT_TOOL_NAME],
                )
                .unwrap()
        },
    );
    let events = io.block_on(async {
        fixture
            .conversation
            .enqueue("Use the selected tool twice".into())
            .unwrap();
        let turn = fixture.conversation.start_next(1).await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(15), turn.collect::<Vec<_>>())
            .await
            .unwrap()
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    });
    let outputs: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.payload {
            TurnEvent::ToolFinished { call_id, output }
                if call_id.as_str().starts_with("call-") =>
            {
                Some((call_id.clone(), output.clone()))
            }
            _ => None,
        })
        .collect();
    let publication = fixture.runtime.feature_publication().unwrap();
    io.block_on(async {
        assert!(
            publication.servers[0]
                .peer
                .lock()
                .await
                .take_notification()
                .is_none()
        );
    });
    drop(publication);
    fixture.runtime.close();
    io.block_on(fixture.runtime.drain_retired(
        Instant::now() + Duration::from_secs(5),
        CancellationToken::new(),
    ))
    .unwrap();
    drop(fixture);
    workers.close();
    workers.completion().wait_on_worker().unwrap();
    assert_eq!(outputs.len(), 2, "{outputs:?}");
    assert!(
        outputs.iter().all(|(_, output)| !output.is_error),
        "{outputs:?}"
    );
}
