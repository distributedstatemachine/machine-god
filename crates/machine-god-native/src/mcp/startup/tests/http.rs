use super::*;
mod configured;
use crate::mcp::{
    auth::{
        McpAuthClock, McpAuthConfig, McpAuthEntropy, McpAuthError, McpAuthInvalidated,
        McpAuthInvalidation, NativeMcpAuthService, NativeMcpCredentialStore,
    },
    headers::McpResolvedHeaders,
    http::{
        McpHttpClock,
        tests::{executor, request},
    },
    network::{McpResolverConfig, NativeMcpNetwork},
    protocol::ProtocolVersion,
};
use futures_util::future::{Either, join, select};
use serde_json::json;
use std::{
    fs,
    io::Write,
    net::{Ipv4Addr, SocketAddr},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn run(future: impl std::future::Future<Output = ()>) {
    executor().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), future)
            .await
            .expect("owned startup fixture must settle");
    });
}

impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        NativeMcpRuntimeClock::now(self)
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        NativeMcpRuntimeClock::sleep_until(self, deadline)
    }
}
impl McpAuthClock for Clock {
    fn unix_millis(&self) -> i64 {
        1_000_000
    }
}

fn http_options(address: SocketAddr) -> NativeMcpStartupOptions {
    let config = json!({"mcp":{"remote":{"type":"http","url":format!("http://127.0.0.1:{}/mcp", address.port()),
        "required":true,"startup_timeout_ms":5000,"operation_timeout_ms":u32::MAX,
        "headers":{"X-Selected":"static-secret"},"header_env":{"X-Capture":"TOKEN"}}}});
    let mut selected = options(&config.to_string());
    selected.captured_environment = vec![("TOKEN".into(), "captured-secret".into())];
    selected.network = Some(Arc::new(
        NativeMcpNetwork::new(
            McpResolverConfig::literal_only(),
            [7; 32],
            None,
            Arc::new(Clock::default()),
            CancellationToken::new(),
            1,
        )
        .unwrap(),
    ));
    selected
}
async fn reply(listener: &TcpListener, body: &[u8]) -> Vec<u8> {
    let (mut socket, _) = listener.accept().await.unwrap();
    let received = request(&mut socket).await;
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
    socket.flush().await.unwrap();
    received
}
fn discover(tools: bool) -> Vec<u8> {
    let mut capabilities = json!({"resources":{},"prompts":{}});
    if tools {
        capabilities["tools"] = json!({});
    }
    serde_json::to_vec(
        &json!({"jsonrpc":"2.0","id":1,"result":{"resultType":"complete",
        "supportedVersions":["2026-07-28"],"capabilities":capabilities}}),
    )
    .unwrap()
}

#[test]
fn actual_http_startup_pages_tools_exactly_and_keeps_feature_catalogs_lazy() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = http_options(listener.local_addr().unwrap());
        let epoch = Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap();
        selected.catalog_epoch = epoch;
        let startup = NativeMcpStartup::new(selected).unwrap();
        let server = async {
            let first = reply(&listener, &discover(true)).await;
            assert!(String::from_utf8_lossy(&first).contains("x-selected: static-secret"));
            assert!(String::from_utf8_lossy(&first).contains("x-capture: captured-secret"));
            let request = reply(&listener, br#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","nextCursor":"second","tools":[{"name":"first","inputSchema":{"type":"object","minimum":9007199254740993.0001,"$serde_json::private::Number":"literal"}}]}}"#).await;
            assert!(String::from_utf8_lossy(&request).contains("tools/list"));
            let request = reply(&listener, br#"{"jsonrpc":"2.0","id":3,"result":{"resultType":"complete","tools":[{"name":"second","inputSchema":{"type":"object"}}]}}"#).await;
            assert!(String::from_utf8_lossy(&request).contains("second"));
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
        assert!(batch.receipt().required_ready());
        assert_eq!(batch.servers().len(), 1);
        assert_eq!(
            batch.servers()[0].operation_timeout,
            Duration::from_millis(u64::from(u32::MAX))
        );
        assert_eq!(batch.servers()[0].catalogs.len(), 1);
        assert_eq!(batch.servers()[0].catalog_epoch, epoch);
        let catalog = &batch.servers()[0].catalogs[0];
        assert!(catalog.fetched_at_ms() >= 3_600_000);
        assert_eq!(catalog.descriptors().len(), 2);
        assert_eq!(catalog.version(), ProtocolVersion::Modern);
        let crate::mcp::catalog::McpDescriptor::Tool(first) = &catalog.descriptors()[0] else {
            panic!("tools descriptor");
        };
        assert!(
            first
                .input_schema()
                .raw_json()
                .contains("9007199254740993.0001")
        );
        assert!(
            first
                .input_schema()
                .raw_json()
                .contains("$serde_json::private::Number")
        );
        let runtime = runtime();
        let (candidate, receipt) = batch
            .prepare(&runtime, &[], NativeMcpStartupRequirement::Required)
            .unwrap();
        assert_eq!(candidate.descriptors().tools().len(), 2);
        assert!(!receipt.cleanup_complete());
        drop(candidate);
        assert!(receipt.cleanup_complete());
        assert!(startup.cleanup_observations().is_empty());
    });
}

#[test]
fn feature_only_peer_is_ready_without_any_unadvertised_startup_list() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let startup = NativeMcpStartup::new(http_options(listener.local_addr().unwrap())).unwrap();
        let (batch, _) = join(
            startup.build(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            reply(&listener, &discover(false)),
        )
        .await;
        assert!(batch.receipt().required_ready());
        assert!(batch.servers()[0].catalogs.is_empty());
        let runtime = runtime();
        let (candidate, receipt) = batch
            .prepare(&runtime, &[], NativeMcpStartupRequirement::Required)
            .unwrap();
        assert_eq!(candidate.descriptors().servers().len(), 1);
        assert!(candidate.descriptors().tools().is_empty());
        drop(candidate);
        assert!(receipt.cleanup_complete());
    });
}

#[test]
fn abandoned_polled_http_startup_retains_cleanup_observation_and_closes_socket() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let startup = NativeMcpStartup::new(http_options(listener.local_addr().unwrap())).unwrap();
        let entered = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let received = request(&mut socket).await;
            assert!(String::from_utf8_lossy(&received).contains("server/discover"));
            socket
        };
        let future = startup.build(
            NativeMcpStartupPhase::All,
            CancellationToken::new(),
            deadline(),
        );
        let Either::Right((mut socket, pending)) = select(future, Box::pin(entered)).await else {
            panic!("server must remain pending");
        };
        let completion = startup.cleanup_observations();
        assert_eq!(completion.len(), 1);
        assert!(!completion[0].is_complete());
        drop(pending);
        assert!(completion[0].is_complete());
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), socket.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    });
}

#[test]
fn network_generation_cancellation_stops_pending_response_without_replay() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let selected = http_options(listener.local_addr().unwrap());
        let cancellation = selected.network.as_ref().unwrap().owner_cancellation();
        let startup = NativeMcpStartup::new(selected).unwrap();
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            cancellation.cancel();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
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
        assert!(!batch.receipt().required_ready());
        assert!(batch.receipt().cleanup_complete());
        assert_eq!(
            batch.receipt().servers[0].state,
            NativeMcpStartupState::Failed(NativeMcpStartupError::Cancelled)
        );
    });
}

struct Entropy;
impl McpAuthEntropy for Entropy {
    fn fill(&self, bytes: &mut [u8]) -> std::result::Result<(), McpAuthError> {
        bytes.fill(9);
        Ok(())
    }
}
struct Events;
impl McpAuthInvalidation for Events {
    fn invalidate(&self, _: McpAuthInvalidated) {}
}
struct Credentials {
    directory: PathBuf,
    service: Arc<NativeMcpAuthService>,
    store: Arc<NativeMcpCredentialStore>,
}
impl Credentials {
    fn new(network: Arc<NativeMcpNetwork>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mg-mcp-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = fs::canonicalize(directory).unwrap();
        let store = Arc::new(NativeMcpCredentialStore::new(directory.clone()).unwrap());
        let service = Arc::new(NativeMcpAuthService::new(
            store.clone(),
            network,
            Arc::new(Clock::default()),
            Arc::new(Entropy),
            Arc::new(Events),
        ));
        Self {
            directory,
            service,
            store,
        }
    }
    fn seed(&self, config: &McpAuthConfig, expires_ms: i64) {
        let endpoint = config.identity().endpoint();
        let origin = url::Url::parse(endpoint)
            .unwrap()
            .origin()
            .ascii_serialization();
        let value = json!({"version":1,"credentials":[{"identity":config.identity(),"resource":endpoint,
            "issuer":origin,"registration":{"id":"client","secret":null,"method":"none"},
            "access":"old-secret","refresh":"refresh-secret","scope":"read","expires_ms":expires_ms,
            "authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),
            "revocation_endpoint":null}]});
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(self.store.path())
            .unwrap();
        file.write_all(&serde_json::to_vec(&value).unwrap())
            .unwrap();
    }
}
impl Drop for Credentials {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn authentication(source: NativeMcpStartupAuthSource) -> NativeMcpStartupAuthentication {
    NativeMcpStartupAuthentication {
        server: "remote".into(),
        additional_headers: McpResolvedHeaders::from_resolved(&[]).unwrap(),
        source,
    }
}

#[test]
fn stored_refresh_is_real_and_resource_headers_never_reach_token_endpoint() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = http_options(listener.local_addr().unwrap());
        let credentials = Credentials::new(selected.network.as_ref().unwrap().clone());
        selected
            .authentication
            .push(authentication(NativeMcpStartupAuthSource::Stored(
                credentials.service.clone(),
            )));
        let startup = NativeMcpStartup::new(selected).unwrap();
        credentials.seed(&startup.authentication_config("remote").unwrap(), 0);
        let server = async {
            let token = reply(
                &listener,
                br#"{"access_token":"renewed-secret","expires_in":3600}"#,
            )
            .await;
            assert!(token.starts_with(b"POST /token HTTP/1.1"));
            assert!(String::from_utf8_lossy(&token).contains("grant_type=refresh_token"));
            assert!(!String::from_utf8_lossy(&token).contains("static-secret"));
            assert!(!String::from_utf8_lossy(&token).contains("captured-secret"));
            let discover = reply(&listener, &discover(false)).await;
            assert!(
                String::from_utf8_lossy(&discover).contains("authorization: Bearer renewed-secret")
            );
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
        assert!(batch.receipt().required_ready());
        assert_eq!(batch.servers()[0].authority_cancellations.len(), 4);
        let generation = batch.servers()[0].authority_cancellations[3].clone();
        credentials
            .service
            .retire(startup.authentication_config("remote").unwrap().identity());
        assert!(generation.is_cancelled());
        let failure = batch
            .prepare(&runtime(), &[], NativeMcpStartupRequirement::Required)
            .unwrap_err();
        assert!(failure.receipt.cleanup_complete());
    });
}

#[test]
fn foreign_endpoint_or_changed_header_selection_cannot_reuse_an_oauth_lease() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let original = http_options(listener.local_addr().unwrap());
        let credentials = Credentials::new(original.network.as_ref().unwrap().clone());
        let original = NativeMcpStartup::new(original).unwrap();
        let identity = original.authentication_config("remote").unwrap();
        credentials.seed(&identity, i64::MAX);
        let lease = Arc::new(
            credentials
                .service
                .access_token(identity.identity(), &CancellationToken::new(), deadline())
                .await
                .unwrap(),
        );
        for endpoint_change in [false, true] {
            let mut selected = http_options(listener.local_addr().unwrap());
            let mut config =
                machine_god_core::json::from_slice(&selected.configuration.encode().unwrap())
                    .unwrap();
            if endpoint_change {
                config["mcp"]["remote"]["url"] = json!(format!(
                    "http://127.0.0.1:{}/other",
                    listener.local_addr().unwrap().port()
                ));
            } else {
                config["mcp"]["remote"]["headers"]["X-Selected"] = json!("changed-selection");
            }
            selected.configuration =
                Arc::new(McpConfig::decode(&serde_json::to_vec(&config).unwrap()).unwrap());
            selected
                .authentication
                .push(authentication(NativeMcpStartupAuthSource::Lease(
                    lease.clone(),
                )));
            let startup = NativeMcpStartup::new(selected).unwrap();
            let batch = startup
                .build(
                    NativeMcpStartupPhase::All,
                    CancellationToken::new(),
                    deadline(),
                )
                .await;
            assert_eq!(
                batch.receipt().servers[0].state,
                NativeMcpStartupState::Failed(NativeMcpStartupError::Authentication)
            );
            assert!(batch.receipt().cleanup_complete());
            assert!(!format!("{startup:?}").contains("static-secret"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(5), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn missing_stored_credentials_allow_configured_bearer_but_corruption_never_does() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = http_options(listener.local_addr().unwrap());
        let mut config =
            machine_god_core::json::from_slice(&selected.configuration.encode().unwrap()).unwrap();
        config["mcp"]["remote"]["bearer_token_env"] = json!("TOKEN");
        selected.configuration =
            Arc::new(McpConfig::decode(&serde_json::to_vec(&config).unwrap()).unwrap());
        let credentials = Credentials::new(selected.network.as_ref().unwrap().clone());
        selected
            .authentication
            .push(authentication(NativeMcpStartupAuthSource::Stored(
                credentials.service.clone(),
            )));
        let startup = NativeMcpStartup::new(selected).unwrap();
        let (batch, received) = join(
            startup.build(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            reply(&listener, &discover(false)),
        )
        .await;
        assert!(batch.receipt().required_ready());
        assert!(
            String::from_utf8_lossy(&received).contains("authorization: Bearer captured-secret")
        );
        drop(batch);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(credentials.store.path())
            .unwrap();
        file.write_all(b"malformed selected credentials").unwrap();
        let batch = startup
            .build(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            )
            .await;
        assert_eq!(
            batch.receipt().servers[0].state,
            NativeMcpStartupState::Failed(NativeMcpStartupError::Authentication)
        );
        assert!(batch.receipt().cleanup_complete());
        assert!(
            tokio::time::timeout(Duration::from_millis(5), listener.accept())
                .await
                .is_err()
        );
    });
}
