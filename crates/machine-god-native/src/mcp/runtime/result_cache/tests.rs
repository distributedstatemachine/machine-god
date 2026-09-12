use super::*;
use crate::mcp::{
    catalog::McpDescriptorLimits,
    commands::McpCommand,
    feature::{McpFeatureCodecLimits, McpFeatureExchange, McpFeatureExchangeOptions},
    pagination::{McpCatalogBuilder, McpCatalogKind, McpCatalogLimits},
    peer::McpPeerCapabilities,
    protocol::{
        NegotiatedProtocol, ProtocolVersion, RpcId, TransportKind, WireLimits, parse_envelope,
    },
};
use serde_json::json;
use std::time::Duration;

fn request(command: &str) -> McpFeatureRequest {
    let McpCommand::Feature(command) = command.parse::<McpCommand>().unwrap() else {
        panic!("feature")
    };
    McpFeatureRequest::try_from(command).unwrap()
}
fn response(result: serde_json::Value, now: Instant) -> McpFeatureResponse {
    let init = parse_envelope(br#"{"jsonrpc":"2.0","id":0,"result":{"resultType":"complete","capabilities":{"resources":{}}}}"#, WireLimits::default()).unwrap();
    let caps = McpPeerCapabilities::admit(&init, ProtocolVersion::Modern).unwrap();
    let mut builder = McpCatalogBuilder::new(
        McpCatalogKind::Resources,
        ProtocolVersion::Modern,
        McpCatalogLimits::default(),
    )
    .unwrap();
    builder
        .append_response(br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","resources":[{"uri":"test://fixed","name":"fixed"}]}}"#, &RpcId::Integer(1), None, 0)
        .unwrap();
    let catalog =
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap();
    let exchange = McpFeatureExchange::prepare(
        &request("resource read fixture test://fixed"),
        "fixture",
        &[catalog],
        McpFeatureExchangeOptions::new(
            NegotiatedProtocol {
                transport: TransportKind::Stdio,
                version: ProtocolVersion::Modern,
            },
            1,
            caps,
        )
        .unwrap(),
        None,
        McpFeatureCodecLimits::default(),
    )
    .unwrap();
    exchange
        .admit_response(
            &serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"result":result})).unwrap(),
        )
        .unwrap()
        .observed_at(now)
}
fn complete(ttl: u64, now: Instant) -> McpFeatureResponse {
    response(
        json!({"resultType":"complete","contents":[{"uri":"test://fixed","text":"retained"}],"ttlMs":ttl}),
        now,
    )
}
fn ticket(cache: &mut FeatureResultCache, key: &str, at: u64) -> Ticket {
    let Lookup::Fetch(ticket) = cache
        .begin(key.into(), Action::ResourceRead, (0, 0), at)
        .unwrap()
    else {
        panic!("fetch")
    };
    ticket
}
fn cache(max: usize) -> FeatureResultCache {
    FeatureResultCache::new(Arc::new(FeatureCacheBudget::new(max)))
}

#[test]
fn hits_keep_original_expiry_and_never_mint_continuations() {
    let epoch = Instant::now();
    let mut cache = cache(1_000_000);
    let fetch = ticket(&mut cache, "one", 0);
    cache
        .finish(fetch, &complete(100, epoch), (0, 0), epoch, 0)
        .unwrap();
    for time in [0, 50, 99] {
        let Lookup::Hit(response) = cache
            .begin("one".into(), Action::ResourceRead, (0, 0), time)
            .unwrap()
        else {
            panic!("hit")
        };
        assert_eq!(response.received_at(), Some(epoch));
        assert!(
            crate::mcp::control::McpFeatureRound::cached(response)
                .unwrap()
                .input()
                .is_none()
        );
    }
    assert!(matches!(
        cache
            .begin("one".into(), Action::ResourceRead, (0, 0), 100)
            .unwrap(),
        Lookup::Fetch(_)
    ));
}

#[test]
fn delayed_completion_cannot_rebase_ttl_or_overwrite_newer_fetch() {
    let epoch = Instant::now();
    let mut cache = cache(1_000_000);
    let old = ticket(&mut cache, "one", 0);
    let current = ticket(&mut cache, "two", 0);
    cache
        .finish(old, &complete(100, epoch), (0, 0), epoch, 0)
        .unwrap();
    assert!(cache.entries.is_empty());
    cache
        .finish(current, &complete(100, epoch), (0, 0), epoch, 100)
        .unwrap();
    assert!(cache.entries.is_empty());
}

#[test]
fn notifications_retire_cached_data_and_pending_candidates() {
    let epoch = Instant::now();
    let mut cache = cache(1_000_000);
    let old = ticket(&mut cache, "one", 0);
    cache
        .finish(old, &complete(100, epoch), (1, 0), epoch, 0)
        .unwrap();
    assert!(cache.entries.is_empty());
    let fetch = ticket(&mut cache, "one", 0);
    cache
        .finish(fetch, &complete(100, epoch), (0, 0), epoch, 0)
        .unwrap();
    assert_eq!(cache.entries.len(), 1);
    assert!(matches!(
        cache
            .begin("one".into(), Action::ResourceRead, (1, 0), 1)
            .unwrap(),
        Lookup::Fetch(_)
    ));
    assert!(cache.entries.is_empty());
}

#[test]
fn bounded_fifo_and_shared_budget_release_on_drop() {
    let epoch = Instant::now();
    let budget = Arc::new(FeatureCacheBudget::new(1_000_000));
    let mut cache = FeatureResultCache::new(budget.clone());
    for number in 0..65 {
        let fetch = ticket(&mut cache, &number.to_string(), 0);
        cache
            .finish(fetch, &complete(100, epoch), (0, 0), epoch, 0)
            .unwrap();
    }
    assert_eq!(cache.entries.len(), MAX_ENTRIES);
    assert_eq!(cache.entries.front().unwrap().key.as_ref(), "1");
    assert!(budget.reserve(1_000_000).is_none());
    drop(cache);
    assert!(budget.reserve(1_000_000).is_some());
    let mut pressure = FeatureResultCache::new(budget.clone());
    let held = budget.reserve(1_000_000).unwrap();
    let fetch = ticket(&mut pressure, "one", 0);
    pressure
        .finish(fetch, &complete(100, epoch), (0, 0), epoch, 0)
        .unwrap();
    assert!(pressure.entries.is_empty());
    drop(held);
}

#[test]
fn zero_ttl_unresolved_input_and_clock_regression_are_not_cache_hits() {
    let epoch = Instant::now();
    let mut cache = cache(1_000_000);
    for reply in [
        complete(0, epoch),
        response(json!({"resultType":"input_required","ttlMs":100}), epoch),
    ] {
        let fetch = ticket(&mut cache, "one", 0);
        cache.finish(fetch, &reply, (0, 0), epoch, 0).unwrap();
        assert!(cache.entries.is_empty());
    }
    let fetch = ticket(&mut cache, "one", 10);
    cache
        .finish(
            fetch,
            &complete(100, epoch + Duration::from_millis(10)),
            (0, 0),
            epoch,
            10,
        )
        .unwrap();
    assert!(
        cache
            .begin("one".into(), Action::ResourceRead, (0, 0), 9)
            .is_err()
    );
    assert!(
        cache
            .begin("one".into(), Action::ResourceRead, (0, 0), 11)
            .is_err()
    );
}
