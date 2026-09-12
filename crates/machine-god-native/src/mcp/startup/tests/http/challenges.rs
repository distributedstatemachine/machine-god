use super::*;
use crate::mcp::http_peer::McpHttpAuthentication;

fn observed(startup: &NativeMcpStartup) -> Arc<NativeMcpStartupAuthChallenge> {
    startup.authentication_challenge("remote").unwrap().unwrap()
}
fn challenge(bytes: &[u8]) -> McpHttpAuthentication {
    McpHttpAuthentication {
        status: 401,
        challenges: vec![bytes.into()].into(),
    }
}
async fn unauthorized(listener: &TcpListener) {
    let (mut socket, _) = listener.accept().await.unwrap();
    request(&mut socket).await;
    socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer resource_metadata=\"https://example.test/meta\", scope=\"read\"\r\nWWW-Authenticate: Invalid syntax is still observed\r\nContent-Length: 0\r\n\r\n").await.unwrap();
    socket.flush().await.unwrap();
}

#[test]
fn exact_http_challenges_survive_failed_startup_and_attempt_cleanup() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let startup = NativeMcpStartup::new(http_options(listener.local_addr().unwrap())).unwrap();
        assert!(
            startup
                .authentication_challenge("remote")
                .unwrap()
                .is_none()
        );
        let (batch, ()) = join(
            startup.build(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            unauthorized(&listener),
        )
        .await;
        assert_eq!(
            batch.receipt().servers[0].state,
            NativeMcpStartupState::Failed(NativeMcpStartupError::Authentication)
        );
        assert!(batch.receipt().cleanup_complete());
        drop(batch);
        let receipt = observed(&startup);
        assert_eq!(receipt.server(), "remote");
        assert_eq!(receipt.status(), 401);
        assert_eq!(
            receipt.challenges()[0].as_ref(),
            b"Bearer resource_metadata=\"https://example.test/meta\", scope=\"read\""
        );
        assert_eq!(
            receipt.challenges()[1].as_ref(),
            b"Invalid syntax is still observed"
        );
        receipt.revalidate(&startup).unwrap();
        assert!(!format!("{receipt:?}").contains("example.test"));
        assert!(!format!("{receipt:?}").contains("remote"));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn catalog_auth_failure_retains_headers_without_changing_catalog_error() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let startup = NativeMcpStartup::new(http_options(listener.local_addr().unwrap())).unwrap();
        let server = async {
            reply(&listener, &discover(true)).await;
            unauthorized(&listener).await;
        };
        let (batch, ()) = join(
            startup.build(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            server,
        )
        .await;
        assert_eq!(
            batch.receipt().servers[0].state,
            NativeMcpStartupState::Failed(NativeMcpStartupError::Catalog)
        );
        assert!(batch.receipt().cleanup_complete());
        observed(&startup).revalidate(&startup).unwrap();
    });
}

#[test]
fn source_replacement_and_real_lifetimes_reject_stale_observations() {
    let address = "127.0.0.1:34567".parse().unwrap();
    let startup = NativeMcpStartup::new(http_options(address)).unwrap();
    let foreign = NativeMcpStartup::new(http_options(address)).unwrap();
    startup.capture_challenge("remote", challenge(b"first"), None, usize::MAX);
    let first = observed(&startup);
    assert_eq!(
        first.revalidate(&foreign),
        Err(NativeMcpStartupError::Invalid)
    );
    startup.capture_challenge("remote", challenge(b"second"), None, usize::MAX);
    assert_eq!(
        first.revalidate(&startup),
        Err(NativeMcpStartupError::Unavailable)
    );
    assert_eq!(first.challenges()[0].as_ref(), b"first");
    for kind in 0..4 {
        let selected = http_options(address);
        let auth = CancellationToken::new();
        let token = match kind {
            0 => selected.owner_cancellation.clone(),
            1 => selected.configuration_cancellation.clone(),
            2 => selected.network.as_ref().unwrap().owner_cancellation(),
            _ => auth.clone(),
        };
        let startup = NativeMcpStartup::new(selected).unwrap();
        startup.capture_challenge("remote", challenge(b"history"), Some(&auth), usize::MAX);
        let receipt = observed(&startup);
        receipt.revalidate(&startup).unwrap();
        token.cancel();
        assert_eq!(
            receipt.revalidate(&startup),
            Err(NativeMcpStartupError::Cancelled)
        );
        assert_eq!(receipt.challenges()[0].as_ref(), b"history");
    }
    assert!(matches!(
        startup.authentication_challenge("absent"),
        Err(NativeMcpStartupError::Invalid)
    ));
}

#[test]
fn replaced_external_owners_remain_charged_and_failed_capture_invalidates_latest() {
    let startup = NativeMcpStartup::new(http_options("127.0.0.1:34567".parse().unwrap())).unwrap();
    let mut retained = Vec::new();
    for _ in 0..2 * crate::mcp::config::MAX_SERVERS {
        startup.capture_challenge("remote", challenge(b"historical"), None, usize::MAX);
        retained.push(observed(&startup));
    }
    let charge = startup.challenge_charge();
    startup.capture_challenge("remote", challenge(b"exhausted"), None, usize::MAX);
    assert!(matches!(
        startup.authentication_challenge("remote"),
        Err(NativeMcpStartupError::Limit)
    ));
    assert_eq!(startup.challenge_charge(), charge);
    assert!(retained.last().unwrap().revalidate(&startup).is_err());
    drop(retained);
    assert_eq!(startup.challenge_charge(), 0);
    startup.capture_challenge("remote", challenge(b"fresh"), None, usize::MAX);
    observed(&startup).revalidate(&startup).unwrap();
    startup.capture_challenge("remote", challenge(b"over budget"), None, 1);
    assert!(matches!(
        startup.authentication_challenge("remote"),
        Err(NativeMcpStartupError::Limit)
    ));
    assert_eq!(startup.challenge_charge(), 0);
    startup.capture_challenge(
        "remote",
        challenge(&vec![b'x'; 16 * 1024 + 1]),
        None,
        usize::MAX,
    );
    assert!(matches!(
        startup.authentication_challenge("remote"),
        Err(NativeMcpStartupError::Limit)
    ));
}

#[test]
fn unpolled_and_precancelled_builds_do_not_replace_observations() {
    let startup = NativeMcpStartup::new(http_options("127.0.0.1:34567".parse().unwrap())).unwrap();
    startup.capture_challenge("remote", challenge(b"old"), None, usize::MAX);
    let old = observed(&startup);
    drop(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    old.revalidate(&startup).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let batch = block_on(startup.build(NativeMcpStartupPhase::All, cancellation, deadline()));
    assert_eq!(
        batch.receipt().failure,
        Some(NativeMcpStartupError::Cancelled)
    );
    old.revalidate(&startup).unwrap();
    startup.clear_challenge("remote");
    assert!(old.revalidate(&startup).is_err());
    assert!(
        startup
            .authentication_challenge("remote")
            .unwrap()
            .is_none()
    );
    assert_eq!(old.challenges()[0].as_ref(), b"old");
}

#[test]
fn byte_and_header_limits_never_reuse_an_older_observation() {
    let mut selected = http_options("127.0.0.1:34567".parse().unwrap());
    selected.max_retained_bytes = 1024;
    let startup = NativeMcpStartup::new(selected).unwrap();
    startup.capture_challenge("remote", challenge(b"first"), None, usize::MAX);
    let first = observed(&startup);
    startup.capture_challenge("remote", challenge(b"second"), None, usize::MAX);
    assert!(matches!(
        startup.authentication_challenge("remote"),
        Err(NativeMcpStartupError::Limit)
    ));
    assert!(first.revalidate(&startup).is_err());
    assert!(startup.challenge_charge() <= 1024);
    drop(first);
    startup.capture_challenge("remote", challenge(b"fresh"), None, usize::MAX);
    observed(&startup).revalidate(&startup).unwrap();
    startup.capture_challenge(
        "remote",
        McpHttpAuthentication {
            status: 401,
            challenges: vec![Box::<[u8]>::from(b"a".as_slice()); 9].into(),
        },
        None,
        usize::MAX,
    );
    assert!(matches!(
        startup.authentication_challenge("remote"),
        Err(NativeMcpStartupError::Limit)
    ));
    assert_eq!(startup.challenge_charge(), 0);
}

#[test]
fn actual_failed_capture_keeps_original_auth_error_and_reports_retention_limit() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = http_options(listener.local_addr().unwrap());
        selected.max_retained_bytes = 8192;
        let startup = NativeMcpStartup::new(selected).unwrap();
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            socket.write_all(format!("HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: {}\r\nContent-Length: 0\r\n\r\n", "x".repeat(10 * 1024)).as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
        };
        let (batch, ()) = join(
            startup.build(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            server,
        )
        .await;
        assert_eq!(
            batch.receipt().servers[0].state,
            NativeMcpStartupState::Failed(NativeMcpStartupError::Authentication)
        );
        assert!(matches!(
            startup.authentication_challenge("remote"),
            Err(NativeMcpStartupError::Limit)
        ));
        assert!(batch.receipt().cleanup_complete());
        assert_eq!(startup.challenge_charge(), 0);
    });
}
