//! Actual issued leases and localhost HTTP, with no process/session fixtures.
use super::*;
use crate::mcp::{
    auth::McpAuthLease,
    pagination::{McpCatalogKind, McpCatalogLimits},
    runtime::NativeMcpOwnedPeer,
};
use std::{
    sync::Mutex,
    task::{Poll, Waker},
};

struct AuthClock {
    state: Mutex<ClockState>,
}
struct ClockState {
    now: Instant,
    wall: i64,
    waiters: Vec<Waker>,
}
impl AuthClock {
    fn new() -> Self {
        Self {
            state: Mutex::new(ClockState {
                // Intentionally not the startup/HTTP clock domain. The selected
                // lease's clock must fence I/O independently of request deadlines.
                now: Instant::now() + Duration::from_secs(3600),
                wall: 1_000_000,
                waiters: Vec::new(),
            }),
        }
    }
    fn advance(&self, monotonic: Duration, wall_ms: i64) {
        let waiters = {
            let mut state = self.state.lock().unwrap();
            state.now += monotonic;
            state.wall += wall_ms;
            std::mem::take(&mut state.waiters)
        };
        for waiter in waiters {
            waiter.wake();
        }
    }
}
impl McpHttpClock for AuthClock {
    fn now(&self) -> Instant {
        self.state.lock().unwrap().now
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::poll_fn(move |cx| {
            let mut state = self.state.lock().unwrap();
            if state.now >= deadline {
                return Poll::Ready(());
            }
            if !state
                .waiters
                .iter()
                .any(|waker| waker.will_wake(cx.waker()))
            {
                assert!(state.waiters.len() < 32, "bounded selected-clock fixture");
                state.waiters.push(cx.waker().clone());
            }
            Poll::Pending
        }))
    }
}
impl McpAuthClock for AuthClock {
    fn unix_millis(&self) -> i64 {
        self.state.lock().unwrap().wall
    }
}

async fn authenticated(
    address: SocketAddr,
    clock: Arc<AuthClock>,
) -> (NativeMcpStartup, Credentials, Arc<McpAuthLease>) {
    let mut selected = http_options(address);
    selected.peer_lifetime = McpPeerLifetime::OwnerControlled;
    let credentials = Credentials::with_clock(
        selected.network.as_ref().unwrap().clone(),
        selected.workers.clone(),
        clock.clone(),
    );
    let crate::mcp::config::McpTransportConfig::Http(remote) =
        selected.configuration.server("remote").unwrap().transport()
    else {
        panic!("HTTP fixture")
    };
    let lookup = |name: &str| (name == "TOKEN").then_some(&b"captured-secret"[..]);
    let headers = McpResolvedHeaders::resolve(remote, lookup, Some(b""), &[]).unwrap();
    let config =
        McpAuthConfig::new(remote, &headers.authentication_identity_bytes(), lookup).unwrap();
    credentials.seed(&config, 1_120_000);
    let lease = Arc::new(
        credentials
            .service
            .access_token(
                config.identity(),
                &CancellationToken::new(),
                clock.now() + Duration::from_secs(5),
            )
            .await
            .unwrap(),
    );
    selected
        .authentication
        .push(authentication(NativeMcpStartupAuthSource::Lease(
            lease.clone(),
        )));
    (NativeMcpStartup::new(selected).unwrap(), credentials, lease)
}

async fn ready(startup: &NativeMcpStartup, listener: &TcpListener) -> NativeMcpStartupBatch {
    let (batch, received) = join(
        startup.build_configured(NativeMcpStartupPhase::All, CancellationToken::new()),
        reply(listener, &discover(false)),
    )
    .await;
    assert!(String::from_utf8_lossy(&received).contains("authorization: Bearer old-secret"));
    assert!(batch.receipt().required_ready());
    batch
}

#[test]
fn selected_lease_refresh_skew_and_expired_tombstone_survive_peer_cleanup() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = Arc::new(AuthClock::new());
        let (startup, _credentials, lease) =
            authenticated(listener.local_addr().unwrap(), clock.clone()).await;
        let batch = ready(&startup, &listener).await;
        assert!(!startup.authentication_refresh_due().unwrap());
        clock.advance(Duration::from_secs(59), 0);
        assert!(!startup.authentication_refresh_due().unwrap());
        clock.advance(Duration::from_secs(1), 0);
        assert!(startup.authentication_refresh_due().unwrap());
        drop(batch);
        assert!(startup.cleanup_observations().is_empty());
        clock.advance(Duration::from_secs(60), -60_000);
        assert!(
            lease.access_token().is_err(),
            "wall rollback cannot extend issued expiry"
        );
        assert!(!lease.generation().is_cancelled());
        assert!(startup.authentication_refresh_due().unwrap());
        assert_eq!(startup.authentication_leases.lock().unwrap().len(), 1);
        assert_eq!(
            startup.authentication_slot("remote"),
            Err(NativeMcpStartupError::Unavailable)
        );
    });
}

#[test]
fn duplicate_live_selection_is_inert_and_fresh_closed_candidates_can_be_pruned() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (startup, _credentials, _) =
            authenticated(listener.local_addr().unwrap(), Arc::new(AuthClock::new())).await;
        let batch = ready(&startup, &listener).await;
        let runtime = runtime();
        let (candidate, _) = batch
            .prepare(&runtime, &[], NativeMcpStartupRequirement::Required)
            .unwrap();
        let duplicate = startup
            .build_configured(NativeMcpStartupPhase::All, CancellationToken::new())
            .await;
        assert_eq!(
            duplicate.receipt().servers[0].state,
            NativeMcpStartupState::Failed(NativeMcpStartupError::Unavailable)
        );
        assert_eq!(
            startup.authentication_slot("remote"),
            Err(NativeMcpStartupError::Unavailable)
        );
        assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
        drop(duplicate);
        drop(candidate);
        assert!(!startup.authentication_refresh_due().unwrap());
        assert!(startup.authentication_leases.lock().unwrap().is_empty());
        assert!(startup.authentication_slot("remote").is_ok());
        drop(ready(&startup, &listener).await);
    });
}

#[test]
fn failed_discovery_never_registers_lease_and_original_auth_timer_stops_blocked_io() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let clock = Arc::new(AuthClock::new());
        let (startup, _credentials, lease) =
            authenticated(listener.local_addr().unwrap(), clock.clone()).await;
        let server = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let bytes = request(&mut socket).await;
            assert!(String::from_utf8_lossy(&bytes).contains("authorization: Bearer old-secret"));
            clock.advance(Duration::from_secs(120), 0);
            let mut byte = [0];
            match socket.read(&mut byte).await {
                Ok(0) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                    ) => {}
                other => panic!("owned expiry must close the pending response: {other:?}"),
            }
        };
        let (batch, ()) = join(
            startup.build_configured(NativeMcpStartupPhase::All, CancellationToken::new()),
            server,
        )
        .await;
        assert!(!batch.receipt().required_ready());
        assert!(batch.receipt().cleanup_complete());
        assert!(!lease.generation().is_cancelled());
        assert!(startup.authentication_leases.lock().unwrap().is_empty());
        assert!(!startup.authentication_refresh_due().unwrap());
    });
}

#[test]
fn queued_peer_catalog_does_not_write_after_wall_expiry_or_generation_revocation() {
    for revoke in [false, true] {
        run(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let clock = Arc::new(AuthClock::new());
            let (startup, _credentials, lease) =
                authenticated(listener.local_addr().unwrap(), clock.clone()).await;
            let mut batch = ready(&startup, &listener).await;
            let NativeMcpOwnedPeer::Http(peer) = &mut batch.servers[0].peer else {
                panic!("HTTP peer")
            };
            let readiness = peer.readiness();
            let epoch = startup.catalog_epoch;
            let operation = peer.catalog(
                McpCatalogKind::Resources,
                McpCatalogLimits::default(),
                epoch,
                deadline(),
            );
            if revoke {
                lease.generation().cancel();
            } else {
                clock.advance(Duration::ZERO, 120_000);
            }
            assert!(operation.await.is_err());
            assert!(!readiness.is_ready());
            assert!(futures_util::poll!(Box::pin(listener.accept())).is_pending());
            assert_eq!(
                startup.authentication_refresh_due(),
                if revoke {
                    Err(NativeMcpStartupError::Authentication)
                } else {
                    Ok(true)
                }
            );
            drop(batch);
            assert_eq!(
                startup.authentication_refresh_due(),
                if revoke {
                    Err(NativeMcpStartupError::Authentication)
                } else {
                    Ok(true)
                }
            );
        });
    }
}
