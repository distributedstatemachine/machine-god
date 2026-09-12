use super::*;
use crate::mcp::{
    auth::{McpAuthClock, McpAuthDestination, McpAuthEntropy, McpAuthNetwork},
    config::{McpServerConfig, McpTransportConfig},
    endpoint::McpEndpoint,
    http::{
        McpHttpClock, McpHttpDestination,
        tests::{executor, request},
    },
};
use futures_util::{
    FutureExt,
    future::{Either, poll_fn, select},
    task::AtomicWaker,
};
use machine_god_core::BoxFuture;
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    task::Poll,
};
use tokio::{net::TcpStream, sync::oneshot};

struct Clock {
    base: Instant,
    now: Mutex<Instant>,
    deadlines: Mutex<Vec<Instant>>,
    wake: AtomicWaker,
}
impl Clock {
    fn new() -> Self {
        let base = Instant::now();
        Self {
            base,
            now: Mutex::new(base),
            deadlines: Mutex::default(),
            wake: AtomicWaker::new(),
        }
    }
    fn at(&self, seconds: u64) -> Instant {
        self.base + Duration::from_secs(seconds)
    }
    fn advance(&self, seconds: u64) {
        *self.now.lock().unwrap() += Duration::from_secs(seconds);
        self.wake.wake();
    }
    async fn observed(&self, after: usize, expected: Instant) {
        let deadline = poll_fn(|cx| {
            self.wake.register(cx.waker());
            if let Some(deadline) = self.deadlines.lock().unwrap().get(after) {
                Poll::Ready(*deadline)
            } else {
                Poll::Pending
            }
        })
        .await;
        assert_eq!(deadline, expected);
    }
}
impl McpHttpClock for Clock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        self.deadlines.lock().unwrap().push(deadline);
        self.wake.wake();
        // All fixture futures are polled by one joined task, never detached.
        Box::pin(poll_fn(move |cx| {
            self.wake.register(cx.waker());
            if self.now() >= deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}
impl McpAuthClock for Clock {
    fn unix_millis(&self) -> i64 {
        0
    }
}
struct Entropy;
impl McpAuthEntropy for Entropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<()> {
        bytes.fill(42);
        Ok(())
    }
}
struct Network {
    address: SocketAddr,
    clock: Arc<Clock>,
    token_delay: u64,
    requests: Mutex<Vec<(String, Instant, Instant)>>,
}
impl McpAuthNetwork for Network {
    fn admit<'a>(
        &'a self,
        url: &'a str,
        _: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<McpAuthDestination>> {
        Box::pin(async move {
            let parsed = url::Url::parse(url).unwrap();
            assert_eq!(parsed.port(), Some(self.address.port()));
            self.requests
                .lock()
                .unwrap()
                .push((parsed.path().into(), self.clock.now(), deadline));
            self.clock.advance(if parsed.path() == "/token" {
                self.token_delay
            } else {
                20
            });
            Ok(McpAuthDestination {
                destination: McpHttpDestination::new(
                    McpEndpoint::parse(url).unwrap(),
                    &[self.address],
                )
                .unwrap(),
                trust: None,
            })
        })
    }
}
struct Callback {
    address: SocketAddr,
    target: String,
    timer_marker: usize,
}
struct Browser {
    clock: Arc<Clock>,
    handoff: Mutex<Option<oneshot::Sender<Callback>>>,
    deadlines: Mutex<Vec<Instant>>,
}
impl McpAuthBrowser for Browser {
    fn approve<'a>(
        &'a self,
        _: &'a McpAuthBrowserRequest,
        _: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            self.deadlines.lock().unwrap().push(deadline);
            self.clock.advance(360);
            Ok(true)
        })
    }
    fn launch<'a>(
        &'a self,
        request: &'a McpAuthBrowserRequest,
        _: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.deadlines.lock().unwrap().push(deadline);
            self.clock.advance(360);
            let parsed = url::Url::parse(request.url()).unwrap();
            let params: std::collections::BTreeMap<_, _> =
                parsed.query_pairs().into_owned().collect();
            let redirect = url::Url::parse(&params["redirect_uri"]).unwrap();
            let callback = Callback {
                address: SocketAddr::from((Ipv4Addr::LOCALHOST, redirect.port().unwrap())),
                timer_marker: self.clock.deadlines.lock().unwrap().len(),
                target: format!(
                    "/callback?{}",
                    token::form(&[
                        ("code", "code"),
                        ("state", &params["state"]),
                        ("iss", request.issuer()),
                    ])
                ),
            };
            assert!(
                self.handoff
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(callback)
                    .is_ok()
            );
            Ok(())
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    Success,
    CallbackDeadline,
    OuterDeadline,
    SocketDeadline,
    NetworkDeadline,
    Cancelled,
}
struct Fixture {
    authority: Authority,
    browser: Browser,
    clock: Arc<Clock>,
    network: Arc<Network>,
    config: McpAuthConfig,
    origin: String,
}
impl Fixture {
    fn new(address: SocketAddr, end: End) -> (Self, oneshot::Receiver<Callback>) {
        let clock = Arc::new(Clock::new());
        let network = Arc::new(Network {
            address,
            clock: clock.clone(),
            token_delay: if end == End::NetworkDeadline { 30 } else { 2 },
            requests: Mutex::default(),
        });
        let origin = format!("http://127.0.0.1:{}", address.port());
        let server = McpServerConfig::decode(
            "fixture",
            &serde_json::to_vec(&json!({
                "type":"http", "url":format!("{origin}/mcp"),
            }))
            .unwrap(),
        )
        .unwrap();
        let McpTransportConfig::Http(remote) = server.transport() else {
            panic!()
        };
        let config = McpAuthConfig::new(remote, b"fixture", |_| None).unwrap();
        let (send, receive) = oneshot::channel();
        (
            Self {
                authority: Authority {
                    network: network.clone(),
                    clock: clock.clone(),
                    entropy: Arc::new(Entropy),
                },
                browser: Browser {
                    clock: clock.clone(),
                    handoff: Mutex::new(Some(send)),
                    deadlines: Mutex::default(),
                },
                clock,
                network,
                config,
                origin,
            },
            receive,
        )
    }
    async fn discovery(&self, listener: &TcpListener) {
        let origin = &self.origin;
        reply(
            listener,
            json!({"resource":format!("{origin}/mcp"), "authorization_servers":[origin]}),
        )
        .await;
        reply(listener, json!({
            "issuer":origin, "authorization_endpoint":format!("{origin}/authorize"),
            "token_endpoint":format!("{origin}/token"), "registration_endpoint":format!("{origin}/register"),
            "code_challenge_methods_supported":["S256"], "token_endpoint_auth_methods_supported":["none"],
            "grant_types_supported":["authorization_code"], "authorization_response_iss_parameter_supported":true,
        })).await;
        reply(
            listener,
            json!({"client_id":"client", "token_endpoint_auth_method":"none"}),
        )
        .await;
    }
}
async fn reply(listener: &TcpListener, body: serde_json::Value) {
    let (mut socket, _) = listener.accept().await.unwrap();
    request(&mut socket).await;
    let bytes = serde_json::to_vec(&body).unwrap();
    socket.write_all(format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", bytes.len()
    ).as_bytes()).await.unwrap();
    socket.write_all(&bytes).await.unwrap();
}

async fn drive_callback(
    fixture: &Fixture,
    callback: &Callback,
    cancellation: &CancellationToken,
    end: End,
) -> Option<TcpStream> {
    let clock = &fixture.clock;
    assert_eq!(clock.now(), clock.at(780)); // 3 exchanges, consent, successful launch.
    clock
        .observed(
            callback.timer_marker,
            clock.at(if end == End::OuterDeadline { 830 } else { 1080 }),
        )
        .await;
    match end {
        End::CallbackDeadline => {
            clock.advance(300);
            return None;
        }
        End::OuterDeadline => {
            clock.advance(50);
            return None;
        }
        End::Cancelled => {
            cancellation.cancel();
            return None;
        }
        _ => {}
    }
    clock.advance(299);
    let marker = clock.deadlines.lock().unwrap().len();
    let mut socket = TcpStream::connect(callback.address).await.unwrap();
    clock.observed(marker, clock.at(1109)).await; // Accepted at t=1079, not capped at t=1080.
    if end == End::SocketDeadline {
        clock.advance(30);
    } else {
        clock.advance(2); // Headers arrive after the callback acceptance cutoff.
        socket
            .write_all(
                format!(
                    "GET {} HTTP/1.1\r\nhost: localhost\r\n\r\n",
                    callback.target
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }
    Some(socket)
}

fn scenario(end: End) {
    executor().block_on(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (fixture, handoff) = Fixture::new(listener.local_addr().unwrap(), end);
        let cancellation = CancellationToken::new();
        let outer = fixture.clock.at(if end == End::OuterDeadline { 830 } else { 2000 });
        let challenge = McpAuthChallenge::default();
        let authorize = fixture.authority.authorize(&fixture.config, &challenge, None,
            &fixture.browser, &cancellation, outer);
        let server = async {
            fixture.discovery(&listener).await;
            let callback = handoff.await.unwrap();
            let socket = drive_callback(&fixture, &callback, &cancellation, end).await;
            if end == End::Success { reply(&listener, json!({"access_token":"access", "token_type":"Bearer"})).await; }
            (callback.address, socket)
        };
        let (result, address, socket) = match select(Box::pin(server), Box::pin(authorize)).await {
            Either::Left(((address, socket), authorize)) => (authorize.await, address, socket),
            Either::Right((early, _)) => panic!("authorization ended before the driven phase: {:?}", early.err()),
        };
        assert_eq!(*fixture.browser.deadlines.lock().unwrap(), [outer, outer]);
        if end == End::Success {
            assert_eq!(result.unwrap().access.bytes(), b"access");
        } else {
            assert!(matches!(result, Err(error) if error == if end == End::Cancelled { McpAuthError::Cancelled } else { McpAuthError::Deadline }));
        }
        {
            let requests = fixture.network.requests.lock().unwrap();
            assert_eq!(requests.len(), if matches!(end, End::Success | End::NetworkDeadline) { 4 } else { 3 });
            for (_, begun, deadline) in requests.iter() { assert_eq!(*deadline, *begun + Duration::from_secs(30)); }
            if requests.len() == 4 { assert_eq!(requests[3].2, fixture.clock.at(1111)); }
        }
        assert!(listener.accept().now_or_never().is_none());
        assert!(TcpStream::connect(address).await.is_err()); // Actual listener cleanup.
        if let Some(mut socket) = socket {
            let mut bytes = Vec::new();
            socket.read_to_end(&mut bytes).await.unwrap(); // Actual accepted socket cleanup.
        }
    });
}

#[test]
fn late_handoff_and_near_cutoff_callback_keep_separate_socket_and_token_budgets() {
    scenario(End::Success);
}
#[test]
fn callback_wait_expires_five_minutes_after_successful_handoff() {
    scenario(End::CallbackDeadline);
}
#[test]
fn caller_overall_deadline_still_shortens_the_post_handoff_wait() {
    scenario(End::OuterDeadline);
}
#[test]
fn accepted_callback_socket_expires_after_its_own_thirty_seconds() {
    scenario(End::SocketDeadline);
}
#[test]
fn token_exchange_retains_its_thirty_second_network_limit() {
    scenario(End::NetworkDeadline);
}
#[test]
fn cancellation_after_handoff_closes_the_listener_without_token_exchange() {
    scenario(End::Cancelled);
}
