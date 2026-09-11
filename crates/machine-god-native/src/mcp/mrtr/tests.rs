use super::*;
use crate::mcp::protocol::ProtocolVersion;

fn json(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).unwrap()
}
fn input(text: &str) -> Result<McpInputRequired> {
    McpInputRequired::parse(&json(text), McpMrtrLimits::default())
}
fn elicitation(text: &str) -> Result<McpElicitationRequest> {
    McpElicitationRequest::parse(
        &json(text),
        ProtocolVersion::Modern,
        McpMrtrLimits::default(),
    )
}
fn form(schema: &str) -> Result<McpElicitationRequest> {
    elicitation(&format!(
        "{{\"message\":\"Choose\",\"requestedSchema\":{schema}}}"
    ))
}

#[test]
fn closed_requests_and_exact_correlated_responses() {
    let required = input(r#"{"inputRequests":{"sample":{"method":"sampling/createMessage","params":{"messages":[],"maxTokens":8}},"roots":{"method":"roots/list"},"form":{"method":"elicitation/create","params":{"message":"Continue?","requestedSchema":{"type":"object","properties":{"confirmed":{"type":"boolean"}},"required":["confirmed"]}}},"url":{"method":"elicitation/create","params":{"mode":"url","message":"Authenticate","url":"https://example.test/auth"}}},"requestState":"opaque"}"#).unwrap();
    assert_eq!(required.requests().len(), 4);
    assert_eq!(required.request_state_json().unwrap().get(), "\"opaque\"");
    assert!(!required.legacy_retry_without_responses());
    let response = json(
        r#"{"sample":{"role":"assistant","content":{"type":"text","text":"done"},"model":"fixture"},"roots":{"roots":[{"uri":"file:///tmp"}]},"form":{"action":"accept","content":{"confirmed":true}},"url":{"action":"accept"}}"#,
    );
    let valid = required.validate_responses(&response).unwrap();
    assert_eq!(valid.responses().len(), 4);
    assert_eq!(valid.raw_json().get(), response.get());
    for invalid in [
        "{}",
        r#"{"sample":{"roots":[]},"roots":{"roots":[]},"form":{"action":"decline"},"url":{"action":"accept"}}"#,
        r#"{"sample":{"role":"assistant","content":{"type":"text","text":"done"},"model":"fixture"},"roots":{"roots":[]},"form":{"action":"accept","content":{"confirmed":"yes"}},"url":{"action":"accept"}}"#,
    ] {
        assert!(required.validate_responses(&json(invalid)).is_err());
    }
    assert!(input(r#"{"inputRequests":{"bad":{"method":"custom/request","params":{}}}}"#).is_err());
    assert!(!format!("{required:?} {valid:?}").contains("opaque"));
}

#[test]
fn opaque_state_and_unknown_metadata_are_exact_and_bounded() {
    let text = r#"{"requestState":{"$serde_json::private::Number":-0,"large":9007199254740993.00000001,"future":1e400}}"#;
    let required = input(text).unwrap();
    assert!(required.requests().is_empty());
    assert_eq!(required.raw_json().get(), text);
    assert!(
        required
            .request_state_json()
            .unwrap()
            .get()
            .contains("1e400")
    );
    assert_eq!(
        required
            .validate_responses(&json("{}"))
            .unwrap()
            .wire_json()
            .get(),
        "{}"
    );
    for text in [
        "{}",
        "null",
        r#"{"inputRequests":null}"#,
        r#"{"inputRequests":{"":{"method":"roots/list"}}}"#,
        r#"{"requestState":{"x":1,"x":2}}"#,
        r#"{"requestState":{"x":1,"\u0078":2}}"#,
    ] {
        assert!(input(text).is_err(), "{text}");
    }
    let raw = json(r#"{"inputRequests":{},"requestState":[[[[true]]]]}"#);
    assert!(
        McpInputRequired::parse(
            &raw,
            McpMrtrLimits {
                max_depth: 3,
                ..McpMrtrLimits::default()
            }
        )
        .is_err()
    );
    assert!(
        input(r#"{"requestState":null}"#)
            .unwrap()
            .request_state_json()
            .is_some()
    );
}

#[test]
fn sampling_preserves_all_pinned_content_shapes_without_implementation() {
    let params = r#"{"messages":[{"role":"user","content":[{"type":"text","text":""},{"type":"image","data":"not-base64","mimeType":""},{"type":"audio","data":"x","mimeType":"audio/test"},{"type":"tool_use","id":"id","name":"name","input":{"n":-0}},{"type":"tool_result","toolUseId":"id","content":[{"type":"text","text":"nested"}]}]}],"maxTokens":8.0,"temperature":1e400,"systemPrompt":"","includeContext":"allServers","stopSequences":[""],"metadata":{},"modelPreferences":{},"tools":[{"name":"","inputSchema":{}}],"toolChoice":{"mode":"required"}}"#;
    let required = input(&format!("{{\"inputRequests\":{{\"sample\":{{\"method\":\"sampling/createMessage\",\"params\":{params}}}}}}}")).unwrap();
    assert_eq!(
        required.requests()[0]
            .payload()
            .params_json()
            .unwrap()
            .get(),
        params
    );
    for max in [
        "0",
        "-1",
        "1.5",
        "18446744073709551616",
        "1e1000001",
        "null",
    ] {
        assert!(input(&format!("{{\"inputRequests\":{{\"s\":{{\"method\":\"sampling/createMessage\",\"params\":{{\"messages\":[],\"maxTokens\":{max}}}}}}}}}")).is_err());
    }
    let roots =
        input(r#"{"inputRequests":{"r":{"method":"roots/list","params":{"_meta":{}}}}}"#).unwrap();
    assert!(
        roots
            .validate_responses(&json(
                r#"{"r":{"roots":[{"uri":"","name":"","_meta":{"future":1e400}}]}}"#
            ))
            .is_ok()
    );
    assert!(
        input(r#"{"inputRequests":{"r":{"method":"roots/list","params":{"_meta":null}}}}"#)
            .is_err()
    );
}

#[test]
fn every_form_kind_and_exact_decimal_constraints() {
    let request = form(r#"{"type":"object","additionalProperties":false,"properties":{"name":{"type":"string","minLength":2,"maxLength":20,"pattern":"^[A-Za-z]+$"},"age":{"type":"integer","minimum":18,"maximum":120},"ratio":{"type":"number","minimum":0.1,"maximum":0.3,"multipleOf":0.1,"default":0.3},"enabled":{"type":"boolean","default":true},"color":{"type":"string","oneOf":[{"const":"red","title":"Duplicate"},{"const":"blue","title":"Duplicate"}]},"tags":{"type":"array","items":{"anyOf":[{"const":"a","title":"A"},{"const":"b","title":"B"}]},"minItems":1},"large":{"type":"integer","minimum":9007199254740993,"maximum":9007199254740995,"default":9007199254740994}},"required":["name","age","large"]}"#).unwrap();
    assert_eq!(request.form_schema().unwrap().fields().len(), 7);
    let valid = r#"{"action":"accept","content":{"name":"Alice","age":42.0,"ratio":0.300000000000000000000000000000,"enabled":false,"color":"red","tags":["a","b"],"large":9007199254740994}}"#;
    assert!(
        request
            .validate_response(&json(valid))
            .unwrap()
            .1
            .get()
            .contains("0.300000000000000000000000000000")
    );
    for bad in [
        valid.replace("9007199254740994", "9007199254740992"),
        valid.replace(
            "0.300000000000000000000000000000",
            "0.300000000000000000000000000001",
        ),
        valid.replace("\"a\",\"b\"", "\"a\",\"a\""),
        valid.replace("Alice", "A1"),
        valid.replace("42.0", "42.5"),
    ] {
        assert!(request.validate_response(&json(&bad)).is_err());
    }
    assert!(request.validate_response(&json(r#"{"action":"accept","content":{"name":"Alice","age":42,"large":9007199254740994,"extra":1}}"#)).is_err());
}

#[test]
fn form_grammar_is_not_general_json_schema() {
    // Pin validates the string constraints' schema but ignores them for selects;
    // unrelated keywords are annotations rather than broad JSON Schema policy.
    let request = form(r#"{"$schema":"future","type":"object","properties":{"choice":{"type":"string","enum":["x"],"enumNames":["X"],"minLength":10,"pattern":"^other$","const":"other"}},"required":["choice"]}"#).unwrap();
    assert!(
        request
            .validate_response(&json(r#"{"action":"accept","content":{"choice":"x"}}"#))
            .is_ok()
    );
    for schema in [
        r#"{"type":"object","title":"not-on-mcp","properties":{}}"#,
        r#"{"type":"object","additionalProperties":true,"properties":{}}"#,
        r#"{"type":"object","properties":{"x":{"type":"object"}}}"#,
        r#"{"type":"object","properties":{"x":{"type":"string","enum":["a","a"]}}}"#,
        r#"{"type":"object","properties":{"x":{"type":"boolean"}},"required":["x","x"]}"#,
        r#"{"type":"object","properties":{"x":{"type":"string","format":"future"}}}"#,
        r#"{"type":"object","properties":{"x":{"type":"string","minLength":1.0}}}"#,
        r#"{"type":"object","properties":{"x":{"type":"number","minimum":2,"maximum":1}}}"#,
        r#"{"type":"object","properties":{"x":{"type":"number","multipleOf":0}}}"#,
        r#"{"type":"object","properties":{"x":{"type":"string","oneOf":[{"const":"a","title":"A","description":"not-on-mcp"}]}}}"#,
    ] {
        assert!(form(schema).is_err(), "{schema}");
    }
}

#[test]
fn patterns_formats_and_secret_field_classification_match_producer() {
    let request = form(r#"{"type":"object","properties":{"glyph":{"type":"string","pattern":"^.{1}$"},"space":{"type":"string","pattern":"^\\s$"}},"required":["glyph","space"]}"#).unwrap();
    assert!(
        request
            .validate_response(&json(
                r#"{"action":"accept","content":{"glyph":"π","space":"\u00a0"}}"#
            ))
            .is_ok()
    );
    assert!(
        request
            .validate_response(&json(
                r#"{"action":"accept","content":{"glyph":"πa","space":"\u00a0"}}"#
            ))
            .is_err()
    );
    assert!(
        form(r#"{"type":"object","properties":{"x":{"type":"string","pattern":"(?=x)"}}}"#)
            .is_err()
    );
    for secret in [
        "token",
        "client_secret",
        "paymentCredential",
        "PASSWORD",
        "user-passcode",
        "otp",
        "userPin",
        "api key",
        "APIKey",
        "access.token",
        "privateKey",
        "credit-card",
    ] {
        assert!(
            form(&format!(
                "{{\"type\":\"object\",\"properties\":{{\"{secret}\":{{\"type\":\"string\"}}}}}}"
            ))
            .is_err()
        );
    }
    assert!(form(r#"{"type":"object","properties":{"shipping_address":{"type":"string","description":"Do not provide passwords or private keys"}}}"#).is_ok());
    for (format, good, bad) in [
        ("email", "user@example.test", "bad"),
        ("date", "2024-02-29", "2023-02-29"),
        (
            "date-time",
            "2026-08-02T12:34:56.123-04:00",
            "2026-08-02T99:34:56Z",
        ),
        ("uri", "urn:test", "relative"),
    ] {
        let request = form(&format!("{{\"type\":\"object\",\"properties\":{{\"x\":{{\"type\":\"string\",\"format\":\"{format}\"}}}}}}")).unwrap();
        for (value, valid) in [(good, true), (bad, false)] {
            assert_eq!(
                request
                    .validate_response(&json(&format!(
                        "{{\"action\":\"accept\",\"content\":{{\"x\":\"{value}\"}}}}"
                    )))
                    .is_ok(),
                valid
            );
        }
    }
}

#[test]
fn legacy_modes_url_required_and_canonical_responses_stay_distinct() {
    let params = json(
        r#"{"message":"Choose","requestedSchema":{"type":"object","properties":{"choice":{"type":"string","enum":["a"],"enumNames":["A"]}}}}"#,
    );
    assert!(
        McpElicitationRequest::parse(
            &params,
            ProtocolVersion::Legacy20250618,
            McpMrtrLimits::default()
        )
        .is_ok()
    );
    let explicit = json(&params.get().replacen('{', "{\"mode\":\"form\",", 1));
    assert!(
        McpElicitationRequest::parse(
            &explicit,
            ProtocolVersion::Legacy20250618,
            McpMrtrLimits::default()
        )
        .is_err()
    );
    let data = json(
        r#"{"elicitations":[{"mode":"url","message":"Authorize","url":"https://example.test/connect","elicitationId":"url-1"}]}"#,
    );
    let required = McpInputRequired::parse_legacy_url_required(
        &data,
        ProtocolVersion::Legacy20251125,
        McpMrtrLimits::default(),
    )
    .unwrap();
    assert!(required.legacy_retry_without_responses());
    assert_eq!(required.requests()[0].key(), "url-1");
    assert!(required.request_state_json().is_none());
    assert!(
        required
            .render_requests_json()
            .unwrap()
            .get()
            .contains("elicitation/create")
    );
    for version in [
        ProtocolVersion::Modern,
        ProtocolVersion::Legacy20250618,
        ProtocolVersion::Legacy20241105,
    ] {
        assert!(
            McpInputRequired::parse_legacy_url_required(&data, version, McpMrtrLimits::default())
                .is_err()
        );
    }
    let response = required
        .validate_responses(&json(
            r#"{"url-1":{"action":"decline","content":{"ignored":"private"},"future":1e400}}"#,
        ))
        .unwrap();
    assert_eq!(
        response.wire_json().get(),
        r#"{"url-1":{"action":"decline"}}"#
    );
    assert!(response.raw_json().get().contains("private"));
    assert!(
        required
            .validate_responses(&json(r#"{"url-1":{"action":"accept","content":null}}"#))
            .is_err()
    );
    let duplicate = json(&data.get().replace("]}", ", {\"mode\":\"url\",\"message\":\"Again\",\"url\":\"https://example.test\",\"elicitationId\":\"url-1\"}]}"));
    assert!(
        McpInputRequired::parse_legacy_url_required(
            &duplicate,
            ProtocolVersion::Legacy20251125,
            McpMrtrLimits::default()
        )
        .is_err()
    );
}

#[test]
fn url_validation_preserves_raw_host_and_never_normalizes_loopback_authority() {
    for url in [
        "https://example.test/path?state=opaque",
        "http://localhost:4321/callback",
        "http://127.0.0.1",
        "http://[::1]",
        "https://xn--e1awd7f.test",
        "https://exämple.test",
    ] {
        let request = elicitation(&format!(
            "{{\"mode\":\"url\",\"message\":\"Authorize\",\"url\":\"{url}\"}}"
        ))
        .unwrap();
        assert_eq!(request.url(), Some(url));
    }
    for url in [
        "http://example.test",
        "http://127.1",
        "http://2130706433",
        "https://user@example.test",
        " https://example.test",
        "https://",
        "file:///tmp",
    ] {
        assert!(
            elicitation(&format!(
                "{{\"mode\":\"url\",\"message\":\"Authorize\",\"url\":\"{url}\"}}"
            ))
            .is_err(),
            "{url}"
        );
    }
    assert_eq!(
        strings::classify_host(b"XN--e1awd7f.test"),
        McpHostClassification::Punycode
    );
    assert_eq!(
        strings::classify_host("exämple.test".as_bytes()),
        McpHostClassification::NonAscii
    );
    assert!(elicitation(r#"{"mode":"url","message":"Authorize","url":"https://example.test","elicitationId":"not-modern"}"#).is_err());
}

#[test]
fn direct_entrypoints_enforce_duplicate_node_byte_and_inclusive_retained_bounds() {
    let raw = json(r#"{"requestState":null}"#);
    let required = McpInputRequired::parse(&raw, McpMrtrLimits::default()).unwrap();
    let limits = McpMrtrLimits {
        max_retained_bytes: required.retained_byte_charge(),
        ..McpMrtrLimits::default()
    };
    assert!(McpInputRequired::parse(&raw, limits).is_ok());
    assert!(
        McpInputRequired::parse(
            &raw,
            McpMrtrLimits {
                max_retained_bytes: limits.max_retained_bytes - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        McpInputRequired::parse(
            &raw,
            McpMrtrLimits {
                max_json_bytes: raw.get().len(),
                ..limits
            }
        )
        .is_ok()
    );
    assert!(
        McpInputRequired::parse(
            &raw,
            McpMrtrLimits {
                max_json_bytes: raw.get().len() - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        McpInputRequired::parse(
            &raw,
            McpMrtrLimits {
                max_nodes: 2,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        McpInputRequired::parse(
            &raw,
            McpMrtrLimits {
                max_requests: 0,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        elicitation(r#"{"mode":"url","mode":"form","message":"x","url":"https://example.test"}"#)
            .is_err()
    );
    let request = form(r#"{"type":"object","properties":{"x":{"type":"number"}}}"#).unwrap();
    assert!(
        request
            .validate_response(&json(r#"{"action":"accept","content":{"x":1,"x":2}}"#))
            .is_err()
    );
    assert!(
        request.form_schema().unwrap().fields()[0]
            .validate_json(&json(r#"{"x":1,"x":2}"#), McpMrtrLimits::default())
            .is_err()
    );
}

#[test]
fn standalone_and_nested_elicitation_limits_remain_distinct() {
    for (size, direct) in [(8192, true), (8193, false)] {
        let params = format!(
            "{{\"message\":\"{}\",\"requestedSchema\":{{\"type\":\"object\",\"properties\":{{}}}}}}",
            "m".repeat(size)
        );
        assert_eq!(elicitation(&params).is_ok(), direct);
        assert!(input(&format!("{{\"inputRequests\":{{\"f\":{{\"method\":\"elicitation/create\",\"params\":{params}}}}}}}")).is_ok());
    }
    let properties = (0..65)
        .map(|index| format!("\"field{index}\":{{\"type\":\"boolean\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    let params = format!(
        "{{\"message\":\"Fields\",\"requestedSchema\":{{\"type\":\"object\",\"properties\":{{{properties}}}}}}}"
    );
    assert!(elicitation(&params).is_err());
    assert!(input(&format!("{{\"inputRequests\":{{\"f\":{{\"method\":\"elicitation/create\",\"params\":{params}}}}}}}")).is_ok());
    let legacy = json(&format!(
        "{{\"elicitations\":[{{\"mode\":\"url\",\"message\":\"{}\",\"url\":\"https://example.test\",\"elicitationId\":\"id\"}}]}}",
        "m".repeat(8193)
    ));
    assert!(
        McpInputRequired::parse_legacy_url_required(
            &legacy,
            ProtocolVersion::Legacy20251125,
            McpMrtrLimits::default()
        )
        .is_err()
    );
    let legacy = json(
        r#"{"elicitations":[{"mode":"url","message":"","url":"https://example.test","elicitationId":""}]}"#,
    );
    assert!(
        McpInputRequired::parse_legacy_url_required(
            &legacy,
            ProtocolVersion::Legacy20251125,
            McpMrtrLimits::default()
        )
        .is_err()
    );
}

#[test]
fn pinned_uri_parser_retains_non_utf8_and_literal_percent_hosts() {
    let request = elicitation(r#"{"mode":"url","message":"","url":"https://%FF.test"}"#).unwrap();
    assert_eq!(request.url_host_bytes(), Some(b"\xff.test".as_slice()));
    assert_eq!(request.url_host(), None);
    assert_eq!(
        request.host_classification(),
        Some(McpHostClassification::NonAscii)
    );
    assert_eq!(
        strings::url_host("https://bad%zz").unwrap().as_ref(),
        b"bad%zz"
    );
    assert!(strings::url_host(&format!("https://{}", "x".repeat(256))).is_ok());
    assert!(strings::url_host(&format!("https://{}%78", "x".repeat(255))).is_err());
    assert!(strings::url_host(&format!("https://{}%78", "x".repeat(254))).is_ok());
    for valid in [
        "urn:test",
        "file:///tmp",
        "http://user@",
        "https://[::1]ignored",
        "http://x:8_0",
    ] {
        assert!(strings::uri(valid), "{valid}");
    }
    for invalid in [
        "relative",
        "http://",
        "http://]bad",
        "http://[",
        "http://x:65536",
        "http://x:no",
    ] {
        assert!(!strings::uri(invalid), "{invalid}");
    }
}

#[test]
fn request_collection_number_and_pattern_boundaries_are_inclusive() {
    for (count, valid) in [(32, true), (33, false)] {
        let entries = (0..count)
            .map(|index| format!("\"r{index}\":{{\"method\":\"roots/list\"}}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            input(&format!("{{\"inputRequests\":{{{entries}}}}}")).is_ok(),
            valid
        );
    }
    for (count, valid) in [(256, true), (257, false)] {
        let array = vec!["null"; count].join(",");
        assert_eq!(
            input(&format!("{{\"requestState\":[{array}]}}")).is_ok(),
            valid
        );
    }
    for (count, valid) in [(4096, true), (4097, false)] {
        let number = format!("1{}", "0".repeat(count - 1));
        assert_eq!(form(&format!("{{\"type\":\"object\",\"properties\":{{\"x\":{{\"type\":\"number\",\"minimum\":{number}}}}}}}")).is_ok(), valid);
    }
    for (exponent, valid) in [(1_000_000, true), (1_000_001, false)] {
        assert_eq!(form(&format!("{{\"type\":\"object\",\"properties\":{{\"x\":{{\"type\":\"number\",\"minimum\":1e{exponent}}}}}}}")).is_ok(), valid);
    }
    let params = json(
        r#"{"message":"Pattern","requestedSchema":{"type":"object","properties":{"x":{"type":"string","pattern":"^a$"}}}}"#,
    );
    assert!(
        McpElicitationRequest::parse(
            &params,
            ProtocolVersion::Modern,
            McpMrtrLimits {
                max_pattern_states: 1,
                ..McpMrtrLimits::default()
            }
        )
        .is_err()
    );
    let request = McpElicitationRequest::parse(
        &params,
        ProtocolVersion::Modern,
        McpMrtrLimits {
            max_pattern_steps: 1,
            ..McpMrtrLimits::default()
        },
    )
    .unwrap();
    assert!(
        request
            .validate_response(&json(r#"{"action":"accept","content":{"x":"a"}}"#))
            .is_err()
    );
    let request =
        form(r#"{"type":"object","properties":{"x":{"type":"number","multipleOf":1e-10000}}}"#)
            .unwrap();
    assert!(
        request
            .validate_response(&json(r#"{"action":"accept","content":{"x":1}}"#))
            .is_err()
    );
    let request =
        form(r#"{"type":"object","properties":{"x":{"type":"integer","minimum":0,"maximum":0}}}"#)
            .unwrap();
    assert!(
        request
            .validate_response(&json(r#"{"action":"accept","content":{"x":-0}}"#))
            .is_ok()
    );
}

#[test]
fn strict_shared_wire_move_preserves_exact_values_without_private_key_coercion() {
    let raw = br#"{"jsonrpc":"2.0","id":1,"result":{"$serde_json::private::Number":1e400,"negativeZero":-0}}"#;
    let envelope =
        crate::mcp::protocol::parse_envelope(raw, crate::mcp::protocol::WireLimits::default())
            .unwrap();
    let value = envelope.into_value();
    assert!(value["result"].is_object());
    let rendered = serde_json::to_string(&value).unwrap();
    assert!(rendered.contains("1e400") && rendered.contains("-0"));
}

#[test]
fn request_and_form_presentation_order_remains_source_order() {
    let required = input(r#"{"inputRequests":{"z":{"method":"roots/list"},"a":{"method":"elicitation/create","params":{"message":"","requestedSchema":{"type":"object","properties":{"z":{"type":"boolean"},"a":{"type":"boolean"}}}}}}}"#).unwrap();
    assert_eq!(
        required
            .requests()
            .iter()
            .map(McpInputRequest::key)
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    assert!(
        required
            .render_requests_json()
            .unwrap()
            .get()
            .starts_with(r#"{"z":{"method":"roots/list"},"a":"#)
    );
    let McpInputRequestPayload::Elicitation(request) = required.requests()[1].payload() else {
        panic!("expected form");
    };
    assert_eq!(
        request
            .form_schema()
            .unwrap()
            .fields()
            .iter()
            .map(McpFormField::name)
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    assert!(!strings::uri("http://x:_80"));
    assert!(!strings::uri("http://x:80_"));
    assert!(strings::format("date", "+024-+1-+1"));
    assert_eq!(
        strings::url_host("https://%+a.test").unwrap().as_ref(),
        b"\n.test"
    );
}
