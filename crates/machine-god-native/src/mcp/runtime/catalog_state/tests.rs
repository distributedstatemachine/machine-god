use super::*;
use crate::mcp::{
    catalog::{McpDescriptor, McpDescriptorLimits},
    pagination::{McpCatalogBuilder, McpCatalogLimits},
    protocol::ProtocolVersion,
};

fn catalog(kind: McpCatalogKind, now: u64, ttl: u64) -> McpDescriptorCatalog {
    let field = match kind {
        McpCatalogKind::Tools => "tools",
        McpCatalogKind::Resources => "resources",
        McpCatalogKind::ResourceTemplates => "resourceTemplates",
        McpCatalogKind::Prompts => "prompts",
    };
    let items = if kind == McpCatalogKind::Resources {
        r#"[{"uri":"test://fixed","name":"fixed"}]"#
    } else {
        "[]"
    };
    let mut builder =
        McpCatalogBuilder::new(kind, ProtocolVersion::Modern, McpCatalogLimits::default()).unwrap();
    builder.append_response(format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","{field}":{items},"ttlMs":{ttl}}}}}"#).as_bytes(), &RpcId::Integer(1), None, now).unwrap();
    McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default()).unwrap()
}
fn ticket(state: &mut NativeMcpCatalogState, kind: McpCatalogKind, now: u64) -> McpRefreshTicket {
    let McpRefreshDecision::Refresh(ticket) = state.begin(kind, now).unwrap() else {
        panic!("expected refresh")
    };
    ticket
}

#[test]
fn initial_handoff_requires_exact_allocations_and_unique_families() {
    let first = catalog(McpCatalogKind::Tools, 0, 0);
    let state = NativeMcpCatalogState::new(std::slice::from_ref(&first)).unwrap();
    assert!(state.initial_matches(std::slice::from_ref(&first)));
    assert!(!state.initial_matches(&[catalog(McpCatalogKind::Tools, 0, 0)]));
    assert!(NativeMcpCatalogState::new(&[first.clone(), first]).is_err());
}

#[test]
fn unchanged_tools_refreshes_metadata_without_retaining_an_extra_payload() {
    let original = catalog(McpCatalogKind::Tools, 0, 0);
    let mut state = NativeMcpCatalogState::new(std::slice::from_ref(&original)).unwrap();
    let fresh = catalog(McpCatalogKind::Tools, 1, 10);
    let pending = ticket(&mut state, McpCatalogKind::Tools, 1);
    state.finish_tools(pending, &fresh, 1, false).unwrap();
    assert!(
        state
            .cached(McpCatalogKind::Tools)
            .unwrap()
            .same_allocation(&original)
    );
    assert!(matches!(
        state.begin(McpCatalogKind::Tools, 10).unwrap(),
        McpRefreshDecision::Hit
    ));
    assert!(matches!(
        state.begin(McpCatalogKind::Tools, 11).unwrap(),
        McpRefreshDecision::Refresh(_)
    ));
}

#[test]
fn shared_feature_pressure_preserves_old_data_backoff_and_releases_on_drop() {
    let fresh = catalog(McpCatalogKind::Resources, 0, 0);
    let charge = fresh.retained_byte_charge();
    assert!(charge > 0);
    let budget = Arc::new(FeatureCacheBudget::new(charge));
    let mut first = NativeMcpCatalogState::new(&[]).unwrap();
    first.bind_budget(budget.clone());
    let pending = ticket(&mut first, McpCatalogKind::Resources, 0);
    assert!(first.finish(pending, &fresh, 0).unwrap());
    assert_eq!(budget.bytes.load(Ordering::Acquire), charge);
    let mut second = NativeMcpCatalogState::new(&[]).unwrap();
    second.bind_budget(budget.clone());
    let pending = ticket(&mut second, McpCatalogKind::Resources, 0);
    assert!(!second.finish(pending, &fresh, 0).unwrap());
    assert!(second.cached(McpCatalogKind::Resources).is_none());
    let pending = ticket(&mut first, McpCatalogKind::Resources, 0);
    assert!(!first.finish(pending, &fresh, 0).unwrap());
    assert!(
        first
            .cached(McpCatalogKind::Resources)
            .unwrap()
            .same_allocation(&fresh)
    );
    assert!(matches!(
        first.begin(McpCatalogKind::Resources, 99).unwrap(),
        McpRefreshDecision::RetryLater {
            may_serve_snapshot: true
        }
    ));
    drop(first);
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
    let fresh = catalog(McpCatalogKind::Resources, 100, 10);
    let pending = ticket(&mut second, McpCatalogKind::Resources, 100);
    assert!(second.finish(pending, &fresh, 100).unwrap());
    drop(second);
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
}

#[test]
fn shared_lazy_cache_rejects_payload_only_budget_and_releases_complete_charges() {
    let fresh = catalog(McpCatalogKind::Resources, 0, 0);
    let McpDescriptor::Resource(resource) = &fresh.descriptors()[0] else {
        panic!("expected resource");
    };
    let payload_charge = resource.raw_json().get().len() * 4;
    let budget = Arc::new(FeatureCacheBudget::new(payload_charge));
    let mut state = NativeMcpCatalogState::new(&[]).unwrap();
    state.bind_budget(budget.clone());
    let pending = ticket(&mut state, McpCatalogKind::Resources, 0);
    assert!(!state.finish(pending, &fresh, 0).unwrap());
    assert!(state.cached(McpCatalogKind::Resources).is_none());
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
    // Cache pressure does not consume the independently admitted fresh response.
    assert_eq!(resource.uri(), "test://fixed");
    assert!(matches!(
        state.begin(McpCatalogKind::Resources, 99).unwrap(),
        McpRefreshDecision::RetryLater {
            may_serve_snapshot: false
        }
    ));

    let charge = fresh.retained_byte_charge();
    let budget = Arc::new(FeatureCacheBudget::new(charge));
    let mut first = NativeMcpCatalogState::new(&[]).unwrap();
    let mut second = NativeMcpCatalogState::new(&[]).unwrap();
    first.bind_budget(budget.clone());
    second.bind_budget(budget.clone());
    let pending = ticket(&mut first, McpCatalogKind::Resources, 0);
    assert!(first.finish(pending, &fresh, 0).unwrap());
    let pending = ticket(&mut second, McpCatalogKind::Resources, 0);
    assert!(!second.finish(pending, &fresh, 0).unwrap());
    assert_eq!(budget.bytes.load(Ordering::Acquire), charge);
    drop(first);
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
    let fresh = catalog(McpCatalogKind::Resources, 100, 10);
    let pending = ticket(&mut second, McpCatalogKind::Resources, 100);
    assert!(second.finish(pending, &fresh, 100).unwrap());
    assert_eq!(budget.bytes.load(Ordering::Acquire), charge);
    drop(second);
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
}
