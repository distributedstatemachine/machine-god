use super::*;
use crate::mcp::{
    catalog_refresh::{McpRefreshDecision, McpRefreshGeneration},
    pagination::McpCatalogKind,
    peer::McpPeerCapabilities,
    protocol::{ProtocolVersion, WireLimits, parse_envelope},
};
use serde_json::{Value, json};

#[cfg(feature = "mcp-http")]
mod http;

fn envelope(value: Value) -> RpcEnvelope {
    parse_envelope(&serde_json::to_vec(&value).unwrap(), WireLimits::default()).unwrap()
}

fn capabilities() -> McpPeerCapabilities {
    McpPeerCapabilities::admit(
        &envelope(json!({"jsonrpc":"2.0","id":1,"result":{
            "resultType":"complete","capabilities":{
                "resources":{"listChanged":true,"subscribe":true},"prompts":{"listChanged":true}
            }
        }})),
        ProtocolVersion::Modern,
    )
    .unwrap()
}

fn ack(id: i64, filters: Value) -> RpcEnvelope {
    envelope(
        json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged",
        "params":{"_meta":{"io.modelcontextprotocol/subscriptionId":id},"notifications":filters}}),
    )
}

fn selected() -> NativeMcpCatalogState {
    let mut state = NativeMcpCatalogState::new(&[]).unwrap();
    state
        .install(
            RpcId::Integer(2),
            McpSubscriptionFilters::new(capabilities(), &[]).unwrap(),
        )
        .unwrap();
    state
}

#[test]
fn terminal_policy_expires_results_once_and_rejects_old_listener_end() {
    let mut state = selected();
    let generation = state.generation.clone();
    assert_eq!(
        state.policy.result_cache_invalidation(&generation).unwrap(),
        (0, 0)
    );
    assert!(
        state
            .policy
            .end_subscription(&McpRefreshGeneration::new(), 2)
            .is_err()
    );
    assert!(!state.policy.end_subscription(&generation, 1).unwrap());
    state.ended().unwrap();
    assert_eq!(
        state.policy.result_cache_invalidation(&generation).unwrap(),
        (1, 1)
    );
    state.ended().unwrap();
    assert_eq!(
        state.policy.result_cache_invalidation(&generation).unwrap(),
        (1, 1)
    );
    state
        .install(
            RpcId::Integer(3),
            McpSubscriptionFilters::new(capabilities(), &[]).unwrap(),
        )
        .unwrap();
    let before = state.policy.result_cache_invalidation(&generation).unwrap();
    assert!(!state.policy.end_subscription(&generation, 2).unwrap());
    assert_eq!(
        state.policy.result_cache_invalidation(&generation).unwrap(),
        before
    );
    assert_eq!(state.subscription, Some(RpcId::Integer(3)));
}

#[test]
fn unsupported_or_cancelled_listener_stops_without_admitting_late_ack() {
    for terminal in [
        ack(2, json!({"resourcesListChanged":true})),
        envelope(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}),
        ),
    ] {
        let mut state = selected();
        assert!(state.observe(&terminal).unwrap());
        assert!(state.subscription_stopped);
        assert!(state.subscription.is_none());
        assert!(!state.acknowledged);
        assert_eq!(
            state
                .policy
                .result_cache_invalidation(&state.generation)
                .unwrap(),
            (1, 1)
        );
        assert!(
            !state
                .observe(&ack(
                    2,
                    json!({"resourcesListChanged":true,"promptsListChanged":true})
                ))
                .unwrap()
        );
        assert!(!state.acknowledged);
        assert!(matches!(
            state.begin(McpCatalogKind::Prompts, 0).unwrap(),
            McpRefreshDecision::Refresh(_)
        ));
    }
}

#[test]
fn result_epochs_keep_prompt_and_resource_invalidations_separate() {
    let mut state = selected();
    state
        .observe(&ack(
            2,
            json!({"resourcesListChanged":true,"promptsListChanged":true}),
        ))
        .unwrap();
    for (method, expected) in [
        ("notifications/prompts/list_changed", (0, 1)),
        ("notifications/resources/list_changed", (1, 1)),
    ] {
        state
            .observe(&envelope(json!({"jsonrpc":"2.0","method":method,"params":{
                "_meta":{"io.modelcontextprotocol/subscriptionId":2}
            }})))
            .unwrap();
        assert_eq!(
            state
                .policy
                .result_cache_invalidation(&state.generation)
                .unwrap(),
            expected
        );
    }
}
