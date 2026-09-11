use super::*;
use crate::mcp::{
    catalog::{McpDescriptor, McpDescriptorCatalog, McpDescriptorLimits},
    feature::content,
    pagination::{McpCatalogBuilder, McpCatalogKind, McpCatalogLimits},
    protocol::TransportKind,
    submission::{McpSubmissionRuntimeBinding, McpSubmissionRuntimeOwner},
};
use machine_god_core::{SessionId, SessionIncarnationId, ToolCallId, TurnId};

fn context(schema: Option<&str>, version: ProtocolVersion) -> McpToolResponseContext {
    let schema = schema.map_or_else(String::new, |value| format!(",\"outputSchema\":{value}"));
    let page = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"tools":[{{"name":"remote","inputSchema":{{"type":"object"}}{schema}}}]}}}}"#
    );
    let mut builder =
        McpCatalogBuilder::new(McpCatalogKind::Tools, version, McpCatalogLimits::default())
            .unwrap();
    builder
        .append_response(page.as_bytes(), &RpcId::Integer(1), None, 0)
        .unwrap();
    let catalog =
        McpDescriptorCatalog::admit(builder.finish().unwrap(), McpDescriptorLimits::default())
            .unwrap();
    let McpDescriptor::Tool(descriptor) = &catalog.descriptors()[0] else {
        panic!()
    };
    let tool = ToolName::new("mcp_remote").unwrap();
    let owner = McpSubmissionRuntimeOwner::new();
    let runtime = owner
        .install(
            McpSubmissionRuntimeBinding::new(
                "secret-server",
                tool.clone(),
                "remote",
                b"secret-config",
                descriptor.input_schema().raw_json().as_bytes(),
                b"secret-auth",
            )
            .unwrap(),
        )
        .unwrap();
    McpToolResponseContext::new(
        ToolContext {
            session_id: SessionId::new("session").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("turn").unwrap(),
            call_id: ToolCallId::new("call").unwrap(),
        },
        tool,
        "secret-server".into(),
        descriptor.clone(),
        runtime,
        NegotiatedProtocol {
            transport: TransportKind::Stdio,
            version,
        },
        RpcId::Integer(7),
    )
    .unwrap()
}

fn admit(result: &str, schema: Option<&str>) -> Result<McpToolResponseDisposition> {
    let raw = format!(r#"{{"jsonrpc":"2.0","id":7,"result":{result}}}"#);
    NativeMcpToolResultAdmission::new(McpToolResultLimits::default())
        .unwrap()
        .admit(context(schema, ProtocolVersion::Modern), raw.as_bytes())
}

#[test]
fn complete_result_keeps_exact_numbers_unknown_fields_and_tool_failure() {
    let result = r#"{"content":[{"type":"text","text":"hello"}],"isError":true,"structuredContent":{"n":9007199254740993.00000001},"unknown":{"$serde_json::private::Number":"literal","wide":1e400,"zero":-0}}"#;
    let McpToolResponseDisposition::Complete(output) = admit(result, None).unwrap() else {
        panic!()
    };
    assert!(output.is_error);
    assert_eq!(output.content["content"][0]["text"], "hello");
    assert_eq!(
        output.content["unknown"]["$serde_json::private::Number"],
        "literal"
    );
    let encoded = serde_json::to_string(&output).unwrap();
    for number in ["9007199254740993.00000001", "1e400", "-0"] {
        assert!(encoded.contains(number));
    }
}

#[test]
fn output_schema_is_enforced_even_for_tool_failures_without_rounding() {
    let schema = r#"{"type":"object","properties":{"n":{"const":9007199254740993.00000001}},"required":["n"]}"#;
    let valid =
        r#"{"content":[],"structuredContent":{"n":9007199254740993.00000001},"isError":true}"#;
    assert!(matches!(
        admit(valid, Some(schema)),
        Ok(McpToolResponseDisposition::Complete(_))
    ));
    for invalid in [
        r#"{"content":[],"isError":true}"#,
        r#"{"content":[],"structuredContent":{"n":9007199254740993.00000002}}"#,
        r#"{"content":[],"structuredContent":null}"#,
    ] {
        assert!(matches!(
            admit(invalid, Some(schema)),
            Err(Error::InvalidStructuredContent)
        ));
    }
    assert!(matches!(
        admit(r#"{"content":[],"structuredContent":{}}"#, Some("false")),
        Err(Error::InvalidStructuredContent)
    ));
    // Unsupported pattern dialect remains server-authoritative, not locally valid.
    assert!(
        admit(
            r#"{"content":[],"structuredContent":"value"}"#,
            Some(r#"{"type":"string","pattern":"(?=value)"}"#)
        )
        .is_ok()
    );
}

#[test]
fn strict_envelopes_reject_foreign_ids_duplicate_keys_and_invalid_shapes() {
    let decoder = NativeMcpToolResultAdmission::new(McpToolResultLimits::default()).unwrap();
    for bytes in [
        r#"{"jsonrpc":"2.0","id":8,"result":{"content":[]}}"#,
        r#"{"jsonrpc":"2.0","id":null,"result":{"content":[]}}"#,
        r#"{"jsonrpc":"2.0","id":7,"result":{"content":[],"content":[]}}"#,
        r#"{"jsonrpc":"2.0","id":7,"result":{"content":[]},"error":{"code":1,"message":"x"}}"#,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{}}"#,
    ] {
        assert!(
            decoder
                .admit(context(None, ProtocolVersion::Modern), bytes.as_bytes())
                .is_err()
        );
    }
    for result in [
        "{}",
        r#"{"content":null}"#,
        r#"{"content":[],"isError":"false"}"#,
        r#"{"content":[],"resultType":"other"}"#,
        r#"{"content":[],"resultType":null}"#,
        r#"{"content":[{"type":"image","data":"invalid!","mimeType":"image/png"}]}"#,
    ] {
        assert!(admit(result, None).is_err());
    }
}

#[test]
fn all_content_kinds_follow_tool_not_feature_empty_string_policy() {
    for item in [
        r#"{"type":"text","text":""}"#,
        r#"{"type":"image","data":"AA==","mimeType":""}"#,
        r#"{"type":"audio","data":"","mimeType":""}"#,
        r#"{"type":"resource_link","uri":"","name":"","icons":[{"src":""}]}"#,
        r#"{"type":"resource","resource":{"uri":"","text":"","annotations":false}}"#,
    ] {
        assert!(matches!(
            admit(&format!("{{\"content\":[{item}]}}"), None),
            Ok(McpToolResponseDisposition::Complete(_))
        ));
    }
    for item in [
        r#"{"type":"image","data":"AA==","mimeType":""}"#,
        r#"{"type":"resource_link","uri":"","name":""}"#,
        r#"{"type":"resource","resource":{"uri":"ok","text":"","annotations":false}}"#,
    ] {
        let raw: &RawValue = serde_json::from_str(item).unwrap();
        assert!(content::admit(raw, McpFeatureCodecLimits::default(), &mut 0).is_err());
        assert!(
            content::admit_with_policy(
                raw,
                McpFeatureCodecLimits::default(),
                &mut 0,
                content::Policy::Tool
            )
            .is_ok()
        );
    }
}

#[test]
fn tool_uri_bounds_and_complete_result_budget_are_independent_of_feature_policy() {
    let uri = "x".repeat(64 * 1024 + 1);
    let item = format!(
        r#"{{"type":"resource_link","uri":"{uri}","name":"name","icons":[{{"src":"{uri}"}}]}}"#
    );
    let raw: &RawValue = serde_json::from_str(&item).unwrap();
    assert!(content::admit(raw, McpFeatureCodecLimits::default(), &mut 0).is_err());
    assert!(admit(&format!("{{\"content\":[{item}]}}"), None).is_ok());
    let limits = McpToolResultLimits {
        max_result_bytes: 14,
        ..McpToolResultLimits::default()
    };
    let decoder = NativeMcpToolResultAdmission::new(limits).unwrap();
    let output = br#"{"jsonrpc":"2.0","id":7,"result":{"content":[]}}"#;
    assert!(
        decoder
            .admit(context(None, ProtocolVersion::Modern), output)
            .is_ok()
    );
    let output = br#"{"jsonrpc":"2.0","id":7,"result":{"content":[],"unknown":1}}"#;
    assert!(matches!(
        decoder.admit(context(None, ProtocolVersion::Modern), output),
        Err(Error::Limit)
    ));
    let many = format!(
        "{{\"content\":[{}]}}",
        std::iter::repeat_n(r#"{"type":"text","text":""}"#, 257)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(matches!(admit(&many, None), Err(Error::Limit)));
}

#[test]
fn input_required_retains_exact_context_and_is_not_a_complete_result() {
    let decoder = NativeMcpToolResultAdmission::new(McpToolResultLimits::default()).unwrap();
    let selected = context(None, ProtocolVersion::Modern);
    let runtime = selected.runtime().clone();
    let response = br#"{"jsonrpc":"2.0","id":7,"result":{"resultType":"input_required","requestState":{"n":9007199254740993.0}}}"#;
    let McpToolResponseDisposition::InputRequired(custody) =
        decoder.admit(selected, response).unwrap()
    else {
        panic!()
    };
    assert!(Arc::ptr_eq(custody.context().runtime(), &runtime));
    assert_eq!(custody.context().tool_context().call_id.as_str(), "call");
    assert_eq!(custody.context().request_id(), &RpcId::Integer(7));
    assert!(
        custody
            .required()
            .request_state_json()
            .unwrap()
            .get()
            .contains("9007199254740993.0")
    );
    assert!(
        decoder
            .admit(context(None, ProtocolVersion::Legacy20251125), response)
            .is_err()
    );
    assert!(admit(r#"{"resultType":"input_required"}"#, None).is_err());
}

#[test]
fn protocol_failures_remain_distinct_and_do_not_expose_data_in_debug() {
    let decoder = NativeMcpToolResultAdmission::new(McpToolResultLimits::default()).unwrap();
    let response = br#"{"jsonrpc":"2.0","id":7,"error":{"code":-32042,"message":"secret-message","data":{"unknown":9007199254740993.0}}}"#;
    for version in [ProtocolVersion::Modern, ProtocolVersion::Legacy20251125] {
        let selected = context(None, version);
        assert!(!format!("{selected:?}").contains("secret-server"));
        let disposition = decoder.admit(selected, response).unwrap();
        assert!(!format!("{disposition:?}").contains("secret-message"));
        let McpToolResponseDisposition::ProtocolFailure(failure) = disposition else {
            panic!()
        };
        assert_eq!(failure.code(), -32042);
        assert_eq!(failure.message(), "secret-message");
        assert!(failure.raw_json().get().contains("9007199254740993.0"));
    }
}

#[test]
fn only_the_pinned_legacy_url_error_enters_input_custody() {
    let decoder = NativeMcpToolResultAdmission::new(McpToolResultLimits::default()).unwrap();
    let response = br#"{"jsonrpc":"2.0","id":7,"error":{"code":-32042,"message":"Authorize","data":{"elicitations":[{"mode":"url","message":"Continue","url":"https://example.test/connect","elicitationId":"url-1"}]}}}"#;
    let McpToolResponseDisposition::InputRequired(custody) = decoder
        .admit(context(None, ProtocolVersion::Legacy20251125), response)
        .unwrap()
    else {
        panic!()
    };
    assert!(custody.required().legacy_retry_without_responses());
    assert_eq!(custody.required().requests()[0].key(), "url-1");
    assert!(custody.required().request_state_json().is_none());
    for version in [ProtocolVersion::Modern, ProtocolVersion::Legacy20250618] {
        assert!(matches!(
            decoder.admit(context(None, version), response),
            Ok(McpToolResponseDisposition::ProtocolFailure(_))
        ));
    }
}

#[test]
fn every_selected_limit_is_enforced_and_cannot_be_enlarged() {
    let defaults = McpToolResultLimits::default();
    for limits in [
        McpToolResultLimits {
            max_response_bytes: 1,
            ..defaults
        },
        McpToolResultLimits {
            max_nodes: 1,
            ..defaults
        },
        McpToolResultLimits {
            max_retained_bytes: 4,
            ..defaults
        },
        McpToolResultLimits {
            max_content_field_bytes: 1,
            ..defaults
        },
        McpToolResultLimits {
            max_content_items: 1,
            ..defaults
        },
        McpToolResultLimits {
            max_result_bytes: 1,
            ..defaults
        },
    ] {
        let decoder = NativeMcpToolResultAdmission::new(limits).unwrap();
        assert!(decoder.admit(context(None, ProtocolVersion::Modern),
            br#"{"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"ab"},{"type":"text","text":"ab"}]}}"#).is_err());
    }
    for limits in [
        McpToolResultLimits {
            max_response_bytes: 0,
            ..defaults
        },
        McpToolResultLimits {
            max_nodes: defaults.max_nodes + 1,
            ..defaults
        },
        McpToolResultLimits {
            max_retained_bytes: defaults.max_retained_bytes + 1,
            ..defaults
        },
        McpToolResultLimits {
            max_content_field_bytes: defaults.max_content_field_bytes + 1,
            ..defaults
        },
        McpToolResultLimits {
            max_content_items: defaults.max_content_items + 1,
            ..defaults
        },
        McpToolResultLimits {
            max_result_bytes: defaults.max_result_bytes + 1,
            ..defaults
        },
    ] {
        assert!(matches!(
            NativeMcpToolResultAdmission::new(limits),
            Err(Error::InvalidLimits)
        ));
    }
}
