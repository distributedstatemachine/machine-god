use super::*;

mod accounting;
mod corpus;
mod limits;
mod patterns;

fn check(schema: &str, instance: &str, valid: bool) {
    let schema = McpSchema::parse(schema.as_bytes(), McpSchemaLimits::default()).unwrap();
    assert_eq!(schema.assessment(), McpSchemaAssessment::LocallyEvaluable);
    assert_eq!(
        schema.validate_json(instance.as_bytes()).unwrap() == McpSchemaValidation::Valid,
        valid,
        "instance={instance}"
    );
}

#[test]
fn pinned_applicator_and_validation_corpus() {
    for &(schema, instance, valid) in corpus::CASES {
        check(schema, instance, valid);
    }
}

#[test]
fn exact_numbers_preserve_schema_and_instance_lexemes() {
    for (schema, instance, valid) in [
        (r#"{"const":9007199254740992}"#, "9007199254740993", false),
        (
            r#"{"enum":[9007199254740993]}"#,
            "9.007199254740993e15",
            true,
        ),
        (r#"{"minimum":9007199254740993}"#, "9007199254740992", false),
        (r#"{"type":"integer"}"#, "9007199254740993.000", true),
        (r#"{"type":"integer"}"#, "1e-1", false),
        (r#"{"const":0}"#, "-0.0e999", true),
        (
            r#"{"uniqueItems":true}"#,
            "[9007199254740992,9007199254740993]",
            true,
        ),
        (r#"{"uniqueItems":true}"#, "[0,-0.0]", false),
        (
            r#"{"multipleOf":0.000000000000000000000000000001}"#,
            "0.123456789012345678901234567890",
            true,
        ),
        (
            r#"{"multipleOf":0.000000000000000000000000000003}"#,
            "0.123456789012345678901234567890",
            true,
        ),
        (
            r#"{"multipleOf":0.000000000000000000000000000004}"#,
            "0.123456789012345678901234567890",
            false,
        ),
        (
            r#"{"const":0.12345678901234567890123456789}"#,
            "12345678901234567890123456789e-29",
            true,
        ),
        (r#"{"minimum":1e1000000}"#, "1e1000000", true),
        (r#"{"maximum":-1e-1000000}"#, "0", false),
    ] {
        check(schema, instance, valid);
    }
    let original = b" { \"type\":\"object\", \"const\": {\"n\":9007199254740993.0} } ";
    let schema = McpSchema::parse(original, McpSchemaLimits::default()).unwrap();
    assert_eq!(schema.raw_json().as_bytes(), original);
    schema.require_object_root().unwrap();
    for source in [
        "true",
        "false",
        "{}",
        r#"{"type":["object"]}"#,
        r##"{"$ref":"#/$defs/o","$defs":{"o":{"type":"object"}}}"##,
    ] {
        assert_eq!(
            McpSchema::parse(source.as_bytes(), McpSchemaLimits::default())
                .unwrap()
                .require_object_root(),
            Err(McpSchemaError::InvalidSchema)
        );
    }
}

#[test]
fn local_pointers_anchors_nested_resources_and_dynamic_scope() {
    let simple = r##"{"$defs":{"positive":{"$anchor":"positive","type":"integer","minimum":1}},"properties":{"a":{"$ref":"#/$defs/positive"},"b":{"$ref":"#positive"}}}"##;
    check(simple, r#"{"a":2,"b":3}"#, true);
    check(simple, r#"{"a":0}"#, false);
    let recursive = r##"{"$defs":{"node":{"$dynamicAnchor":"node","type":"object","properties":{"value":{"type":"integer"},"child":{"$dynamicRef":"#node"}}}},"$ref":"#/$defs/node"}"##;
    check(recursive, r#"{"value":1,"child":{"value":2}}"#, true);
    check(recursive, r#"{"value":1,"child":{"value":false}}"#, false);
    let dynamic = r##"{"$id":"https://test.example/typical/root","$ref":"list","$defs":{"items":{"$dynamicAnchor":"items","type":"string"},"list":{"$id":"list","type":"array","items":{"$dynamicRef":"#items"},"$defs":{"bookend":{"$dynamicAnchor":"items"}}}}}"##;
    check(dynamic, r#"["foo","bar"]"#, true);
    check(dynamic, r#"["foo",42]"#, false);
    let relative = r#"{"$id":"https://test.example/relative/root","$dynamicAnchor":"meta","properties":{"foo":{"const":"pass"}},"$ref":"extended","$defs":{"extended":{"$id":"extended","$dynamicAnchor":"meta","properties":{"bar":{"$ref":"bar"}}},"bar":{"$id":"bar","properties":{"baz":{"$dynamicRef":"extended#meta"}}}}}"#;
    check(
        relative,
        r#"{"foo":"pass","bar":{"baz":{"foo":"pass"}}}"#,
        true,
    );
    check(
        relative,
        r#"{"foo":"pass","bar":{"baz":{"foo":"fail"}}}"#,
        false,
    );
    check(
        r##"{"$defs":{"a/b":{"const":1},"~name":{"const":2},"":{"const":3}},"prefixItems":[{"$ref":"#/$defs/a~1b"},{"$ref":"#/$defs/%7E0name"},{"$ref":"#/$defs/"}]}"##,
        "[1,2,3]",
        true,
    );
}

#[test]
fn draft7_tuples_dependencies_identifiers_and_ignored_ref_siblings() {
    let prefix = r#"{"$schema":"http://json-schema.org/draft-07/schema#","#;
    for (body, instance, valid) in [
        (
            r##""definitions":{"positive":{"type":"integer","minimum":1}},"properties":{"value":{"$ref":"#/definitions/positive"}}}"##,
            r#"{"value":2}"#,
            true,
        ),
        (
            r#""items":[{"type":"integer"},{"type":"string"}],"additionalItems":false}"#,
            r#"[1,"x",true]"#,
            false,
        ),
        (
            r#""dependencies":{"card":["billing"],"billing":{"required":["address"]}}}"#,
            r#"{"card":1,"billing":2,"address":3}"#,
            true,
        ),
        (
            r##""$id":"https://example.test/root","definitions":{"number":{"$id":"#number","type":"number"}},"properties":{"value":{"$ref":"#number"}}}"##,
            r#"{"value":1}"#,
            true,
        ),
        (
            r##""definitions":{"array":{"type":"array"}},"$ref":"#/definitions/array","maxItems":1}"##,
            "[1,2]",
            true,
        ),
        (
            r##""definitions":{"array":{"type":"array"}},"$ref":"#/definitions/array","type":99,"pattern":"(?=a)"}"##,
            "[1,2]",
            true,
        ),
        (
            r#""prefixItems":7,"dependentRequired":false,"$dynamicRef":false,"type":"string"}"#,
            "7",
            false,
        ),
    ] {
        check(&format!("{prefix}{body}"), instance, valid);
    }
}

#[test]
fn hard_schema_errors_dominate_delegated_patterns() {
    for schema in [
        r#"{"pattern":"(?=a)","minLength":"invalid"}"#,
        r#"{"minLength":"invalid","pattern":"(?=a)"}"#,
        r#"{"patternProperties":{"(?=a)":true},"required":7}"#,
        r#"{"properties":{"delegated":{"pattern":"(?=a)"},"bad":{"required":7}}}"#,
        r##"{"pattern":"(?=a)","$ref":"#/default","default":{"type":7}}"##,
        r##"{"$ref":"#/default","default":{"$id":"child","type":"string"}}"##,
        r#"{"allOf":[]}"#,
        r#"{"anyOf":[]}"#,
        r#"{"oneOf":[]}"#,
        r#"{"required":["a","a"]}"#,
        r#"{"dependentRequired":{"a":["b","b"]}}"#,
        r#"{"multipleOf":0}"#,
        r#"{"multipleOf":-1}"#,
        r#"{"type":["string","string"]}"#,
    ] {
        assert!(
            McpSchema::parse(schema.as_bytes(), McpSchemaLimits::default()).is_err(),
            "{schema}"
        );
    }
    for schema in [
        r#"{"pattern":"(?=a)","$ref":"https://example.test/schema"}"#,
        r#"{"$ref":"https://example.test/schema","pattern":"(?=a)"}"#,
    ] {
        assert_eq!(
            McpSchema::parse(schema.as_bytes(), McpSchemaLimits::default()).unwrap_err(),
            McpSchemaError::ExternalReference
        );
    }
    for schema in [
        r##"{"default":{"$ref":"#/default"},"$ref":"#/default"}"##,
        r##"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","default":{"$ref":"#/default"},"$ref":"#/default"}"##,
    ] {
        let schema = McpSchema::parse(schema.as_bytes(), McpSchemaLimits::default()).unwrap();
        assert_eq!(
            schema.validate_json(b"null"),
            Err(McpSchemaError::InstanceLimitExceeded)
        );
    }
}

#[test]
fn unsupported_pattern_grammar_delegates_whole_instance_not_siblings() {
    for pattern in [
        "(?=a)",
        "[\\s-a]",
        "[\\w-a]",
        "(a)\\1",
        "^*",
        "$+",
        "\\q",
        "[\\q]",
        "\\00",
        "[\\00]",
        "]",
        "}",
        "\\p{digit}",
    ] {
        let schema =
            serde_json::to_vec(&serde_json::json!({"type":"integer","pattern":pattern})).unwrap();
        let schema = McpSchema::parse(&schema, McpSchemaLimits::default()).unwrap();
        assert_eq!(
            schema.assessment(),
            McpSchemaAssessment::ServerAuthoritative,
            "{pattern}"
        );
        assert_eq!(
            schema.validate_json(br#""not an integer""#).unwrap(),
            McpSchemaValidation::ServerAuthoritative
        );
        assert_eq!(
            schema.validate_json(b"invalid"),
            Err(McpSchemaError::InvalidJson)
        );
    }
    check(
        r#"{"type":"string","futureKeyword":{"type":7},"$vocabulary":{"https://example.test/required":true}}"#,
        "7",
        false,
    );
    check(r#"{"pattern":"[\\s-]"}"#, r#""-""#, true);
    check(r#"{"pattern":"(^)?"}"#, r#""a""#, true);
}

#[test]
fn evaluated_annotations_propagate_only_successful_branches() {
    check(
        r#"{"anyOf":[{"properties":{"a":true},"required":["x"]},{"properties":{"b":true}}],"unevaluatedProperties":false}"#,
        r#"{"a":1,"b":2}"#,
        false,
    );
    check(
        r#"{"anyOf":[{"properties":{"a":true}},{"properties":{"b":true}}],"unevaluatedProperties":false}"#,
        r#"{"a":1,"b":2}"#,
        true,
    );
    check(
        r#"{"if":{"properties":{"a":true}},"then":{"properties":{"b":true}},"unevaluatedProperties":false}"#,
        r#"{"a":1,"b":2}"#,
        true,
    );
    check(
        r#"{"contains":{"type":"integer"},"unevaluatedItems":false}"#,
        "[1,2]",
        true,
    );
    check(
        r#"{"contains":{"type":"integer"},"unevaluatedItems":false}"#,
        r#"[1,"x"]"#,
        false,
    );
}

#[test]
fn bounds_duplicates_dialects_and_exact_exponent_limits() {
    let defaults = McpSchemaLimits::default();
    for source in [
        br#"{"type":"object","type":"object"}"#.as_slice(),
        br#"{"type":"object","\u0074ype":"object"}"#,
        b"true false",
        &[0xff],
    ] {
        assert_eq!(
            McpSchema::parse(source, defaults).unwrap_err(),
            McpSchemaError::InvalidJson
        );
    }
    for source in [
        r#"{"$schema":"https://json-schema.org/draft/2019-09/schema"}"#,
        r#"{"$schema":"https://json-schema.org/draft-07/schema"}"#,
    ] {
        assert_eq!(
            McpSchema::parse(source.as_bytes(), defaults).unwrap_err(),
            McpSchemaError::UnsupportedDialect
        );
    }
    assert_eq!(
        McpSchema::parse(br#"{"minimum":1e1000001}"#, defaults).unwrap_err(),
        McpSchemaError::SchemaLimitExceeded
    );
    assert_eq!(
        McpSchema::parse(br#"{"pattern":"a{1025}"}"#, defaults).unwrap_err(),
        McpSchemaError::SchemaLimitExceeded
    );
    let pattern = McpSchema::parse(
        br#"{"pattern":"(a|aa)*b"}"#,
        McpSchemaLimits {
            max_pattern_steps: 64,
            ..defaults
        },
    )
    .unwrap();
    assert_eq!(
        pattern.validate_json(br#""aaaaaaaaaaaaaaaa""#),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
    let shallow = McpSchema::parse(
        b"true",
        McpSchemaLimits {
            max_depth: 2,
            ..defaults
        },
    )
    .unwrap();
    assert_eq!(
        shallow.validate_json(b"[[[0]]]"),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
    let small = McpSchemaLimits {
        max_container_entries: 2,
        ..defaults
    };
    McpSchema::parse(br#"{"required":["a","b"]}"#, small).unwrap();
    assert_eq!(
        McpSchema::parse(br#"{"required":["a","b","c"]}"#, small).unwrap_err(),
        McpSchemaError::SchemaLimitExceeded
    );
    let exact = McpSchema::parse(br#"{"multipleOf":1}"#, defaults).unwrap();
    assert_eq!(
        exact.validate_json(b"1e8192"),
        Err(McpSchemaError::InstanceLimitExceeded)
    );
}
