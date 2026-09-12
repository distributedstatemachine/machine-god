use super::*;
use crate::mcp::{
    auth::{McpAuthConfig, McpAuthError},
    clock::TokioMcpClock,
    config::{McpConfig, McpTransportConfig},
    controller::NativeMcpControllerStartupOptions,
    headers::McpResolvedHeaders,
    lifetime::McpPeerLifetime,
    management::NativeMcpManagementService,
    network::{McpResolverConfig, NativeMcpNetwork},
    startup::NativeMcpStartupPhase,
    store::NativeMcpConfigStore,
};
use futures_util::future::join;
use serde_json::json;
use std::{net::Ipv4Addr, os::unix::fs::PermissionsExt};
use tokio::net::TcpListener;

fn fixture(network: bool) -> (Fixture, PathBuf) {
    let mut profile = PathBuf::new();
    let fixture = Fixture::with_options("ask", true, |mut options, directory, _| {
        profile = directory.0.join("selected-profile");
        let contexts = options.mcp_runtime.take().unwrap().contexts;
        let clock = Arc::new(TokioMcpClock);
        let owner = CancellationToken::new();
        let selected_network = network.then(|| {
            Arc::new(
                NativeMcpNetwork::new(
                    McpResolverConfig::literal_only(),
                    [3; 32],
                    None,
                    clock.clone(),
                    owner.clone(),
                    4,
                )
                .unwrap(),
            )
        });
        let mut mcp = NativeReferenceHostMcpOptions::new(contexts, clock.clone())
            .with_controller_startup(NativeMcpControllerStartupOptions {
                captured_environment: vec![],
                stdio: None,
                clock: clock.clone(),
                catalog_epoch: Instant::now(),
                owner_cancellation: owner,
                network: selected_network,
                authentication: vec![],
                peer_lifetime: McpPeerLifetime::OwnerControlled,
                max_retained_bytes: 1024 * 1024,
            });
        mcp.authentication = Some(super::super::authentication::Options::captured(clock));
        options.with_mcp_runtime(mcp).with_mcp_management(Arc::new(
            NativeMcpManagementService::new(Arc::new(
                NativeMcpConfigStore::new(profile.clone()).unwrap(),
            )),
        ))
    });
    (fixture, profile)
}

fn auth_config(endpoint: &str) -> McpAuthConfig {
    let config = McpConfig::decode(
        &serde_json::to_vec(&json!({"mcp": {
            "remote": {"type":"http", "url":endpoint}
        }}))
        .unwrap(),
    )
    .unwrap();
    let McpTransportConfig::Http(remote) = config.servers()[0].transport() else {
        panic!()
    };
    let headers = McpResolvedHeaders::resolve(remote, |_| None, Some(b""), &[]).unwrap();
    McpAuthConfig::new(remote, &headers.authentication_identity_bytes(), |_| None).unwrap()
}

fn write_private(profile: &std::path::Path, name: &str, bytes: &[u8]) {
    fs::create_dir_all(profile).unwrap();
    fs::set_permissions(profile, fs::Permissions::from_mode(0o700)).unwrap();
    let path = profile.join(name);
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn auth_composition_is_inert_offline_and_uses_the_actual_host_workers() {
    let (fixture, profile) = fixture(false);
    let host = fixture.host();
    let service = host.mcp_authentication().unwrap();
    assert!(Arc::ptr_eq(
        &service,
        &host
            .mcp_controller()
            .unwrap()
            .authentication_service()
            .unwrap()
    ));
    assert!(!profile.exists());
    // A workspace lookalike is not the selected native credential namespace.
    write_private(
        &fixture.workspace,
        "mcp-credentials.json",
        b"malformed ambient lookalike",
    );
    let config = auth_config("http://127.0.0.1:9/mcp");
    run(async {
        let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
        let cancellation = CancellationToken::new();
        assert!(
            !service
                .status_owned(config.identity(), &cancellation, deadline)
                .await
                .unwrap()
        );
        assert!(
            !profile.exists(),
            "read-only absence does not create a profile"
        );
        host.control_workers().unwrap().close();
        assert_eq!(
            service
                .status_owned(config.identity(), &cancellation, deadline)
                .await,
            Err(McpAuthError::Unavailable),
            "auth must not silently create a replacement worker scope",
        );
    });
}

#[test]
fn retained_auth_observer_cannot_extend_engine_authority() {
    let (mut fixture, _) = fixture(false);
    let host = fixture.host.take().unwrap();
    let service = host.mcp_authentication().unwrap();
    let controller = host.mcp_controller().unwrap();
    let completion = host.terminal_shutdown_completion().unwrap();
    drop(host.into_engine());
    assert!(service.cleanup_status().complete);
    run(async {
        let config = auth_config("http://127.0.0.1:9/mcp");
        let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
        assert!(
            service
                .status_owned(config.identity(), &CancellationToken::new(), deadline)
                .await
                .is_err()
        );
        assert!(
            controller
                .settle(deadline, CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
    });
    completion.wait_on_worker().unwrap();
}

#[test]
fn stored_credentials_are_selected_again_for_each_actual_profile_reload() {
    let (fixture, profile) = fixture(true);
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let endpoint = format!("{origin}/mcp");
        let identity = auth_config(&endpoint);
        let credentials = json!({"version":1,"credentials":[{
            "identity":identity.identity(),"resource":endpoint,"issuer":origin,
            "registration":{"id":"client","secret":null,"method":"none"},
            "access":"selected-stored-token","refresh":null,"scope":"read","expires_ms":i64::MAX,
            "authorization_endpoint":format!("{origin}/authorize"),
            "token_endpoint":format!("{origin}/token"),"revocation_endpoint":null
        }]});
        write_private(
            &profile,
            "mcp-credentials.json",
            &serde_json::to_vec(&credentials).unwrap(),
        );
        let controller = fixture.host().mcp_controller().unwrap();
        for (index, name) in ["first", "second"].into_iter().enumerate() {
            write_private(
                &profile,
                "mcp.json",
                &serde_json::to_vec(&json!({"mcp": {
                    name: {"type":"http", "url":endpoint,"required":true}
                }}))
                .unwrap(),
            );
            let activation = if index == 0 {
                controller.start_configured(NativeMcpStartupPhase::All, CancellationToken::new())
            } else {
                controller.reload_configured(CancellationToken::new())
            };
            let response = super::http::reply(&listener,
                br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{}}}"#);
            let (receipt, request) = join(activation, response).await;
            let receipt = receipt.unwrap();
            assert!(receipt.startup().unwrap().required_ready());
            assert_eq!(receipt.startup().unwrap().servers[0].name.as_ref(), name);
            assert!(
                String::from_utf8(request)
                    .unwrap()
                    .contains("authorization: Bearer selected-stored-token")
            );
        }
        assert!(fixture.transport.requests.lock().unwrap().is_empty());
        let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
        assert!(
            controller
                .settle(deadline, CancellationToken::new())
                .await
                .unwrap()
                .complete
        );
        assert!(
            fixture
                .host()
                .mcp_authentication()
                .unwrap()
                .cleanup_status()
                .complete
        );
    });
}
