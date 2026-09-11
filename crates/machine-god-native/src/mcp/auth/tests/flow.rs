use super::super::*;
use crate::mcp::{
    config::{McpServerConfig, McpTransportConfig},
    endpoint::McpEndpoint,
    http::tests::{executor, request},
};
use futures_util::future::join;
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};

struct Clock;
impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(deadline.into()).await;
        })
    }
}
impl McpAuthClock for Clock {
    fn unix_millis(&self) -> i64 {
        1_000_000
    }
}
struct Entropy;
impl McpAuthEntropy for Entropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<()> {
        bytes.fill(42);
        Ok(())
    }
}
struct Network(SocketAddr);
impl McpAuthNetwork for Network {
    fn admit<'a>(
        &'a self,
        url: &'a str,
        _: &'a CancellationToken,
        _: Instant,
    ) -> BoxFuture<'a, Result<McpAuthDestination>> {
        Box::pin(async move {
            if url::Url::parse(url).unwrap().port() != Some(self.0.port()) {
                return Err(McpAuthError::Denied);
            }
            Ok(McpAuthDestination {
                destination: McpHttpDestination::new(McpEndpoint::parse(url).unwrap(), &[self.0])
                    .unwrap(),
                trust: None,
            })
        })
    }
}
#[derive(Default)]
struct Events(Mutex<Vec<McpAuthInvalidated>>);
impl McpAuthInvalidation for Events {
    fn invalidate(&self, event: McpAuthInvalidated) {
        self.0.lock().unwrap().push(event);
    }
}
struct Browser {
    approved: bool,
    launches: AtomicU64,
    socket: Mutex<Option<TcpStream>>,
    wrong_state: bool,
}
impl Browser {
    fn new(approved: bool) -> Self {
        Self {
            approved,
            launches: AtomicU64::new(0),
            socket: Mutex::new(None),
            wrong_state: false,
        }
    }
}
impl McpAuthBrowser for Browser {
    fn approve<'a>(
        &'a self,
        _: &'a McpAuthBrowserRequest,
        _: &'a CancellationToken,
        _: Instant,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move { Ok(self.approved) })
    }
    fn launch<'a>(
        &'a self,
        request: &'a McpAuthBrowserRequest,
        _: &'a CancellationToken,
        _: Instant,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            assert!(self.approved);
            self.launches.fetch_add(1, Ordering::Relaxed);
            let parsed = url::Url::parse(request.url()).unwrap();
            let params: std::collections::BTreeMap<_, _> =
                parsed.query_pairs().into_owned().collect();
            assert_eq!(params["code_challenge_method"], "S256");
            assert_eq!(params["code_challenge"].len(), 43);
            let redirect = url::Url::parse(&params["redirect_uri"]).unwrap();
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, redirect.port().unwrap()))
                .await
                .unwrap();
            let state = if self.wrong_state {
                "wrong"
            } else {
                &params["state"]
            };
            let target = format!(
                "/callback?{}",
                token::form(&[
                    ("code", "secret-code"),
                    ("state", state),
                    ("iss", request.issuer())
                ])
            );
            stream
                .write_all(format!("GET {target} HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n").as_bytes())
                .await
                .unwrap();
            *self.socket.lock().unwrap() = Some(stream);
            Ok(())
        })
    }
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    store: Arc<NativeMcpCredentialStore>,
    events: Arc<Events>,
    service: NativeMcpAuthService,
    config: McpAuthConfig,
    origin: String,
}
impl Fixture {
    fn new(address: SocketAddr) -> Self {
        let base = std::env::temp_dir().join(format!(
            "mg-mcp-auth-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let store = Arc::new(NativeMcpCredentialStore::new(base.join("profile")).unwrap());
        let events = Arc::new(Events::default());
        let service = NativeMcpAuthService::new(
            store.clone(),
            Arc::new(Network(address)),
            Arc::new(Clock),
            Arc::new(Entropy),
            events.clone(),
        );
        let origin = format!("http://127.0.0.1:{}", address.port());
        let config = config(&format!("{origin}/mcp"), b"selected");
        Self {
            base,
            store,
            events,
            service,
            config,
            origin,
        }
    }
    fn credentials(&self, expires_ms: i64) -> codec::Credentials {
        codec::Credentials {
            identity: self.config.identity.clone(),
            resource: format!("{}/mcp", self.origin).into(),
            issuer: self.origin.clone().into(),
            registration: codec::Registration {
                id: codec::Secret::new(b"client").unwrap(),
                secret: None,
                method: "none".into(),
            },
            access: codec::Secret::new(b"access-secret").unwrap(),
            refresh: Some(codec::Secret::new(b"refresh-secret").unwrap()),
            scope: "read".into(),
            expires_ms,
            authorization_endpoint: format!("{}/authorize", self.origin).into(),
            token_endpoint: format!("{}/token", self.origin).into(),
            revocation_endpoint: Some(format!("{}/revoke", self.origin).into()),
        }
    }
    fn seed(&self, expires_ms: i64) {
        self.store
            .publish(
                &self.store.load().unwrap(),
                self.config.identity(),
                Some(&self.credentials(expires_ms)),
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn config(endpoint: &str, identity: &[u8]) -> McpAuthConfig {
    let server = McpServerConfig::decode(
        "server",
        &serde_json::to_vec(&serde_json::json!({"type":"http","url":endpoint})).unwrap(),
    )
    .unwrap();
    let McpTransportConfig::Http(remote) = server.transport() else {
        panic!()
    };
    McpAuthConfig::new(remote, identity, |_| None).unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
async fn reply(listener: &TcpListener, status: u16, value: serde_json::Value) -> Vec<u8> {
    let (mut stream, _) = listener.accept().await.unwrap();
    let bytes = request(&mut stream).await;
    let body = serde_json::to_vec(&value).unwrap();
    stream.write_all(format!("HTTP/1.1 {status} Status\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
    stream.write_all(&body).await.unwrap();
    bytes
}
async fn discovery(listener: &TcpListener, origin: &str) {
    let request = reply(listener, 200, serde_json::json!({"resource":format!("{origin}/mcp"),"authorization_servers":[origin],"scopes_supported":["read"]})).await;
    assert!(request.starts_with(b"GET /.well-known/oauth-protected-resource/mcp "));
    assert!(!String::from_utf8_lossy(&request).contains("authorization:"));
    reply(listener, 200, serde_json::json!({"issuer":origin,"authorization_endpoint":format!("{origin}/authorize"),
        "token_endpoint":format!("{origin}/token"),"registration_endpoint":format!("{origin}/register"),"revocation_endpoint":format!("{origin}/revoke"),
        "code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"],"grant_types_supported":["authorization_code","refresh_token"],
        "scopes_supported":["offline_access"],"authorization_response_iss_parameter_supported":true})).await;
    let request = reply(
        listener,
        201,
        serde_json::json!({"client_id":"client","token_endpoint_auth_method":"none"}),
    )
    .await;
    assert!(request.starts_with(b"POST /register "));
}

#[test]
fn actual_browser_token_persistence_refresh_and_two_token_logout() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap());
        let browser = Browser::new(true);
        let client = async {
            let lease = fixture.service.authenticate(&fixture.config, &McpAuthChallenge::default(), &browser, &CancellationToken::new(), deadline()).await.unwrap();
            assert_eq!(lease.access_token().unwrap(), b"first-secret");
            assert!(fixture.service.status(fixture.config.identity()).unwrap());
            assert_eq!(fs::metadata(fixture.store.path()).unwrap().permissions().mode() & 0o777, 0o600);
            let refreshed = fixture.service.access_token(fixture.config.identity(), &CancellationToken::new(), deadline()).await.unwrap();
            assert_eq!(refreshed.access_token().unwrap(), b"second-secret");
            assert!(fixture.store.load().unwrap().get(fixture.config.identity()).unwrap().scope.is_empty());
            assert!(lease.access_token().is_err());
            let receipt = fixture.service.logout(fixture.config.identity(), &CancellationToken::new(), deadline()).await.unwrap();
            assert_eq!(receipt, McpAuthLogoutReceipt { local: McpAuthLocalRemoval::Removed, remote: McpAuthRemoteRevocation::Confirmed });
            assert!(refreshed.access_token().is_err());
            assert!(!fixture.service.status(fixture.config.identity()).unwrap());
            assert!(fixture.events.0.lock().unwrap().iter().all(|e| e.generation.is_cancelled()));
        };
        let server = async {
            discovery(&listener, &fixture.origin).await;
            let first = reply(&listener, 200, serde_json::json!({"access_token":"first-secret","refresh_token":"refresh-secret","expires_in":0})).await;
            let first = String::from_utf8(first).unwrap();
            assert!(first.contains("content-type: application/x-www-form-urlencoded"));
            assert!(first.contains("code_verifier=")); assert!(first.contains("grant_type=authorization_code"));
            let refresh = reply(&listener, 200, serde_json::json!({"access_token":"second-secret","scope":"","expires_in":3600})).await;
            assert!(String::from_utf8(refresh).unwrap().contains("grant_type=refresh_token"));
            for hint in ["refresh_token", "access_token"] {
                let request = reply(&listener, 200, serde_json::json!({})).await;
                assert!(String::from_utf8(request).unwrap().contains(&format!("token_type_hint={hint}")));
            }
        };
        join(client, server).await;
        assert_eq!(browser.launches.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn browser_decline_and_wrong_state_never_reach_token_or_store() {
    executor().block_on(async {
        for approved in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let fixture = Fixture::new(listener.local_addr().unwrap());
            let mut browser = Browser::new(approved);
            browser.wrong_state = true;
            let challenge = McpAuthChallenge::default();
            let cancellation = CancellationToken::new();
            let client = fixture.service.authenticate(
                &fixture.config,
                &challenge,
                &browser,
                &cancellation,
                deadline(),
            );
            let (result, ()) = join(client, discovery(&listener, &fixture.origin)).await;
            assert!(matches!(
                result,
                Err(McpAuthError::Denied | McpAuthError::StateMismatch)
            ));
            assert_eq!(
                browser.launches.load(Ordering::Relaxed),
                u64::from(approved)
            );
            assert!(!fixture.store.path().exists());
            assert!(
                tokio::time::timeout(Duration::from_millis(10), listener.accept())
                    .await
                    .is_err()
            );
        }
    });
}

#[test]
fn store_partitions_exact_endpoint_and_selection_and_rejects_stale_cas() {
    let fixture = Fixture::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 34567)));
    let stale = fixture.store.load().unwrap();
    fixture.seed(i64::MAX);
    let endpoint = config(&format!("{}/mcp?other=1", fixture.origin), b"selected");
    let selection = config(&format!("{}/mcp", fixture.origin), b"foreign");
    assert!(!fixture.service.status(endpoint.identity()).unwrap());
    assert!(!fixture.service.status(selection.identity()).unwrap());
    assert!(matches!(
        fixture.store.publish(
            &stale,
            fixture.config.identity(),
            Some(&fixture.credentials(0))
        ),
        Err(McpAuthError::Conflict)
    ));
    let observed = fixture.store.load().unwrap();
    let bytes = fs::read(fixture.store.path()).unwrap();
    let replacement = fixture.base.join("replacement");
    fs::write(&replacement, &bytes).unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
    fs::rename(&replacement, fixture.store.path()).unwrap();
    assert!(matches!(
        fixture
            .store
            .publish(&observed, fixture.config.identity(), None),
        Err(McpAuthError::Conflict)
    ));
    let mut expired = fixture.credentials(0);
    expired.refresh = None;
    fixture
        .store
        .publish(
            &fixture.store.load().unwrap(),
            fixture.config.identity(),
            Some(&expired),
        )
        .unwrap();
    assert!(matches!(
        executor().block_on(fixture.service.access_token(
            fixture.config.identity(),
            &CancellationToken::new(),
            deadline()
        )),
        Err(McpAuthError::Unavailable)
    ));
}

#[test]
fn logout_during_partial_refresh_prevents_resurrection_and_reports_remote_state() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap());
        fixture.seed(0);
        let caller = CancellationToken::new();
        let refresh = fixture
            .service
            .access_token(fixture.config.identity(), &caller, deadline());
        let race = async {
            let (mut pending, _) = listener.accept().await.unwrap();
            let bytes = request(&mut pending).await;
            assert!(
                String::from_utf8(bytes)
                    .unwrap()
                    .contains("grant_type=refresh_token")
            );
            assert!(matches!(
                fixture
                    .service
                    .access_token(
                        fixture.config.identity(),
                        &CancellationToken::new(),
                        deadline()
                    )
                    .await,
                Err(McpAuthError::Busy)
            ));
            let logout_cancel = CancellationToken::new();
            let logout =
                fixture
                    .service
                    .logout(fixture.config.identity(), &logout_cancel, deadline());
            let remote = async {
                reply(&listener, 200, serde_json::json!({})).await;
                reply(&listener, 200, serde_json::json!({})).await;
            };
            let (receipt, ()) = join(logout, remote).await;
            assert_eq!(
                receipt.unwrap(),
                McpAuthLogoutReceipt {
                    local: McpAuthLocalRemoval::Removed,
                    remote: McpAuthRemoteRevocation::Ambiguous
                }
            );
            // The old pending refresh is never committed, even if the server
            // later tries to return a newly rotated credential.
            let _ = pending
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await;
        };
        let (result, ()) = join(refresh, race).await;
        assert!(matches!(result, Err(McpAuthError::Conflict)));
        assert!(!fixture.service.status(fixture.config.identity()).unwrap());
    });
}

#[test]
fn cancellation_and_dropped_polled_refresh_release_socket_without_replay() {
    use tokio::io::AsyncReadExt;
    executor().block_on(async {
        for cancel in [true, false] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let fixture = Fixture::new(listener.local_addr().unwrap());
            fixture.seed(0);
            let caller = CancellationToken::new();
            let mut refresh = Box::pin(fixture.service.access_token(
                fixture.config.identity(),
                &caller,
                deadline(),
            ));
            let accept = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                request(&mut stream).await;
                stream
            };
            let mut stream =
                match futures_util::future::select(refresh.as_mut(), Box::pin(accept)).await {
                    futures_util::future::Either::Right((stream, _)) => stream,
                    futures_util::future::Either::Left(_) => {
                        panic!("refresh completed before response")
                    }
                };
            if cancel {
                caller.cancel();
                assert!(matches!(
                    refresh.as_mut().await,
                    Err(McpAuthError::Cancelled)
                ));
            }
            drop(refresh);
            let mut byte = [0];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
            assert_eq!(
                fixture
                    .store
                    .load()
                    .unwrap()
                    .get(fixture.config.identity())
                    .unwrap()
                    .access
                    .text(),
                "access-secret"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(10), listener.accept())
                    .await
                    .is_err()
            );
        }
    });
}

#[test]
fn unpolled_and_precancelled_auth_and_logout_have_no_effects() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap());
        let browser = Browser::new(true);
        let challenge = McpAuthChallenge::default();
        let cancellation = CancellationToken::new();
        drop(fixture.service.authenticate(
            &fixture.config,
            &challenge,
            &browser,
            &cancellation,
            deadline(),
        ));
        cancellation.cancel();
        assert!(matches!(
            fixture
                .service
                .authenticate(
                    &fixture.config,
                    &challenge,
                    &browser,
                    &cancellation,
                    deadline()
                )
                .await,
            Err(McpAuthError::Cancelled)
        ));
        assert!(matches!(
            fixture
                .service
                .logout(fixture.config.identity(), &cancellation, deadline())
                .await,
            Err(McpAuthError::Cancelled)
        ));
        assert!(!fixture.store.path().exists());
        assert_eq!(browser.launches.load(Ordering::Relaxed), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn local_delete_failure_is_not_hidden_by_confirmed_remote_revocation() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap());
        fixture.seed(i64::MAX);
        let lease = fixture
            .service
            .access_token(
                fixture.config.identity(),
                &CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        // A retained cooperative writer lock produces a prepublication failure.
        let lock = fs::File::open(fixture.base.join("profile/.mcp-credentials.lock")).unwrap();
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).unwrap();
        let cancellation = CancellationToken::new();
        let logout = fixture
            .service
            .logout(fixture.config.identity(), &cancellation, deadline());
        let remote = async {
            reply(&listener, 200, serde_json::json!({})).await;
            reply(&listener, 200, serde_json::json!({})).await;
        };
        let (receipt, ()) = join(logout, remote).await;
        assert_eq!(
            receipt.unwrap(),
            McpAuthLogoutReceipt {
                local: McpAuthLocalRemoval::Failed,
                remote: McpAuthRemoteRevocation::Confirmed
            }
        );
        assert!(lease.access_token().is_err());
        assert!(fixture.service.status(fixture.config.identity()).unwrap());
    });
}

#[test]
fn local_retirement_releases_capacity_without_deleting_credentials() {
    executor().block_on(async {
        let fixture = Fixture::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 34568)));
        fixture.seed(i64::MAX);
        let lease = fixture
            .service
            .access_token(
                fixture.config.identity(),
                &CancellationToken::new(),
                deadline(),
            )
            .await
            .unwrap();
        fixture.service.retire(fixture.config.identity());
        assert!(lease.access_token().is_err());
        assert!(fixture.service.status(fixture.config.identity()).unwrap());
        for index in 0..70 {
            let selected = config(
                &format!("{}/mcp?selection={index}", fixture.origin),
                b"selection",
            );
            assert!(matches!(
                fixture
                    .service
                    .access_token(selected.identity(), &CancellationToken::new(), deadline())
                    .await,
                Err(McpAuthError::Missing)
            ));
            fixture.service.retire(selected.identity());
        }
    });
}

#[test]
fn unsupported_revocation_of_both_tokens_remains_explicitly_unsupported() {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let fixture = Fixture::new(listener.local_addr().unwrap());
        fixture.seed(i64::MAX);
        let cancellation = CancellationToken::new();
        let logout = fixture
            .service
            .logout(fixture.config.identity(), &cancellation, deadline());
        let remote = async {
            reply(&listener, 405, serde_json::json!({})).await;
            reply(&listener, 405, serde_json::json!({})).await;
        };
        let (receipt, ()) = join(logout, remote).await;
        assert_eq!(
            receipt.unwrap(),
            McpAuthLogoutReceipt {
                local: McpAuthLocalRemoval::Removed,
                remote: McpAuthRemoteRevocation::Unsupported
            }
        );
    });
}
