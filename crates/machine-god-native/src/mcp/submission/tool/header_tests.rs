use super::*;
use crate::mcp::endpoint::McpEndpoint;

fn base(version: ProtocolVersion) -> McpSubmissionHttpHead {
    let mut fields = vec![("Authorization", b"Bearer secret".as_slice())];
    fields.push(("Mcp-Protocol-Version", version.as_str().as_bytes()));
    McpSubmissionHttpHead::new(
        &McpEndpoint::parse("https://example.test/mcp").unwrap(),
        &fields,
    )
    .unwrap()
}

#[test]
fn typed_http_head_preserves_allocation_through_permission_and_claim() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let request = fixture.request("call");
    let projection = project(
        &fixture,
        &schema,
        &request,
        options(ProtocolVersion::Modern, TransportKind::StreamableHttp),
    )
    .unwrap()
    .with_http_head(&base(ProtocolVersion::Modern))
    .unwrap();
    let head = projection.http_head().unwrap();
    let prepared = prepare(&fixture, &request, projection).unwrap();
    assert!(Arc::ptr_eq(
        prepared.data.http_head.as_ref().unwrap(),
        &head
    ));
    fixture.admission("call", prepared).admit().unwrap();
    let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    let claimed = submission.http_head().unwrap();
    assert!(Arc::ptr_eq(&claimed, &head));
    assert!(
        claimed
            .headers()
            .any(|(name, value)| name == "authorization" && value == b"Bearer secret")
    );
}

fn http_projection(raw: &str, value: Value) -> Result<McpToolRequest> {
    let (fixture, schema) = fixture(raw);
    let mut request = fixture.request("call");
    arguments(&mut request, value);
    project(
        &fixture,
        &schema,
        &request,
        options(ProtocolVersion::Modern, TransportKind::StreamableHttp),
    )?
    .with_http_head(&base(ProtocolVersion::Modern))
}

#[test]
fn modern_http_projects_nested_headers_without_removing_body_arguments() {
    let raw = r#"{"type":"object","properties":{"secret":{"type":"integer","x-mcp-header":"Count"},"nested":{"type":"object","properties":{"enabled":{"type":"boolean","x-mcp-header":"Enabled"},"text":{"type":"string","x-mcp-header":"Text"}}}}}"#;
    let value = json!({"secret":1,"nested":{"enabled":true,"text":"café"}});
    let (fixture, schema) = fixture(raw);
    let mut request = fixture.request("call");
    arguments(&mut request, value.clone());
    let projection = project(
        &fixture,
        &schema,
        &request,
        options(ProtocolVersion::Modern, TransportKind::StreamableHttp),
    )
    .unwrap()
    .with_http_head(&base(ProtocolVersion::Modern))
    .unwrap();
    let head = projection.http_head().unwrap();
    let fields: BTreeMap<_, _> = head.headers().collect();
    assert_eq!(fields["authorization"], b"Bearer secret");
    assert_eq!(fields["mcp-method"], b"tools/call");
    assert_eq!(fields["mcp-name"], b"secret-tool");
    assert_eq!(fields["mcp-param-count"], b"1");
    assert_eq!(fields["mcp-param-enabled"], b"true");
    assert_eq!(fields["mcp-param-text"], b"=?base64?Y2Fmw6k=?=");
    let prepared = prepare(&fixture, &request, projection).unwrap();
    let wire = std::str::from_utf8(&prepared.data.wire).unwrap();
    let (headers, body) = wire.split_once("\r\n\r\n").unwrap();
    assert!(headers.contains(&format!("content-length: {}", body.len())));
    let payload: Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["params"]["arguments"], value);
}

#[test]
fn unsafe_header_text_is_encoded_and_empty_text_remains_empty() {
    for (text, expected) in [
        ("", ""),
        ("plain", "plain"),
        (" a ", "=?base64?IGEg?="),
        ("\r\n", "=?base64?DQo=?="),
        ("=?base64?", "=?base64?PT9iYXNlNjQ/?="),
    ] {
        let projection = http_projection(
            r#"{"type":"object","properties":{"secret":{"type":"string","x-mcp-header":"Value"}}}"#,
            json!({"secret":text}),
        )
        .unwrap();
        let head = projection.http_head().unwrap();
        assert_eq!(
            head.headers()
                .find(|(name, _)| *name == "mcp-param-value")
                .unwrap()
                .1,
            expected.as_bytes()
        );
    }
}

#[test]
fn null_and_missing_optional_headers_are_not_projected() {
    for value in [json!({}), json!({"secret":null})] {
        let projection = http_projection(r#"{"type":"object","properties":{"secret":{"type":"integer","x-mcp-header":"Value"}}}"#, value).unwrap();
        let head = projection.http_head().unwrap();
        assert!(
            !head
                .headers()
                .any(|(name, _)| name.starts_with("mcp-param-"))
        );
    }
}

#[test]
fn annotation_eligibility_rejects_ambiguous_locations_types_and_names() {
    for raw in [
        r#"{"type":"object","x-mcp-header":"Root"}"#,
        r#"{"type":"object","properties":{"a":{"type":"integer","x-mcp-header":"A"},"b":{"type":"string","x-mcp-header":"a"}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"number","x-mcp-header":"A"}}}"#,
        r#"{"type":"object","properties":{"a":{"type":["string","null"],"x-mcp-header":"A"}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"string","x-mcp-header":"bad name"}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"string","x-mcp-header":true}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"string","x-mcp-header":"A","oneOf":[{"type":"string"}]}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"array","items":{"type":"string","x-mcp-header":"A"}}}}"#,
        r#"{"type":"object","allOf":[{"properties":{"a":{"type":"string","x-mcp-header":"A"}}}]}"#,
        r#"{"type":"object","$defs":{"a":{"type":"string","x-mcp-header":"A"}}}"#,
    ] {
        let schema = McpSchema::parse(raw.as_bytes(), McpSchemaLimits::default()).unwrap();
        assert!(
            McpToolRequest::validate_modern_http_schema(&schema).is_err(),
            "{raw}"
        );
    }
    let schema = McpSchema::parse(br#"{"type":"object","properties":{"a":{"type":"object","properties":{"b":{"type":"string","x-mcp-header":"B"}}}}}"#, McpSchemaLimits::default()).unwrap();
    McpToolRequest::validate_modern_http_schema(&schema).unwrap();
}

#[test]
fn integer_headers_reject_unsafe_values_and_noninteger_lexemes() {
    let raw =
        r#"{"type":"object","properties":{"secret":{"type":"integer","x-mcp-header":"Value"}}}"#;
    for value in [
        json!(9_007_199_254_740_992_i64),
        json!(-9_007_199_254_740_992_i64),
        json!(1.0),
        json!(1.5),
    ] {
        assert!(http_projection(raw, json!({"secret":value})).is_err());
    }
    for value in [9_007_199_254_740_991_i64, -9_007_199_254_740_991_i64, 0] {
        let projection = http_projection(raw, json!({"secret":value})).unwrap();
        assert_eq!(
            projection
                .http_head()
                .unwrap()
                .headers()
                .find(|(name, _)| *name == "mcp-param-value")
                .unwrap()
                .1,
            value.to_string().as_bytes()
        );
    }
}

#[test]
fn http_header_selection_is_exact_single_use_and_protocol_bound() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let request = fixture.request("call");
    let modern = options(ProtocolVersion::Modern, TransportKind::StreamableHttp);
    assert!(
        project(&fixture, &schema, &request, modern)
            .unwrap()
            .with_http_head(
                &McpSubmissionHttpHead::new(
                    &McpEndpoint::parse("https://example.test/mcp").unwrap(),
                    &[("Mcp-Protocol-Version", b"2025-11-25")],
                )
                .unwrap()
            )
            .is_err()
    );
    assert!(
        project(
            &fixture,
            &schema,
            &request,
            options(ProtocolVersion::Modern, TransportKind::Stdio)
        )
        .unwrap()
        .with_http_head(&base(ProtocolVersion::Modern))
        .is_err()
    );
    let projection = project(&fixture, &schema, &request, modern).unwrap();
    assert!(prepare(&fixture, &request, projection).is_err());
    let projection = project(&fixture, &schema, &request, modern)
        .unwrap()
        .with_http_head(&base(ProtocolVersion::Modern))
        .unwrap();
    assert!(
        projection
            .with_http_head(&base(ProtocolVersion::Modern))
            .is_err()
    );
    for reserved in ["Mcp-Method", "Mcp-Name", "Mcp-Param-Value"] {
        let head = McpSubmissionHttpHead::new(
            &McpEndpoint::parse("https://example.test/mcp").unwrap(),
            &[
                (
                    "Mcp-Protocol-Version",
                    ProtocolVersion::Modern.as_str().as_bytes(),
                ),
                (reserved, b"override"),
            ],
        )
        .unwrap();
        assert!(
            project(&fixture, &schema, &request, modern)
                .unwrap()
                .with_http_head(&head)
                .is_err()
        );
    }
}

#[test]
fn http_rejects_ineligible_header_annotations() {
    let (fixture, schema) = fixture(r#"{"type":"object","x-mcp-header":"not-modern-eligible"}"#);
    {
        let request = fixture.request("call");
        let projection = project(
            &fixture,
            &schema,
            &request,
            options(ProtocolVersion::Modern, TransportKind::StreamableHttp),
        )
        .unwrap()
        .with_http_head(&base(ProtocolVersion::Modern));
        assert!(projection.is_err());
    }
}

#[test]
fn header_value_encoded_expansion_is_bounded_before_request_admission() {
    let raw =
        r#"{"type":"object","properties":{"secret":{"type":"string","x-mcp-header":"Value"}}}"#;
    assert!(http_projection(raw, json!({"secret":"a".repeat(16 * 1024)})).is_ok());
    assert!(matches!(
        http_projection(raw, json!({"secret":"a".repeat(16 * 1024 + 1)})),
        Err(McpSubmissionError::Limit)
    ));
    assert!(matches!(
        http_projection(
            raw,
            json!({"secret":format!(" {}", "a".repeat(16 * 1024 - 1))})
        ),
        Err(McpSubmissionError::Limit)
    ));
}

#[test]
fn typed_http_submission_keeps_exact_head_through_concrete_permission_claim() {
    let (fixture, schema) = fixture(
        r#"{"type":"object","properties":{"secret":{"type":"integer","x-mcp-header":"Secret"}}}"#,
    );
    let request = fixture.request("call");
    let projection = project(
        &fixture,
        &schema,
        &request,
        options(ProtocolVersion::Modern, TransportKind::StreamableHttp),
    )
    .unwrap()
    .with_http_head(&base(ProtocolVersion::Modern))
    .unwrap();
    let head = projection.http_head().unwrap();
    let prepared = prepare(&fixture, &request, projection).unwrap();
    let expected = prepared.data.wire.clone();
    fixture.admission("call", prepared).admit().unwrap();
    let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    assert_eq!(submission.http_request_bytes().unwrap(), expected.as_ref());
    assert_eq!(
        head.headers()
            .find(|(name, _)| *name == "mcp-param-secret")
            .unwrap()
            .1,
        b"1"
    );
    assert!(!submission.was_attempted());
    fixture.revoke();
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_err());
}

#[test]
fn composed_protocol_headers_share_the_existing_field_count_budget() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let request = fixture.request("call");
    for count in [254, 255, 256] {
        let names: Vec<_> = (1..count).map(|index| format!("x-field-{index}")).collect();
        let mut fields: Vec<_> = names
            .iter()
            .map(|name| (name.as_str(), b"value".as_slice()))
            .collect();
        fields.push((
            "Mcp-Protocol-Version",
            ProtocolVersion::Modern.as_str().as_bytes(),
        ));
        let base = McpSubmissionHttpHead::new(
            &McpEndpoint::parse("https://example.test/mcp").unwrap(),
            &fields,
        )
        .unwrap();
        let projection = project(
            &fixture,
            &schema,
            &request,
            options(ProtocolVersion::Modern, TransportKind::StreamableHttp),
        )
        .unwrap()
        .with_http_head(&base);
        if count == 254 {
            assert_eq!(
                projection.unwrap().http_head().unwrap().headers().len(),
                256
            );
        } else {
            assert!(matches!(projection, Err(McpSubmissionError::Limit)));
        }
    }
}
