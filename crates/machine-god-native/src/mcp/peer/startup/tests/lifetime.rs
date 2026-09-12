use super::*;
use crate::mcp::{lifetime::McpPeerLifetime, stdio::McpStdioConnection};
use std::sync::Mutex;

struct Clock(Mutex<Instant>);
impl McpPeerTimer for Clock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
    fn sleep_until(&self, _: Instant) -> machine_god_core::BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

#[test]
fn selected_stdio_expiry_is_retained_and_cannot_be_widened_after_startup() {
    let now = Instant::now();
    let timer = Arc::new(Clock(Mutex::new(now)));
    let mut peer = unnegotiated(
        McpStdioConnection::inert_for_test(),
        timer.clone(),
        CancellationToken::new(),
    );
    assert_eq!(
        peer.reserve_tool_id().unwrap(),
        crate::mcp::protocol::RpcId::Integer(1)
    );
    peer.discard_tool_id();
    let until = now + Duration::from_secs(5);
    peer.restrict_lifetime(McpPeerLifetime::Until(until));
    peer.restrict_lifetime(McpPeerLifetime::OwnerControlled);
    peer.restrict_lifetime(McpPeerLifetime::Until(until + Duration::from_secs(50)));
    assert_eq!(peer.lifetime, McpPeerLifetime::Until(until));
    assert_eq!(
        peer.lifetime.constrain(until + Duration::from_secs(50)),
        until
    );
    *timer.0.lock().unwrap() = until;
    assert_eq!(peer.reserve_tool_id(), Err(McpPeerError::Deadline));
    assert_eq!(peer.admit_runtimes(vec![]), Err(McpPeerError::Deadline));
    assert_eq!(
        peer.next_id,
        Some(2),
        "expiry precedes request-ID allocation"
    );
    assert!(
        peer.completion().is_complete(),
        "fixture never acquired a process/worker"
    );
}

#[test]
fn expired_stdio_tool_catalog_and_feature_paths_reject_before_transport_effects() {
    use crate::mcp::control::{
        McpFeatureOperationOptions,
        tests::{human, request},
    };
    let now = Instant::now();
    let mut peer = unnegotiated(
        McpStdioConnection::inert_for_test(),
        Arc::new(Clock(Mutex::new(now))),
        CancellationToken::new(),
    );
    peer.restrict_lifetime(McpPeerLifetime::Until(now));
    let later = now + Duration::from_secs(60);
    assert!(matches!(
        futures_executor::block_on(peer.catalog(
            crate::mcp::pagination::McpCatalogKind::Tools,
            crate::mcp::pagination::McpCatalogLimits::default(),
            now,
            later
        )),
        Err(McpPeerError::Deadline)
    ));
    assert!(matches!(
        futures_executor::block_on(peer.feature(
            &request("resource list fixture"),
            "fixture",
            &[],
            human(CancellationToken::new()),
            McpFeatureOperationOptions::new(now),
            later
        )),
        Err(McpPeerError::Deadline)
    ));
    let fixture = crate::mcp::submission::tests::Fixture::new();
    fixture.ready("expiry");
    let submission =
        futures_executor::block_on(fixture.claim("expiry", CancellationToken::new())).unwrap();
    assert!(matches!(
        futures_executor::block_on(peer.call_frame(submission, later)),
        Err(McpPeerError::Deadline)
    ));
    assert_eq!(peer.next_id, Some(1));
    assert!(!peer.closed);
}

#[test]
fn selected_timer_expiry_stops_a_polled_exchange_before_its_writer() {
    let deadline = Instant::now() + Duration::from_secs(60);
    let timer = Clock(Mutex::new(deadline));
    let mut polled = false;
    assert!(matches!(
        futures_executor::block_on(crate::mcp::peer::routing::bounded(
            async {
                polled = true;
            },
            &timer,
            &CancellationToken::new(),
            deadline
        )),
        Err(McpPeerError::Deadline)
    ));
    assert!(!polled);
}
