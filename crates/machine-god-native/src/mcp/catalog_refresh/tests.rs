use super::*;
use crate::mcp::{
    catalog::McpDescriptorLimits,
    pagination::{McpCatalogBuilder, McpCatalogLimits},
    peer::McpPeerCapabilities,
    protocol::{ProtocolVersion, RpcEnvelope, RpcId, WireLimits, parse_envelope},
};
use serde_json::{Value, json};

mod notification;

fn catalog(kind: McpCatalogKind, received: u64, ttl: &str) -> McpDescriptorCatalog {
    let field = match kind {
        McpCatalogKind::Tools => "tools",
        McpCatalogKind::Resources => "resources",
        McpCatalogKind::ResourceTemplates => "resourceTemplates",
        McpCatalogKind::Prompts => "prompts",
    };
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","{field}":[]{ttl}}}}}"#
    );
    let mut builder =
        McpCatalogBuilder::new(kind, ProtocolVersion::Modern, McpCatalogLimits::default()).unwrap();
    builder
        .append_response(body.as_bytes(), &RpcId::Integer(1), None, received)
        .unwrap();
    McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default()).unwrap()
}
fn envelope(value: &Value) -> RpcEnvelope {
    parse_envelope(&serde_json::to_vec(value).unwrap(), WireLimits::default()).unwrap()
}
fn capabilities() -> McpPeerCapabilities {
    McpPeerCapabilities::admit(
        &envelope(&json!({
            "jsonrpc":"2.0","id":1,"result":{"resultType":"complete","capabilities":{
                "tools":{"listChanged":true},
                "resources":{"listChanged":true,"subscribe":true},
                "prompts":{"listChanged":true}
            }}
        })),
        ProtocolVersion::Modern,
    )
    .unwrap()
}
fn ticket(
    policy: &mut McpCatalogRefresh,
    generation: &McpRefreshGeneration,
    kind: McpCatalogKind,
    now: u64,
) -> McpRefreshTicket {
    let McpRefreshDecision::Refresh(ticket) = policy.begin(generation, kind, now).unwrap() else {
        panic!("expected refresh");
    };
    ticket
}

#[test]
fn admitted_modern_ttl_is_immediate_when_missing_zero_or_negative() {
    for ttl in ["", r#", "ttlMs":0"#, r#", "ttlMs":-1"#] {
        for kind in [
            McpCatalogKind::Tools,
            McpCatalogKind::Resources,
            McpCatalogKind::ResourceTemplates,
            McpCatalogKind::Prompts,
        ] {
            let generation = McpRefreshGeneration::new();
            let mut policy =
                McpCatalogRefresh::new(generation.clone(), &[catalog(kind, 20, ttl)]).unwrap();
            assert!(ticket(&mut policy, &generation, kind, 20).may_serve_snapshot);
        }
    }
}

#[test]
fn expiry_boundary_and_missing_family_have_distinct_serving_evidence() {
    let generation = McpRefreshGeneration::new();
    let mut policy = McpCatalogRefresh::new(
        generation.clone(),
        &[catalog(McpCatalogKind::Tools, 20, r#", "ttlMs":10"#)],
    )
    .unwrap();
    assert!(matches!(
        policy
            .begin(&generation, McpCatalogKind::Tools, 29)
            .unwrap(),
        McpRefreshDecision::Hit
    ));
    let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, 30);
    assert!(pending.may_serve_snapshot);
    assert!(matches!(
        policy
            .begin(&generation, McpCatalogKind::Tools, 30)
            .unwrap(),
        McpRefreshDecision::AlreadyRefreshing {
            may_serve_snapshot: true
        }
    ));
    let missing = ticket(&mut policy, &generation, McpCatalogKind::Prompts, 30);
    assert!(!missing.may_serve_snapshot);
    assert!(matches!(
        policy
            .begin(&generation, McpCatalogKind::Prompts, 30)
            .unwrap(),
        McpRefreshDecision::AlreadyRefreshing {
            may_serve_snapshot: false
        }
    ));
}

#[test]
fn abandoned_ticket_releases_only_its_own_family_without_callbacks() {
    let generation = McpRefreshGeneration::new();
    let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
    let tools = ticket(&mut policy, &generation, McpCatalogKind::Tools, 0);
    let prompts = ticket(&mut policy, &generation, McpCatalogKind::Prompts, 0);
    drop(tools);
    let replacement = ticket(&mut policy, &generation, McpCatalogKind::Tools, 1);
    assert!(matches!(
        policy
            .begin(&generation, McpCatalogKind::Prompts, 1)
            .unwrap(),
        McpRefreshDecision::AlreadyRefreshing { .. }
    ));
    drop((prompts, replacement));
    assert!(matches!(
        policy
            .begin(&generation, McpCatalogKind::Prompts, 1)
            .unwrap(),
        McpRefreshDecision::Refresh(_)
    ));
}

#[test]
fn failure_backoff_is_bounded_and_success_resets_it() {
    let generation = McpRefreshGeneration::new();
    let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
    let mut now = 0;
    for delay in [100, 200, 400, 800, 1600, 3200, 5000, 5000, 5000, 5000] {
        let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, now);
        policy.fail(pending, now).unwrap();
        assert!(matches!(
            policy
                .begin(&generation, McpCatalogKind::Tools, now + delay - 1)
                .unwrap(),
            McpRefreshDecision::RetryLater {
                may_serve_snapshot: false
            }
        ));
        now += delay;
    }
    let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, now);
    policy
        .finish(pending, &catalog(McpCatalogKind::Tools, now, ""), now)
        .unwrap();
    let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, now);
    policy.fail(pending, now).unwrap();
    assert_eq!(policy.families[0].retry_at, Some(now + 100));
    assert!(matches!(
        policy
            .begin(&generation, McpCatalogKind::Tools, now + 99)
            .unwrap(),
        McpRefreshDecision::RetryLater {
            may_serve_snapshot: true
        }
    ));
}

#[test]
fn foreign_generations_and_same_generation_foreign_tickets_cannot_settle() {
    let generation = McpRefreshGeneration::new();
    let foreign = McpRefreshGeneration::new();
    let mut first = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
    let mut second = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
    assert!(matches!(
        first.begin(&foreign, McpCatalogKind::Tools, 100),
        Err(McpRefreshError::Foreign)
    ));
    let first_ticket = ticket(&mut first, &generation, McpCatalogKind::Tools, 0);
    let second_ticket = ticket(&mut second, &generation, McpCatalogKind::Tools, 0);
    assert_eq!(second.fail(first_ticket, 0), Err(McpRefreshError::Foreign));
    assert!(matches!(
        second.begin(&generation, McpCatalogKind::Tools, 0).unwrap(),
        McpRefreshDecision::AlreadyRefreshing { .. }
    ));
    second.fail(second_ticket, 0).unwrap();
    assert!(matches!(
        first.begin(&generation, McpCatalogKind::Tools, 0).unwrap(),
        McpRefreshDecision::Refresh(_)
    ));
}

#[test]
fn wrong_family_or_timestamp_never_replaces_retained_metadata() {
    for candidate in [
        catalog(McpCatalogKind::Prompts, 10, ""),
        catalog(McpCatalogKind::Tools, 9, ""),
        catalog(McpCatalogKind::Tools, 12, ""),
    ] {
        let generation = McpRefreshGeneration::new();
        let mut policy = McpCatalogRefresh::new(
            generation.clone(),
            &[catalog(McpCatalogKind::Tools, 10, "")],
        )
        .unwrap();
        let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, 11);
        assert_eq!(
            policy.finish(pending, &candidate, 11),
            Err(McpRefreshError::Invalid)
        );
        assert_eq!(policy.families[0].times.unwrap().fetched, 10);
        assert!(matches!(
            policy
                .begin(&generation, McpCatalogKind::Tools, 11)
                .unwrap(),
            McpRefreshDecision::Refresh(_)
        ));
    }
}

#[test]
fn clock_regression_and_retry_overflow_close_without_wrapping() {
    let generation = McpRefreshGeneration::new();
    let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
    drop(ticket(&mut policy, &generation, McpCatalogKind::Tools, 10));
    assert!(matches!(
        policy.begin(&generation, McpCatalogKind::Tools, 9),
        Err(McpRefreshError::ClockRegression)
    ));
    assert!(matches!(
        policy.begin(&generation, McpCatalogKind::Tools, 10),
        Err(McpRefreshError::Closed)
    ));
    let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
    let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, u64::MAX);
    assert_eq!(
        policy.fail(pending, u64::MAX),
        Err(McpRefreshError::Exhausted)
    );
    assert!(matches!(
        policy.begin(&generation, McpCatalogKind::Tools, u64::MAX),
        Err(McpRefreshError::Closed)
    ));
}

#[test]
fn duplicate_or_excess_catalog_families_are_rejected_without_retention() {
    let generation = McpRefreshGeneration::new();
    let catalog = catalog(McpCatalogKind::Tools, 0, "");
    assert!(
        McpCatalogRefresh::new(generation.clone(), &[catalog.clone(), catalog.clone()]).is_err()
    );
    assert!(McpCatalogRefresh::new(generation, &vec![catalog; 5]).is_err());
}

#[test]
fn replacement_prevalidation_is_inert_for_foreign_ticket_family_and_time() {
    let generation = McpRefreshGeneration::new();
    let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
    let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, 10);
    let tools = catalog(McpCatalogKind::Tools, 10, "");
    for foreign_generation in [generation.clone(), McpRefreshGeneration::new()] {
        let mut other = McpCatalogRefresh::new(foreign_generation.clone(), &[]).unwrap();
        let other_ticket = ticket(&mut other, &foreign_generation, McpCatalogKind::Tools, 10);
        assert_eq!(
            other.validate_replacement(&pending, &tools, 11),
            Err(McpRefreshError::Foreign)
        );
        assert_eq!(other.last_now, 10);
        assert!(!other.closed);
        assert!(matches!(
            other
                .begin(&foreign_generation, McpCatalogKind::Tools, 10)
                .unwrap(),
            McpRefreshDecision::AlreadyRefreshing { .. }
        ));
        drop(other_ticket);
    }
    let prompts = catalog(McpCatalogKind::Prompts, 10, "");
    assert_eq!(
        policy.validate_replacement(&pending, &prompts, 11),
        Err(McpRefreshError::Invalid)
    );
    assert_eq!(
        policy.validate_replacement(&pending, &tools, 9),
        Err(McpRefreshError::ClockRegression)
    );
    assert_eq!(policy.last_now, 10);
    assert!(!policy.closed);
    policy.validate_replacement(&pending, &tools, 11).unwrap();
    assert_eq!(policy.last_now, 10);
    policy.finish(pending, &tools, 11).unwrap();
    assert_eq!(policy.last_now, 11);
}
