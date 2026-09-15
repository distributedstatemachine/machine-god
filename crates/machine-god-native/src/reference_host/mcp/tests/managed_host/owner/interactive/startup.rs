use super::*;
use crate::mcp::{
    http::tests::request,
    network::{McpResolverConfig, NativeMcpNetwork},
};
use futures_util::future::{Either, select};
use std::{net::Ipv4Addr, os::unix::fs::PermissionsExt};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};

fn seed(profile: &std::path::Path, config: &str) {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(profile)
        .unwrap();
    let path = profile.join("mcp.json");
    fs::write(&path, config).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

async fn pending_startup(
    owner: &mut NativeInteractiveSession,
    listener: &TcpListener,
) -> TcpStream {
    let progress = Box::pin(poll_fn(|cx| {
        let progress = owner.poll_progress(cx, 12);
        assert!(owner.managed_error().is_none());
        assert!(
            owner.take_outcome().is_none(),
            "candidate must await discovery"
        );
        if progress.is_ready() {
            cx.waker().wake_by_ref();
        }
        Poll::<()>::Pending
    }));
    let server = Box::pin(async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let received = request(&mut socket).await;
        assert!(String::from_utf8_lossy(&received).contains("server/discover"));
        socket
    });
    match select(progress, server).await {
        Either::Right((socket, observer)) => {
            drop(observer);
            socket
        }
        Either::Left(_) => panic!("startup progress cannot finish without a receipt"),
    }
}

fn network_fixture() -> (Fixture, PathBuf) {
    let mut profile = None;
    let fixture = Fixture::with_options("auto", true, |options, directory, clock| {
        profile = Some(directory.0.join("profile"));
        let mut options = configured_options(options, directory, clock.clone());
        options
            .mcp_runtime
            .as_mut()
            .unwrap()
            .startup
            .as_mut()
            .unwrap()
            .network = Some(Arc::new(
            NativeMcpNetwork::new(
                McpResolverConfig::literal_only(),
                [9; 32],
                None,
                clock,
                CancellationToken::new(),
                2,
            )
            .unwrap(),
        ));
        options
    });
    (fixture, profile.unwrap())
}

#[test]
fn preselection_shutdown_cancels_discovery_and_settles_original_owner() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (mut fixture, profile) = network_fixture();
        let (mut startup, host) = super::preselection::prepare(&mut fixture).await;
        let completion = host.terminal_shutdown_completion().unwrap();
        seed(
            &profile,
            &format!(
                r#"{{"mcp":{{"pending":{{"type":"http","url":"http://127.0.0.1:{}/mcp","startup_timeout_ms":30000}}}}}}"#,
                listener.local_addr().unwrap().port(),
            ),
        );
        startup
            .request_open(NativeInteractiveInitialSession::Fresh, 1)
            .unwrap();
        let progress = Box::pin(poll_fn(|cx| {
            assert!(startup.poll_open(cx, 2).is_pending());
            Poll::<()>::Pending
        }));
        let server = Box::pin(async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let received = request(&mut socket).await;
            assert!(String::from_utf8_lossy(&received).contains("server/discover"));
            socket
        });
        let mut socket = match select(progress, server).await {
            Either::Right((socket, observer)) => {
                drop(observer);
                socket
            }
            Either::Left(_) => panic!("discovery requires the server response"),
        };
        startup.request_shutdown();
        let (closed, eof) =
            futures_util::future::join(poll_fn(|cx| startup.poll_open(cx, 3)), async {
                socket.read(&mut [0u8; 1]).await.unwrap()
            })
            .await;
        assert!(closed.unwrap().is_none());
        assert_eq!(eof, 0);
        drop(startup);
        drop(host);
        completion.wait().await;
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
    });
}

async fn interrupted_startup(shutdown: bool) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (mut fixture, profile) = network_fixture();
    let path = journal_path(&fixture);
    let host = fixture.host.take().unwrap();
    let completion = host.terminal_shutdown_completion().unwrap();
    let options = NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        host.loaded_config().config().model_preferences(),
    )
    .unwrap();
    let mut owner = NativeInteractiveSession::open_managed(
        host,
        directory(&path),
        options,
        NativeInteractiveInitialSession::Fresh,
        1,
    )
    .await
    .unwrap();
    let source = owner.runtime().id();
    seed(
        &profile,
        &format!(
            r#"{{"mcp":{{"pending":{{"type":"http","url":"http://127.0.0.1:{}/mcp","startup_timeout_ms":30000}}}}}}"#,
            listener.local_addr().unwrap().port()
        ),
    );
    let first = owner
        .request_transition(NativeInteractiveTransition::New, 11)
        .unwrap();
    let mut socket = pending_startup(&mut owner, &listener).await;
    let replacement = if shutdown {
        owner.request_shutdown();
        None
    } else {
        seed(&profile, r#"{"mcp":{}}"#);
        Some(
            owner
                .request_transition(NativeInteractiveTransition::New, 13)
                .unwrap(),
        )
    };
    let NativeInteractiveOutcome::Superseded { request, .. } = outcome(&mut owner).await else {
        panic!("original candidate must settle before supersession");
    };
    assert_eq!(request, first.id);
    assert_eq!(owner.runtime().id(), source);
    if let Some(replacement) = replacement {
        let NativeInteractiveOutcome::Transition(receipt) = outcome(&mut owner).await else {
            panic!("replacement must get a fresh uncancelled startup");
        };
        assert_eq!(receipt.request, replacement.id);
        assert_ne!(owner.runtime().id(), source);
        owner.request_shutdown();
    }
    assert!(matches!(
        outcome(&mut owner).await,
        NativeInteractiveOutcome::Shutdown
    ));
    assert!(owner.is_closed());
    let mut byte = [0];
    assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
    drop(owner);
    completion.wait().await;
    assert!(fixture.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn shutdown_cancels_pending_candidate_discovery_and_settles_original_socket() {
    run(interrupted_startup(true));
}

#[test]
fn supersession_cancels_only_original_candidate_and_replacement_startup_succeeds() {
    run(interrupted_startup(false));
}

#[test]
fn initial_configured_http_peer_does_not_pin_startup_admission_until_shutdown() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (mut fixture, profile) = network_fixture();
        seed(
            &profile,
            &format!(
                r#"{{"mcp":{{"ready":{{"type":"http","url":"http://127.0.0.1:{}/mcp","required":true}}}}}}"#,
                listener.local_addr().unwrap().port()
            ),
        );
        let path = journal_path(&fixture);
        let host = fixture.host.take().unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            host.loaded_config().config().model_preferences(),
        )
        .unwrap();
        let startup = NativeInteractiveSession::open_managed(
            host,
            directory(&path),
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        );
        let response = crate::reference_host::mcp::tests::http::reply(&listener,
            br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{}}}"#);
        let (result, request) = futures_util::future::join(startup, response).await;
        assert!(String::from_utf8_lossy(&request).contains("server/discover"));
        let mut owner = result.unwrap();
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        owner.enqueue("use the ready parent".into()).unwrap();
        match outcome(&mut owner).await {
            NativeInteractiveOutcome::Turn(Ok(_)) => {}
            NativeInteractiveOutcome::Turn(Err(error)) => panic!("ready parent: {error:?}"),
            _ => panic!("parent turn outcome"),
        }
        owner.request_shutdown();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Shutdown
        ));
        drop(owner);
        completion.wait().await;
    });
}

#[test]
fn failed_initial_http_discovery_is_settled_without_disabling_management() {
    failed_initial_http_discovery(false);
}

#[test]
fn required_only_initial_http_discovery_fails_selection_after_cleanup() {
    failed_initial_http_discovery(true);
}

fn failed_initial_http_discovery(required_only: bool) {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (mut fixture, profile) = network_fixture();
        seed(
            &profile,
            &format!(
                r#"{{"mcp":{{"failed":{{"type":"http","url":"http://127.0.0.1:{}/mcp","required":true}}}}}}"#,
                listener.local_addr().unwrap().port()
            ),
        );
        let path = journal_path(&fixture);
        let host = fixture.host.take().unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            host.loaded_config().config().model_preferences(),
        )
        .unwrap();
        let options = if required_only {
            options.with_required_mcp_startup()
        } else {
            options
        };
        let opening = NativeInteractiveSession::open_managed(
            host,
            directory(&path),
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        );
        let response = crate::reference_host::mcp::tests::http::reply(
            &listener,
            b"invalid discovery response",
        );
        let (result, _) = futures_util::future::join(opening, response).await;
        if required_only {
            assert!(
                result.is_err(),
                "one-shot selection must not retain a repair UI"
            );
            drop(result);
            completion.wait().await;
            assert!(fixture.transport.requests.lock().unwrap().is_empty());
            return;
        }
        let mut owner = result.unwrap();
        assert!(owner.mcp_startup_failure().is_some());
        seed(&profile, r#"{"mcp":{}}"#);
        owner
            .request_control(
                NativeInteractiveControl::Mcp {
                    command: crate::mcp::commands::McpCommand::Reload,
                },
                2,
            )
            .unwrap();
        let reloaded = poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 3);
            owner
                .take_control_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        })
        .await;
        assert!(reloaded.result.is_ok(), "{reloaded:?}");
        assert!(owner.mcp_startup_failure().is_none());
        owner.request_shutdown();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Shutdown
        ));
        drop(owner);
        completion.wait().await;
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
    });
}

#[test]
fn required_only_initial_startup_does_not_connect_optional_peers() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (mut fixture, profile) = network_fixture();
        seed(
            &profile,
            &format!(
                r#"{{"mcp":{{"optional":{{"type":"http","url":"http://127.0.0.1:{}/mcp","required":false}}}}}}"#,
                listener.local_addr().unwrap().port()
            ),
        );
        let path = journal_path(&fixture);
        let host = fixture.host.take().unwrap();
        let completion = host.terminal_shutdown_completion().unwrap();
        let options = NativeInteractiveSessionOptions::new(
            fixture.workspace.clone(),
            host.loaded_config().config().model_preferences(),
        )
        .unwrap()
        .with_required_mcp_startup();
        let mut owner = NativeInteractiveSession::open_managed(
            host,
            directory(&path),
            options,
            NativeInteractiveInitialSession::Fresh,
            1,
        )
        .await
        .unwrap();
        assert!(owner.mcp_startup_failure().is_none());
        owner.enqueue("required-only startup".into()).unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Turn(Ok(_))
        ));
        owner.request_shutdown();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeInteractiveOutcome::Shutdown
        ));
        drop(owner);
        completion.wait().await;
        poll_fn(|cx| {
            assert!(
                listener.poll_accept(cx).is_pending(),
                "optional startup opened a socket"
            );
            Poll::Ready(())
        })
        .await;
        assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1);
    });
}
