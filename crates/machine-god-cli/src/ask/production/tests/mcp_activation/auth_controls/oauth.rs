use super::{browser::Browser, server::Server, *};
use futures_util::future::join;
use machine_god_native::mcp::{
    commands::McpFeatureCommand,
    control::McpFeatureReply,
    feature::{McpFeatureOutcome, McpResourceData},
};

fn configured() -> Fixture {
    Fixture::configured(|endpoint| {
        serde_json::json!({"type":"http", "url":endpoint,
        "required":true, "startup_timeout_ms":5000, "operation_timeout_ms":5000})
    })
}

async fn authorize(
    session: &mut NativeInteractiveSession,
    browser: &Browser,
    server: &Server,
    activation: bool,
) -> NativeInteractiveControlOutcome {
    let producer = async {
        let redirect = server.discovery().await;
        let (browser_fields, token_fields) =
            join(browser.callback(&server.origin, true), server.token()).await;
        assert_eq!(browser_fields["redirect_uri"], redirect);
        assert_eq!(token_fields["redirect_uri"], redirect);
        server.activate(activation).await;
    };
    let (observed, ()) = join(
        management(
            session,
            McpCommand::Authenticate {
                server: "remote".into(),
                open_browser: true,
            },
        ),
        producer,
    )
    .await;
    observed
}

fn credential(fixture: &Fixture) -> serde_json::Value {
    assert_eq!(
        fs::metadata(fixture.credentials())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let mut stored: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.credentials()).unwrap()).unwrap();
    assert_eq!(stored["version"], 1);
    assert_eq!(stored["credentials"].as_array().unwrap().len(), 1);
    let value = stored["credentials"][0].take();
    assert_eq!(value["access"], "accepted-access-secret");
    assert_eq!(value["refresh"], "accepted-refresh-secret");
    assert_eq!(value["registration"]["id"], "local-client");
    value
}

#[test]
fn mcp_explicit_browser_oauth_activates_authenticated_resource_and_populated_logout() {
    for revoke in [true, false] {
        let fixture = configured();
        let browser = Browser::new(&fixture.directory);
        fixture.run(async {
            let server = Server::new(&fixture);
            let mut session = fixture.session_with_options(|options| browser.options(options)).await;
            let confirmation = management(&mut session, authenticate("remote")).await;
            assert!(matches!(confirmation.result, Ok(NativeInteractiveControlReceipt::McpAuthentication(NativeMcpAuthenticationReceipt::ConfirmationRequired { .. }))));
            assert!(browser.untouched());
            assert!(!fixture.credentials().exists());
            assert_eq!(fixture.listener.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
            let observed = authorize(&mut session, &browser, &server, true).await;
            assert!(!observed.failed());
            assert!(matches!(observed.result, Ok(NativeInteractiveControlReceipt::McpAuthentication(NativeMcpAuthenticationReceipt::Authenticated { usable: true, activation: Some(Ok(_)), .. }))));
            let persisted = credential(&fixture);
            assert_eq!(persisted["identity"]["endpoint"], format!("{}/mcp", server.origin));
            assert!(fixture.host().mcp_controller().unwrap().required_readiness().is_ok());
            let (read, ()) = join(management(&mut session, McpCommand::Feature(McpFeatureCommand::ResourceRead {
                server: "remote".into(), uri: "memo://private".into(),
            })), server.resource()).await;
            assert!(!read.failed());
            let Ok(NativeInteractiveControlReceipt::McpFeature(read)) = read.result else { panic!("resource receipt") };
            let McpFeatureReply::Response(response) = read.reply() else { panic!("resource response") };
            let McpFeatureOutcome::Resource { contents, .. } = response.outcome() else { panic!("resource contents") };
            assert!(matches!(contents[0].data(), McpResourceData::Text(value) if value.as_ref() == "authenticated private result"));
            assert!(read.revalidate().is_ok());
            let (logout, ()) = join(management(&mut session, McpCommand::Logout { server: "remote".into() }), server.logout(revoke)).await;
            assert_eq!(logout.failed(), !revoke);
            assert!(matches!(logout.result, Ok(NativeInteractiveControlReceipt::McpAuthentication(NativeMcpAuthenticationReceipt::LoggedOut { outcome, .. }))
                if outcome.local == McpAuthLocalRemoval::Removed && outcome.remote == if revoke { McpAuthRemoteRevocation::Confirmed } else { McpAuthRemoteRevocation::Ambiguous }));
            let stored: serde_json::Value = serde_json::from_slice(&fs::read(fixture.credentials()).unwrap()).unwrap();
            assert!(stored["credentials"].as_array().unwrap().is_empty());
            assert_eq!(fixture.listener.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock, "logout does not reload");
            assert!(read.revalidate().is_err());
            drop(read);
            shutdown(session).await;
        });
    }
}

#[test]
fn mcp_confirmed_credential_save_survives_failed_authenticated_activation() {
    let fixture = configured();
    let browser = Browser::new(&fixture.directory);
    fixture.run(async {
        let server = Server::new(&fixture);
        let mut session = fixture
            .session_with_options(|options| browser.options(options))
            .await;
        let observed = authorize(&mut session, &browser, &server, false).await;
        assert!(observed.failed());
        assert!(matches!(
            observed.result,
            Ok(NativeInteractiveControlReceipt::McpAuthentication(
                NativeMcpAuthenticationReceipt::Authenticated {
                    usable: true,
                    activation: Some(Err(_)),
                    ..
                }
            ))
        ));
        let _ = credential(&fixture);
        assert!(
            fixture
                .host()
                .mcp_controller()
                .unwrap()
                .required_readiness()
                .is_err()
        );
        shutdown(session).await;
    });
}

#[test]
fn mcp_browser_wrong_state_cannot_exchange_token_save_credentials_or_activate() {
    let fixture = configured();
    let browser = Browser::new(&fixture.directory);
    fixture.run(async {
        let server = Server::new(&fixture);
        let mut session = fixture
            .session_with_options(|options| browser.options(options))
            .await;
        let producer = async {
            let redirect = server.discovery().await;
            let fields = browser.callback(&server.origin, false).await;
            assert_eq!(fields["redirect_uri"], redirect);
        };
        let (observed, ()) = join(
            management(
                &mut session,
                McpCommand::Authenticate {
                    server: "remote".into(),
                    open_browser: true,
                },
            ),
            producer,
        )
        .await;
        assert!(matches!(
            observed.result,
            Err(NativeInteractiveControlError::McpAuthentication(
                NativeMcpAuthenticationError::Authorization(McpAuthError::Invalid)
            ))
        ));
        assert!(!fixture.credentials().exists());
        assert!(
            fixture
                .host()
                .mcp_controller()
                .unwrap()
                .required_readiness()
                .is_err()
        );
        shutdown(session).await;
    });
}
