use super::*;

fn siblings(prefix_bytes: usize, count: usize) -> String {
    let children = (0..count)
        .map(|index| format!(r#""s{index}":{{"$id":"s{index}","type":"object"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"$id":"https://example.test/{}/","$defs":{{{children}}}}}"#,
        "a".repeat(prefix_bytes)
    )
}

#[test]
fn inherited_uri_expansion_is_shared_charged_and_bounded() {
    let source = siblings(12 * 1024, 10);
    let schema = McpSchema::parse(source.as_bytes(), McpSchemaLimits::default()).unwrap();
    assert!(schema.0.resolver.retained_byte_charge() > source.len() * 4);
    let reference_bytes = schema.0.resolver.retained_byte_charge();
    let limits = McpSchemaLimits {
        max_reference_bytes: reference_bytes,
        ..McpSchemaLimits::default()
    };
    assert!(McpSchema::parse(source.as_bytes(), limits).is_ok());
    assert_eq!(
        McpSchema::parse(
            source.as_bytes(),
            McpSchemaLimits {
                max_reference_bytes: reference_bytes - 1,
                ..limits
            }
        )
        .unwrap_err(),
        McpSchemaError::SchemaLimitExceeded
    );
    assert_eq!(
        McpSchema::parse(
            siblings(12 * 1024, 90).as_bytes(),
            McpSchemaLimits::default()
        )
        .unwrap_err(),
        McpSchemaError::SchemaLimitExceeded
    );
    assert_eq!(
        schema.validate_json(b"{}").unwrap(),
        McpSchemaValidation::Valid
    );
}

#[test]
fn retained_schema_exact_boundary_and_overflow_are_atomic() {
    let source = br#"{"type":"object","properties":{"n":{"minimum":9007199254740993.0}}}"#;
    let schema = McpSchema::parse(source, McpSchemaLimits::default()).unwrap();
    let charge = schema.retained_byte_charge();
    let limits = McpSchemaLimits {
        max_retained_bytes: charge,
        ..McpSchemaLimits::default()
    };
    assert_eq!(
        McpSchema::parse(source, limits)
            .unwrap()
            .retained_byte_charge(),
        charge
    );
    assert_eq!(
        McpSchema::parse(
            source,
            McpSchemaLimits {
                max_retained_bytes: charge - 1,
                ..limits
            }
        )
        .unwrap_err(),
        McpSchemaError::SchemaLimitExceeded
    );
    assert_eq!(
        schema.validate_json(br#"{"n":9007199254740992}"#).unwrap(),
        McpSchemaValidation::Invalid(McpSchemaViolation::Properties)
    );
    let mut total = usize::MAX;
    assert_eq!(
        super::super::accounting::add(&mut total, 1, usize::MAX),
        Err(McpSchemaError::SchemaLimitExceeded)
    );
    assert_eq!(total, usize::MAX);
    assert_eq!(
        super::super::accounting::array::<usize>(usize::MAX),
        Err(McpSchemaError::SchemaLimitExceeded)
    );
    let tiny = McpSchema::parse(
        b"true",
        McpSchemaLimits {
            max_schema_bytes: 4,
            ..limits
        },
    )
    .unwrap();
    assert!(tiny.0.resolver.retained_byte_charge() > 4);
}

#[test]
fn anchors_and_duplicate_resource_semantics_survive_accounting() {
    for source in [
        r#"{"$id":"https://example.test/root","$defs":{"a":{"$id":"child"},"b":{"$id":"child"}}}"#,
        r#"{"$anchor":"same","$dynamicAnchor":"same"}"#,
    ] {
        assert_eq!(
            McpSchema::parse(source.as_bytes(), McpSchemaLimits::default()).unwrap_err(),
            McpSchemaError::InvalidSchema
        );
    }
    let schema = McpSchema::parse(br##"{"$dynamicAnchor":"node","type":"object","properties":{"child":{"$dynamicRef":"#node"}}}"##, McpSchemaLimits::default()).unwrap();
    assert_eq!(
        schema.validate_json(br#"{"child":{}}"#).unwrap(),
        McpSchemaValidation::Valid
    );
    let bytes = schema.0.resolver.retained_byte_charge();
    assert!(
        McpSchema::parse(
            schema.raw_json().as_bytes(),
            McpSchemaLimits {
                max_reference_bytes: bytes,
                ..McpSchemaLimits::default()
            }
        )
        .is_ok()
    );
    assert!(
        McpSchema::parse(
            schema.raw_json().as_bytes(),
            McpSchemaLimits {
                max_reference_bytes: bytes - 1,
                ..McpSchemaLimits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn pattern_cache_budgets_preserve_repeated_and_uncached_validation() {
    let repeated = br#"{"allOf":[{"pattern":"[a-z]"},{"pattern":"[a-z]"}]}"#;
    let ordinary = McpSchema::parse(repeated, McpSchemaLimits::default()).unwrap();
    assert_eq!(ordinary.0.patterns.len(), 1);
    let uncached = McpSchema::parse(
        repeated,
        McpSchemaLimits {
            max_pattern_cache_bytes: 1,
            ..McpSchemaLimits::default()
        },
    )
    .unwrap();
    assert!(uncached.0.patterns.is_empty());
    assert!(ordinary.retained_byte_charge() > uncached.retained_byte_charge());
    for schema in [&ordinary, &uncached] {
        assert_eq!(
            schema.validate_json(br#""abc""#).unwrap(),
            McpSchemaValidation::Valid
        );
        assert!(matches!(
            schema.validate_json(br#""123""#).unwrap(),
            McpSchemaValidation::Invalid(_)
        ));
    }
    let source = br#"{"allOf":[{"pattern":"a"},{"pattern":"b"},{"pattern":"c"}]}"#;
    let schema = McpSchema::parse(
        source,
        McpSchemaLimits {
            max_cached_pattern_states: 2,
            ..McpSchemaLimits::default()
        },
    )
    .unwrap();
    assert_eq!(schema.0.patterns.len(), 1);
    assert_eq!(
        schema.validate_json(br#""abc""#).unwrap(),
        McpSchemaValidation::Valid
    );
    let cache_charge = ordinary.retained_byte_charge() - uncached.retained_byte_charge();
    assert_eq!(
        McpSchema::parse(
            repeated,
            McpSchemaLimits {
                max_pattern_cache_bytes: cache_charge,
                ..McpSchemaLimits::default()
            }
        )
        .unwrap()
        .0
        .patterns
        .len(),
        1
    );
    assert!(
        McpSchema::parse(
            repeated,
            McpSchemaLimits {
                max_pattern_cache_bytes: cache_charge - 1,
                ..McpSchemaLimits::default()
            }
        )
        .unwrap()
        .0
        .patterns
        .is_empty()
    );
}

#[test]
fn retained_limits_are_positive_and_lowerable_only() {
    let defaults = McpSchemaLimits::default();
    for limits in [
        McpSchemaLimits {
            max_reference_bytes: 0,
            ..defaults
        },
        McpSchemaLimits {
            max_reference_bytes: defaults.max_reference_bytes + 1,
            ..defaults
        },
        McpSchemaLimits {
            max_pattern_cache_bytes: 0,
            ..defaults
        },
        McpSchemaLimits {
            max_pattern_cache_bytes: defaults.max_pattern_cache_bytes + 1,
            ..defaults
        },
        McpSchemaLimits {
            max_cached_pattern_states: 0,
            ..defaults
        },
        McpSchemaLimits {
            max_cached_pattern_states: defaults.max_cached_pattern_states + 1,
            ..defaults
        },
        McpSchemaLimits {
            max_retained_bytes: 0,
            ..defaults
        },
        McpSchemaLimits {
            max_retained_bytes: defaults.max_retained_bytes + 1,
            ..defaults
        },
    ] {
        assert_eq!(
            McpSchema::parse(b"true", limits).unwrap_err(),
            McpSchemaError::InvalidLimits
        );
    }
}
