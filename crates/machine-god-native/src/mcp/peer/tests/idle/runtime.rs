use super::*;

const TERMINAL_PREFIX: &str = r#"
IFS= read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":"end-gate","method":"roots/list","params":{}}'
IFS= read -r reply
case "$reply" in *'"id":"end-gate"'*) : ;; *) exit 4 ;; esac
"#;

#[test]
fn actual_idle_terminal_frames_and_eof_retire_without_retry() {
    for (ending, expected) in [
        (
            "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{}}'",
            McpPeerError::Correlation,
        ),
        (
            "printf '%s\\n' 'not-json'",
            McpPeerError::Transport(McpStdioError::Protocol),
        ),
        (
            "printf '%s' '{\"jsonrpc\":'",
            McpPeerError::Transport(McpStdioError::Protocol),
        ),
        ("", McpPeerError::Transport(McpStdioError::Closed)),
    ] {
        let fixture = Fixture::new();
        let script = format!("{TERMINAL_PREFIX}\n{ending}\n");
        let (peer, attempts) = fixture.connect(&[&script], Duration::from_secs(2));
        let mut peer = peer.unwrap();
        assert_eq!(attempts, 1);
        let error = fixture
            .runtime
            .block_on(peer.next_notification(Instant::now() + Duration::from_secs(5)))
            .unwrap_err();
        assert_eq!(error, expected);
        assert!(peer.closed);
        assert!(peer.pending_replies.is_empty());
        peer.completion().wait_on_worker().unwrap();
    }
}

#[test]
fn actual_partial_ndjson_survives_idle_drop_and_later_catalog() {
    let fixture = Fixture::new();
    let script = r#"
IFS= read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}}}'
printf '%s\n%s' '{"jsonrpc":"2.0","method":"notifications/ready"}' '{"jsonrpc":"2.0","method":"notifications/tools/'
IFS= read -r line
case "$line" in *'"method":"tools/list"'*) : ;; *) exit 4 ;; esac
printf '%s\n' 'list_changed"}' '{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}'
while IFS= read -r line; do :; done
"#;
    let (peer, attempts) = fixture.connect(&[script], Duration::from_secs(2));
    let mut peer = peer.unwrap();
    assert_eq!(attempts, 1);
    fixture.runtime.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(5);
        assert_eq!(
            peer.next_notification(deadline).await.unwrap().method(),
            Some("notifications/ready")
        );
        let mut observation = Box::pin(peer.next_notification(deadline));
        assert!(futures_util::poll!(&mut observation).is_pending());
        tokio::task::yield_now().await;
        assert!(futures_util::poll!(&mut observation).is_pending());
        drop(observation);
        assert!(peer.readiness().is_ready());
        let catalog = peer
            .catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                Instant::now(),
                deadline,
            )
            .await
            .unwrap();
        assert_eq!(catalog.items().count(), 0);
        assert_eq!(
            peer.take_notification().unwrap().method(),
            Some("notifications/tools/list_changed")
        );
        assert!(peer.take_notification().is_none());
        peer.close();
    });
    peer.completion().wait_on_worker().unwrap();
}

#[test]
fn actual_idle_unsupported_reply_preserves_exact_id_and_precedes_catalog_once() {
    let fixture = Fixture::new();
    let script = r#"
IFS= read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":"idle-exact","method":"roots/list","params":{}}'
IFS= read -r reply
case "$reply" in *'"id":"idle-exact"'*) : ;; *) exit 4 ;; esac
case "$reply" in *'"code":-32601'*) : ;; *) exit 5 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/replied"}'
IFS= read -r line
case "$line" in *'"method":"tools/list"'*) : ;; *) exit 6 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}'
while IFS= read -r line; do exit 7; done
"#;
    let (peer, attempts) = fixture.connect(&[script], Duration::from_secs(2));
    let mut peer = peer.unwrap();
    assert_eq!(attempts, 1);
    fixture.runtime.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(5);
        assert_eq!(
            peer.next_notification(deadline).await.unwrap().method(),
            Some("notifications/replied")
        );
        assert!(peer.pending_replies.is_empty());
        let catalog = peer
            .catalog(
                McpCatalogKind::Tools,
                McpCatalogLimits::default(),
                Instant::now(),
                deadline,
            )
            .await
            .unwrap();
        assert_eq!(catalog.items().count(), 0);
        assert!(peer.pending_replies.is_empty());
        peer.close();
    });
    peer.completion().wait_on_worker().unwrap();
}
