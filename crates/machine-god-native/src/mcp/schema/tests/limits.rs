use super::*;

#[test]
fn schema_and_instance_byte_node_and_lexeme_boundaries() {
    let defaults = McpSchemaLimits::default();
    let limits = McpSchemaLimits {
        max_schema_bytes: 4,
        max_instance_bytes: 4,
        ..defaults
    };
    let schema = McpSchema::parse(b"true", limits).unwrap();
    assert_eq!(
        schema.validate_json(b"null").unwrap(),
        McpSchemaValidation::Valid
    );
    assert_eq!(
        schema.validate_json(b"false"),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
    assert_eq!(
        McpSchema::parse(b"false", limits).unwrap_err(),
        McpSchemaError::SchemaLimitExceeded
    );
    let limits = McpSchemaLimits {
        max_nodes: 3,
        ..defaults
    };
    let schema = McpSchema::parse(b"true", limits).unwrap();
    assert_eq!(
        schema.validate_json(b"[1,2]").unwrap(),
        McpSchemaValidation::Valid
    );
    assert_eq!(
        schema.validate_json(b"[1,2,3]"),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
    let limits = McpSchemaLimits {
        max_number_bytes: 4,
        ..defaults
    };
    let schema = McpSchema::parse(b"true", limits).unwrap();
    assert_eq!(
        schema.validate_json(b"1e99").unwrap(),
        McpSchemaValidation::Valid
    );
    assert_eq!(
        schema.validate_json(b"1e999"),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
    assert_eq!(
        schema.validate_json(br#"{"x":1,"\u0078":2}"#),
        Err(McpSchemaError::InvalidJson)
    );
}

#[test]
fn evaluation_and_reference_budgets_remain_hard_errors() {
    let limits = McpSchemaLimits {
        max_steps: 2,
        ..McpSchemaLimits::default()
    };
    let schema = McpSchema::parse(br#"{"allOf":[true,true]}"#, limits).unwrap();
    assert_eq!(
        schema.validate_json(b"null"),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
    let limits = McpSchemaLimits {
        max_ref_hops: 1,
        ..McpSchemaLimits::default()
    };
    let schema = McpSchema::parse(
        br##"{"$defs":{"a":{"$ref":"#/$defs/b"},"b":true},"$ref":"#/$defs/a"}"##,
        limits,
    )
    .unwrap();
    assert_eq!(
        schema.validate_json(b"null"),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
    let schema = McpSchema::parse(br#"{"multipleOf":1}"#, McpSchemaLimits::default()).unwrap();
    assert_eq!(
        schema.validate_json(b"1e8191").unwrap(),
        McpSchemaValidation::Valid
    );
    assert_eq!(
        schema.validate_json(b"1e8192"),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
    for source in [
        br#"{"minimum":10e1000000}"#.as_slice(),
        br#"{"minimum":0.1e-1000000}"#,
    ] {
        assert_eq!(
            McpSchema::parse(source, McpSchemaLimits::default()).unwrap_err(),
            McpSchemaError::SchemaLimitExceeded
        );
    }
}
