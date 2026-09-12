use super::*;

fn notification(method: &str, id: i64, extra: Value) -> RpcEnvelope {
    let mut params = extra.as_object().unwrap().clone();
    params.insert(
        "_meta".into(),
        json!({"io.modelcontextprotocol/subscriptionId":id}),
    );
    envelope(&json!({"jsonrpc":"2.0", "method":method,"params":params}))
}
fn installed(uris: &[&str]) -> (McpCatalogRefresh, McpRefreshGeneration) {
    let generation = McpRefreshGeneration::new();
    let catalogs: Vec<_> = [
        McpCatalogKind::Tools,
        McpCatalogKind::Resources,
        McpCatalogKind::ResourceTemplates,
        McpCatalogKind::Prompts,
    ]
    .into_iter()
    .map(|kind| catalog(kind, 0, r#", "ttlMs":1000"#))
    .collect();
    let mut policy = McpCatalogRefresh::new(generation.clone(), &catalogs).unwrap();
    let filters = McpSubscriptionFilters::new(capabilities(), uris).unwrap();
    let ack = notification(
        "notifications/subscriptions/acknowledged",
        5,
        json!({"notifications":serde_json::to_value(&filters).unwrap()}),
    );
    policy
        .install_subscription(&generation, 5, filters)
        .unwrap();
    assert_eq!(
        policy.observe(&generation, &ack).unwrap(),
        McpRefreshNotification::Acknowledged
    );
    (policy, generation)
}

#[test]
fn modern_filters_serialize_exactly_and_reject_unbounded_or_unadvertised_uris() {
    let filters =
        McpSubscriptionFilters::new(capabilities(), &["custom://B", "custom://a"]).unwrap();
    assert_eq!(
        serde_json::to_value(filters).unwrap(),
        json!({"toolsListChanged":true,
        "resourcesListChanged":true,"promptsListChanged":true,
        "resourceSubscriptions":["custom://B","custom://a"]})
    );
    for uris in [&[""][..], &["one", "one"], &["x"; 65]] {
        assert!(McpSubscriptionFilters::new(capabilities(), uris).is_err());
    }
    let maximum = "x".repeat(64 * 1024);
    assert!(McpSubscriptionFilters::new(capabilities(), &[&maximum]).is_ok());
    assert!(McpSubscriptionFilters::new(capabilities(), &[&maximum, "x"]).is_err());
    assert!(McpSubscriptionFilters::new(McpPeerCapabilities::default(), &["x"]).is_err());
}

#[test]
fn no_invalidation_before_exact_ack_and_only_notifications_are_admitted() {
    let generation = McpRefreshGeneration::new();
    let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
    let filters = McpSubscriptionFilters::new(capabilities(), &[]).unwrap();
    let expected = serde_json::to_value(&filters).unwrap();
    policy
        .install_subscription(&generation, 5, filters)
        .unwrap();
    let changed = notification("notifications/tools/list_changed", 5, json!({}));
    assert_eq!(
        policy.observe(&generation, &changed).unwrap(),
        McpRefreshNotification::Ignored
    );
    let late = notification(
        "notifications/subscriptions/acknowledged",
        4,
        json!({"notifications":expected}),
    );
    assert_eq!(
        policy.observe(&generation, &late).unwrap(),
        McpRefreshNotification::Ignored
    );
    let ack = notification(
        "notifications/subscriptions/acknowledged",
        5,
        json!({"notifications":expected}),
    );
    assert_eq!(
        policy.observe(&generation, &ack).unwrap(),
        McpRefreshNotification::Acknowledged
    );
    assert_eq!(
        policy.observe(&generation, &ack).unwrap(),
        McpRefreshNotification::Ignored
    );
    let request = envelope(
        &json!({"jsonrpc":"2.0","id":8,"method":"notifications/tools/list_changed",
        "params":{"_meta":{"io.modelcontextprotocol/subscriptionId":5}}}),
    );
    assert_eq!(
        policy.observe(&generation, &request).unwrap(),
        McpRefreshNotification::Ignored
    );
    assert_eq!(
        policy.observe(&generation, &changed).unwrap(),
        McpRefreshNotification::Invalidated
    );
    assert_eq!(
        policy.observe(&McpRefreshGeneration::new(), &changed),
        Err(McpRefreshError::Foreign)
    );
}

#[test]
fn mismatched_ack_filters_close_without_accepting_later_invalidation() {
    for fields in [
        json!({}),
        json!({"toolsListChanged":false}),
        json!({"toolsListChanged":true,"resourcesListChanged":true,"promptsListChanged":true,"extra":true}),
        json!({"toolsListChanged":true,"resourcesListChanged":true,"promptsListChanged":true,
            "resourceSubscriptions":["b","a"]}),
    ] {
        let generation = McpRefreshGeneration::new();
        let mut policy = McpCatalogRefresh::new(generation.clone(), &[]).unwrap();
        policy
            .install_subscription(
                &generation,
                5,
                McpSubscriptionFilters::new(capabilities(), &["a", "b"]).unwrap(),
            )
            .unwrap();
        let ack = notification(
            "notifications/subscriptions/acknowledged",
            5,
            json!({"notifications":fields}),
        );
        assert_eq!(
            policy.observe(&generation, &ack).unwrap(),
            McpRefreshNotification::CloseUnsupported
        );
        assert_eq!(
            policy
                .observe(
                    &generation,
                    &notification("notifications/tools/list_changed", 5, json!({}))
                )
                .unwrap(),
            McpRefreshNotification::Ignored
        );
    }
}

#[test]
fn coalescing_preserves_notifications_arriving_during_refresh() {
    let (mut policy, generation) = installed(&[]);
    let changed = notification("notifications/tools/list_changed", 5, json!({}));
    policy.observe(&generation, &changed).unwrap();
    let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, 1);
    for _ in 0..1000 {
        assert_eq!(
            policy.observe(&generation, &changed).unwrap(),
            McpRefreshNotification::Invalidated
        );
    }
    policy
        .finish(
            pending,
            &catalog(McpCatalogKind::Tools, 2, r#", "ttlMs":1000"#),
            2,
        )
        .unwrap();
    let pending = ticket(&mut policy, &generation, McpCatalogKind::Tools, 2);
    policy
        .finish(
            pending,
            &catalog(McpCatalogKind::Tools, 2, r#", "ttlMs":1000"#),
            2,
        )
        .unwrap();
    assert!(matches!(
        policy.begin(&generation, McpCatalogKind::Tools, 2).unwrap(),
        McpRefreshDecision::Hit
    ));
}

#[test]
fn resource_list_invalidates_both_families_and_read_results_independently() {
    let (mut policy, generation) = installed(&[]);
    let changed = notification("notifications/resources/list_changed", 5, json!({}));
    assert_eq!(
        policy.observe(&generation, &changed).unwrap(),
        McpRefreshNotification::AllResourceReads
    );
    let read_generation = policy
        .resource_read_invalidation(&generation)
        .unwrap()
        .unwrap();
    let pending = ticket(&mut policy, &generation, McpCatalogKind::Resources, 1);
    policy
        .finish(
            pending,
            &catalog(McpCatalogKind::Resources, 1, r#", "ttlMs":1000"#),
            1,
        )
        .unwrap();
    assert!(matches!(
        policy
            .begin(&generation, McpCatalogKind::ResourceTemplates, 1)
            .unwrap(),
        McpRefreshDecision::Refresh(_)
    ));
    assert_eq!(
        policy.resource_read_invalidation(&generation).unwrap(),
        Some(read_generation)
    );
    policy.observe(&generation, &changed).unwrap();
    policy
        .clear_resource_reads(&generation, read_generation)
        .unwrap();
    assert_eq!(
        policy.resource_read_invalidation(&generation).unwrap(),
        Some(read_generation + 1)
    );
    assert!(
        policy
            .clear_resource_reads(&generation, read_generation + 2)
            .is_err()
    );
}

#[test]
fn resource_update_requires_exact_selected_uri_and_does_not_invalidate_lists() {
    let (mut policy, generation) = installed(&["custom://A"]);
    for uri in ["custom://a", "custom://A/", ""] {
        assert_eq!(
            policy
                .observe(
                    &generation,
                    &notification("notifications/resources/updated", 5, json!({"uri":uri}))
                )
                .unwrap(),
            McpRefreshNotification::Ignored
        );
    }
    assert_eq!(
        policy
            .observe(
                &generation,
                &notification(
                    "notifications/resources/updated",
                    5,
                    json!({"uri":"custom://A"})
                )
            )
            .unwrap(),
        McpRefreshNotification::AllResourceReads
    );
    for kind in [McpCatalogKind::Resources, McpCatalogKind::ResourceTemplates] {
        assert!(matches!(
            policy.begin(&generation, kind, 1).unwrap(),
            McpRefreshDecision::Hit
        ));
    }
}

#[test]
fn subscription_handoff_invalidates_all_caches_and_rejects_reused_ids() {
    let (mut policy, generation) = installed(&[]);
    assert!(
        policy
            .install_subscription(
                &generation,
                5,
                McpSubscriptionFilters::new(capabilities(), &[]).unwrap()
            )
            .is_err()
    );
    policy
        .install_subscription(
            &generation,
            6,
            McpSubscriptionFilters::new(capabilities(), &[]).unwrap(),
        )
        .unwrap();
    for kind in [
        McpCatalogKind::Tools,
        McpCatalogKind::Resources,
        McpCatalogKind::ResourceTemplates,
        McpCatalogKind::Prompts,
    ] {
        assert!(matches!(
            policy.begin(&generation, kind, 0).unwrap(),
            McpRefreshDecision::Refresh(_)
        ));
    }
    assert!(
        policy
            .resource_read_invalidation(&generation)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        policy
            .observe(
                &generation,
                &notification("notifications/tools/list_changed", 5, json!({}))
            )
            .unwrap(),
        McpRefreshNotification::Ignored
    );
    let cancelled = envelope(
        &json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":6}}),
    );
    assert_eq!(
        policy.observe(&generation, &cancelled).unwrap(),
        McpRefreshNotification::CloseCancelled
    );
    assert_eq!(
        policy.observe(&generation, &cancelled).unwrap(),
        McpRefreshNotification::Ignored
    );
}

#[test]
fn invalidation_counter_exhaustion_fails_closed_instead_of_losing_events() {
    let (mut policy, generation) = installed(&[]);
    policy.families[0].invalidation = u64::MAX;
    assert_eq!(
        policy.observe(
            &generation,
            &notification("notifications/tools/list_changed", 5, json!({}))
        ),
        Err(McpRefreshError::Exhausted)
    );
    assert!(matches!(
        policy.begin(&generation, McpCatalogKind::Tools, 0),
        Err(McpRefreshError::Closed)
    ));
}
