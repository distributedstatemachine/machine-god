use super::*;
use serde_json::json;

fn builder(kind: McpCatalogKind, version: ProtocolVersion) -> McpCatalogBuilder {
    McpCatalogBuilder::new(kind, version, McpCatalogLimits::default()).unwrap()
}
fn page(result: serde_json::Value) -> Vec<u8> {
    let mut envelope = json!({"jsonrpc":"2.0","id":1});
    envelope["result"] = result;
    serde_json::to_vec(&envelope).unwrap()
}
fn append(
    builder: &mut McpCatalogBuilder,
    result: serde_json::Value,
    cursor: Option<&str>,
    now: u64,
) -> Result<bool, McpPaginationError> {
    builder.append_response(&page(result), &RpcId::Integer(1), cursor, now)
}

#[test]
fn producer_two_page_tools_keep_raw_metadata_and_sort_exact_names() {
    let mut builder = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
    let first = br#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","nextCursor":"page-2","ttlMs":10,"tools":[{"name":"zeta","title":"Zeta","inputSchema":{"type":"object","const":9007199254740993.0},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":true},"icons":[{"src":"data:image/png;base64,AA=="}],"_meta":{"vendor":"fixture"}}]}}"#;
    assert!(
        builder
            .append_response(first, &RpcId::Integer(1), None, 1000)
            .unwrap()
    );
    assert_eq!(builder.next_cursor(), Some("page-2"));
    assert!(
        !append(
            &mut builder,
            json!({"tools":[{"name":"alpha"}],"ttlMs":100}),
            Some("page-2"),
            1005
        )
        .unwrap()
    );
    let catalog = builder.finish().unwrap();
    let items: Vec<_> = catalog.items().collect();
    assert_eq!(
        items.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert!(items[1].1.get().contains("9007199254740993.0"));
    assert!(items[1].1.get().contains("\"vendor\":\"fixture\""));
    assert_eq!(catalog.fetched_at_ms(), 1000);
    assert_eq!(catalog.expires_at_ms(), 1010);
    assert_eq!(catalog.cache_scope(), McpCatalogCacheScope::Private);
}

#[test]
fn empty_cursor_is_an_outstanding_page_not_a_completed_catalog() {
    let mut b = builder(McpCatalogKind::Resources, ProtocolVersion::Modern);
    assert!(
        append(
            &mut b,
            json!({"resources":[{"uri":"git://b","name":"B"}],"nextCursor":"","ttlMs":100}),
            None,
            10
        )
        .unwrap()
    );
    assert_eq!(b.next_cursor(), Some(""));
    assert!(
        !append(
            &mut b,
            json!({"resources":[{"uri":"custom://a","name":"A"}],"ttlMs":50}),
            Some(""),
            20
        )
        .unwrap()
    );
    let catalog = b.finish().unwrap();
    assert_eq!(catalog.items().next().unwrap().0, "custom://a");
    assert_eq!(catalog.expires_at_ms(), 70);

    let mut b = builder(McpCatalogKind::Resources, ProtocolVersion::Modern);
    append(&mut b, json!({"resources":[],"nextCursor":""}), None, 0).unwrap();
    assert_eq!(b.finish().unwrap_err(), McpPaginationError::Incomplete);
}

#[test]
fn missing_ttl_matches_modern_immediate_and_legacy_indefinite() {
    for version in [
        ProtocolVersion::Modern,
        ProtocolVersion::Legacy20251125,
        ProtocolVersion::Legacy20250618,
        ProtocolVersion::Legacy20250326,
        ProtocolVersion::Legacy20241105,
    ] {
        let mut b = builder(McpCatalogKind::Prompts, version);
        append(&mut b, json!({"prompts":[]}), None, 30).unwrap();
        let catalog = b.finish().unwrap();
        assert_eq!(
            catalog.expires_at_ms(),
            if version == ProtocolVersion::Modern {
                30
            } else {
                u64::MAX
            }
        );
        assert_eq!(catalog.version(), version);
    }
}

#[test]
fn exact_ttl_numbers_do_not_round_through_floating_point() {
    for (literal, expected) in [
        ("-25", Some(0)),
        ("-0.1", Some(0)),
        ("-0", Some(0)),
        ("0", Some(0)),
        ("1e3", Some(1000)),
        ("12300e-2", Some(123)),
        ("1.000e0", Some(1)),
        ("9007199254740993.0", Some(9_007_199_254_740_993)),
        ("18446744073709551615", Some(u64::MAX)),
        ("18446744073709551616", None),
        ("1.00000000000000000001", None),
        ("1e-1", None),
        ("null", None),
        ("\"10\"", None),
        ("false", None),
    ] {
        let raw = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"tools\":[],\"ttlMs\":{literal}}}}}"
        );
        let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
        let result = b.append_response(raw.as_bytes(), &RpcId::Integer(1), None, 0);
        if let Some(expected) = expected {
            result.unwrap();
            assert_eq!(b.finish().unwrap().expires_at_ms(), expected, "{literal}");
        } else {
            assert!(result.is_err(), "{literal}");
            assert_eq!(b.finish().unwrap_err(), McpPaginationError::Closed);
        }
    }
}

#[test]
fn negative_ttl_and_overflowing_absolute_expiry_are_safe() {
    let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
    append(
        &mut b,
        json!({"tools":[],"ttlMs":-25,"cacheScope":"public"}),
        None,
        1000,
    )
    .unwrap();
    let c = b.finish().unwrap();
    assert_eq!(c.expires_at_ms(), 1000);
    assert_eq!(c.cache_scope(), McpCatalogCacheScope::Public);
    let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
    append(&mut b, json!({"tools":[],"ttlMs":10}), None, u64::MAX - 1).unwrap();
    assert_eq!(b.finish().unwrap().expires_at_ms(), u64::MAX);
}

#[test]
fn every_family_uses_its_exact_array_and_identity() {
    for (kind, key, identity, method) in [
        (McpCatalogKind::Tools, "tools", "name", "tools/list"),
        (
            McpCatalogKind::Resources,
            "resources",
            "uri",
            "resources/list",
        ),
        (
            McpCatalogKind::ResourceTemplates,
            "resourceTemplates",
            "uriTemplate",
            "resources/templates/list",
        ),
        (McpCatalogKind::Prompts, "prompts", "name", "prompts/list"),
    ] {
        let mut b = builder(kind, ProtocolVersion::Modern);
        append(&mut b, json!({key:[{identity:"item"}]}), None, 0).unwrap();
        let catalog = b.finish().unwrap();
        assert_eq!(catalog.kind(), kind);
        assert_eq!(kind.method(), method);
        assert_eq!(catalog.items().next().unwrap().0, "item");
        let mut b = builder(kind, ProtocolVersion::Modern);
        assert!(append(&mut b, json!({key:[{"unrelated":"item"}]}), None, 0).is_err());
    }
}

#[test]
fn rejects_duplicate_items_within_and_across_pages_atomically() {
    for across in [false, true] {
        let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
        if across {
            append(
                &mut b,
                json!({"tools":[{"name":"a"}],"nextCursor":"p2"}),
                None,
                0,
            )
            .unwrap();
        }
        let result = if across {
            json!({"tools":[{"name":"a"}]})
        } else {
            json!({"tools":[{"name":"a"},{"name":"a"}]})
        };
        assert_eq!(
            append(&mut b, result, across.then_some("p2"), 1),
            Err(McpPaginationError::DuplicateItem)
        );
        assert_eq!(b.finish().unwrap_err(), McpPaginationError::Closed);
    }
}

#[test]
fn rejects_cursor_cycles_mismatches_scope_changes_and_clock_regression() {
    for (cursor, next, scope, now, expected) in [
        (
            "p2",
            Some("p2"),
            "private",
            10,
            McpPaginationError::DuplicateCursor,
        ),
        (
            "other",
            None,
            "private",
            10,
            McpPaginationError::CursorMismatch,
        ),
        (
            "p2",
            None,
            "public",
            10,
            McpPaginationError::InconsistentCacheScope,
        ),
        (
            "p2",
            None,
            "private",
            9,
            McpPaginationError::RegressingClock,
        ),
    ] {
        let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
        append(&mut b, json!({"tools":[],"nextCursor":"p2"}), None, 10).unwrap();
        let mut second = json!({"tools":[],"cacheScope":scope});
        if let Some(next) = next {
            second["nextCursor"] = json!(next);
        }
        assert_eq!(append(&mut b, second, Some(cursor), now), Err(expected));
        assert_eq!(b.next_cursor(), None);
        assert_eq!(b.finish().unwrap_err(), McpPaginationError::Closed);
    }
}

#[test]
fn rejects_malformed_or_uncorrelated_envelopes_without_revival() {
    for raw in [
        r#"{"jsonrpc":"2.0","id":"1","result":{"tools":[]}}"#,
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-1,"message":"secret"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[],"to\u006fls":[]}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[],"resultType":"input_required"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[],"nextCursor":null}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[],"cacheScope":null}}"#,
    ] {
        let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
        assert!(
            b.append_response(raw.as_bytes(), &RpcId::Integer(1), None, 0)
                .is_err(),
            "{raw}"
        );
        assert_eq!(
            append(&mut b, json!({"tools":[]}), None, 1),
            Err(McpPaginationError::Closed)
        );
    }
    let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
    assert_eq!(
        b.append_response(
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"secret"}}"#,
            &RpcId::Integer(1),
            None,
            0
        ),
        Err(McpPaginationError::ProtocolFailure)
    );
}

#[test]
fn escaped_field_and_identity_spellings_are_supported_without_loss() {
    let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
    b.append_response(br#"{"jsonrpc":"2.0","id":1,"result":{"to\u006fls":[{"na\u006de":"\u0061"}],"ttl\u004ds":1e3}}"#, &RpcId::Integer(1), None, 0).unwrap();
    let catalog = b.finish().unwrap();
    assert_eq!(catalog.items().next().unwrap().0, "a");
    assert!(
        catalog
            .items()
            .next()
            .unwrap()
            .1
            .get()
            .contains("na\\u006de")
    );
    assert_eq!(catalog.expires_at_ms(), 1000);
}

#[test]
fn fresh_finished_and_failed_builders_have_distinct_terminal_states() {
    assert_eq!(
        builder(McpCatalogKind::Tools, ProtocolVersion::Modern)
            .finish()
            .unwrap_err(),
        McpPaginationError::Incomplete
    );
    let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
    append(&mut b, json!({"tools":[]}), None, 0).unwrap();
    assert_eq!(
        append(&mut b, json!({"tools":[]}), None, 0),
        Err(McpPaginationError::Closed)
    );
    assert_eq!(b.finish().unwrap_err(), McpPaginationError::Closed);
}

#[test]
fn budgets_are_positive_cumulative_and_inclusive() {
    let first = page(json!({"tools":[{"name":"a"}],"nextCursor":"p2"}));
    let second = page(json!({"tools":[{"name":"b"}]}));
    let total_bytes = first.len() + second.len();
    let nodes = count_nodes(
        parse_envelope(&first, WireLimits::default())
            .unwrap()
            .value(),
    ) + count_nodes(
        parse_envelope(&second, WireLimits::default())
            .unwrap()
            .value(),
    );
    let item_bytes = 2 * (1 + r#"{"name":"a"}"#.len());
    for (bytes, node_limit, item_limit, ok) in [
        (total_bytes, nodes, item_bytes, true),
        (total_bytes - 1, nodes, item_bytes, false),
        (total_bytes, nodes - 1, item_bytes, false),
        (total_bytes, nodes, item_bytes - 1, false),
    ] {
        let mut b = McpCatalogBuilder::new(
            McpCatalogKind::Tools,
            ProtocolVersion::Modern,
            McpCatalogLimits {
                max_response_bytes: bytes,
                max_nodes: node_limit,
                max_item_bytes: item_limit,
                ..McpCatalogLimits::default()
            },
        )
        .unwrap();
        b.append_response(&first, &RpcId::Integer(1), None, 0)
            .unwrap();
        assert_eq!(
            b.append_response(&second, &RpcId::Integer(1), Some("p2"), 0)
                .is_ok(),
            ok
        );
    }
    for limits in [
        McpCatalogLimits {
            max_pages: 0,
            ..McpCatalogLimits::default()
        },
        McpCatalogLimits {
            max_items: 4097,
            ..McpCatalogLimits::default()
        },
        McpCatalogLimits {
            max_nodes: 0,
            ..McpCatalogLimits::default()
        },
    ] {
        assert!(
            McpCatalogBuilder::new(McpCatalogKind::Tools, ProtocolVersion::Modern, limits).is_err()
        );
    }
}

#[test]
fn page_cursor_and_item_count_limits_prevent_futile_next_requests() {
    for (limits, result) in [
        (
            McpCatalogLimits {
                max_pages: 1,
                ..McpCatalogLimits::default()
            },
            json!({"tools":[],"nextCursor":""}),
        ),
        (
            McpCatalogLimits {
                max_cursor_bytes: 1,
                ..McpCatalogLimits::default()
            },
            json!({"tools":[],"nextCursor":"long"}),
        ),
        (
            McpCatalogLimits {
                max_items: 1,
                ..McpCatalogLimits::default()
            },
            json!({"tools":[{"name":"a"},{"name":"b"}]}),
        ),
    ] {
        let mut b =
            McpCatalogBuilder::new(McpCatalogKind::Tools, ProtocolVersion::Modern, limits).unwrap();
        assert_eq!(
            append(&mut b, result, None, 0),
            Err(McpPaginationError::Limit)
        );
    }
}

#[test]
fn default_catalogs_admit_exact_pinned_family_cardinalities() {
    for kind in [
        McpCatalogKind::Tools,
        McpCatalogKind::Resources,
        McpCatalogKind::ResourceTemplates,
        McpCatalogKind::Prompts,
    ] {
        for overflow in [false, true] {
            let mut builder = builder(kind, ProtocolVersion::Modern);
            let count = kind.max_items() + usize::from(overflow);
            let items: Vec<_> = (0..count)
                .map(|index| json!({kind.identity().0: format!("item-{index}")}))
                .collect();
            let result = append(&mut builder, json!({kind.field():items}), None, 0);
            if overflow {
                assert_eq!(result, Err(McpPaginationError::Limit));
                assert!(builder.finish().is_err());
            } else {
                assert_eq!(result, Ok(false));
                assert_eq!(builder.finish().unwrap().items().count(), kind.max_items());
            }
        }
    }
}

#[test]
fn common_feature_envelope_depth_counts_ignored_metadata_and_root_offset() {
    for kind in [
        McpCatalogKind::Tools,
        McpCatalogKind::Resources,
        McpCatalogKind::ResourceTemplates,
        McpCatalogKind::Prompts,
    ] {
        for deepest in [32, 33] {
            let mut builder = builder(kind, ProtocolVersion::Modern);
            // Envelope root is pinned depth0, this metadata value starts at1.
            let mut metadata = json!(0);
            for _ in 1..deepest {
                metadata = json!([metadata]);
            }
            let bytes = serde_json::to_vec(
                &json!({"jsonrpc":"2.0","id":1,"result":{kind.field():[]},"ignored":metadata}),
            )
            .unwrap();
            let accepted = kind == McpCatalogKind::Tools || deepest == 32;
            assert_eq!(
                builder
                    .append_response(&bytes, &RpcId::Integer(1), None, 0)
                    .is_ok(),
                accepted
            );
            assert_eq!(builder.finish().is_ok(), accepted);
        }
    }
}

#[test]
fn debug_and_errors_never_expose_cursors_or_catalog_data() {
    let mut b = builder(McpCatalogKind::Tools, ProtocolVersion::Modern);
    append(
        &mut b,
        json!({"tools":[{"name":"secret-name"}],"nextCursor":"secret-cursor"}),
        None,
        0,
    )
    .unwrap();
    assert!(!format!("{b:?}").contains("secret"));
    append(&mut b, json!({"tools":[]}), Some("secret-cursor"), 1).unwrap();
    assert!(!format!("{:?}", b.finish().unwrap()).contains("secret"));
    assert_eq!(
        McpPaginationError::InvalidResponse.to_string(),
        "MCP catalog assembly rejected"
    );
}
