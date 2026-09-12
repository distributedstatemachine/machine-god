use super::*;
use crate::mcp::{
    endpoint::McpEndpoint,
    mrtr::{McpInputRequired, McpMrtrLimits},
};
use serde_json::value::RawValue;

fn claimed(transport: TransportKind) -> (Fixture, McpSubmission) {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let request = fixture.request("call");
    let (_, lease) = McpPendingToolReservation::leased(RpcId::Integer(42));
    let mut projection = project(
        &fixture,
        &schema,
        &request,
        options(ProtocolVersion::Modern, transport)
            .with_progress_token(87)
            .with_elicitation(true, false),
    )
    .unwrap()
    .with_reservation(lease)
    .unwrap();
    if transport != TransportKind::Stdio {
        projection = projection
            .with_http_head(
                &McpSubmissionHttpHead::new(
                    &McpEndpoint::parse("https://example.test/mcp").unwrap(),
                    &[
                        ("authorization", b"Bearer original"),
                        (
                            "mcp-protocol-version",
                            ProtocolVersion::Modern.as_str().as_bytes(),
                        ),
                    ],
                )
                .unwrap(),
            )
            .unwrap();
    }
    let prepared = prepare(&fixture, &request, projection).unwrap();
    fixture.admission("call", prepared).admit().unwrap();
    let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    (fixture, submission)
}
fn required() -> McpInputRequired {
    let source = format!(
        r#"{{"inputRequests":{{"answer":{{"method":"elicitation/create","params":{{"message":"Answer","requestedSchema":{{"type":"object","properties":{{"value":{{"type":"string"}}}}}}}}}}}},"requestState":{{"a":"{}","b":"{}","exact":1e-99999,"zero":-0,"$serde_json::private::Number":"literal"}}}}"#,
        "x".repeat(60_000),
        "y".repeat(60_000)
    );
    McpInputRequired::parse(
        &RawValue::from_string(source).unwrap(),
        McpMrtrLimits::default(),
    )
    .unwrap()
}

#[test]
fn typed_continuation_keeps_exact_original_proof_head_args_and_separate_large_wire_bound() {
    for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
        let (fixture, original) = claimed(transport);
        let custody = original.continuation_custody().unwrap();
        let required = required();
        let raw = RawValue::from_string(format!(
            r#"{{"answer":{{"action":"accept","content":{{"value":"{}"}}}}}}"#,
            "z".repeat(60_000)
        ))
        .unwrap();
        let responses = required.validate_responses(&raw).unwrap();
        let (_, lease) = McpPendingToolReservation::leased(RpcId::Integer(43));
        let next = custody
            .prepare(lease, &responses, required.request_state_json())
            .unwrap();
        assert!(Arc::ptr_eq(&next.ready.proof, &original.ready.proof));
        assert!(Arc::ptr_eq(
            &next.ready.data.runtime,
            &original.ready.data.runtime
        ));
        assert_eq!(next.ready.data.arguments, original.ready.data.arguments);
        assert_eq!(
            next.permission_request_id(),
            original.permission_request_id()
        );
        assert_eq!(next.rpc_id(), &RpcId::Integer(43));
        assert_eq!(
            next.tool_options(),
            Some(
                McpToolCallOptions::new(
                    NegotiatedProtocol {
                        transport,
                        version: ProtocolVersion::Modern
                    },
                    43
                )
                .unwrap()
                .with_progress_token(43)
                .with_elicitation(true, false)
            )
        );
        let wire = std::str::from_utf8(&next.ready.data.wire).unwrap();
        let body = if transport == TransportKind::Stdio {
            wire.trim_end_matches('\n')
        } else {
            assert!(Arc::ptr_eq(
                &next.http_head().unwrap(),
                &original.http_head().unwrap()
            ));
            let (head, body) = wire.split_once("\r\n\r\n").unwrap();
            assert!(head.contains("authorization: Bearer original"));
            assert!(head.contains(&format!("content-length: {}\r\n", body.len())));
            body
        };
        assert!(body.len() > MAX_MCP_SUBMISSION_REQUEST_BYTES);
        assert!(body.len() < 384 * 1024);
        assert!(body.contains("1e-99999") && body.contains("\"zero\":-0"));
        let params = machine_god_core::json::from_str(body).unwrap();
        assert_eq!(params["params"]["arguments"], json!({"secret":1}));
        assert!(params["params"].get("inputResponses").is_some());
        assert!(matches!(
            block_on(fixture.prepare_request(
                &fixture.request("other"),
                body.as_bytes(),
                CancellationToken::new()
            )),
            Err(McpSubmissionError::Limit)
        ));
        assert!(block_on(fixture.claim("call", CancellationToken::new())).is_err());
        fixture.revoke();
        assert!(custody.revalidate().is_err());
        assert!(next.checkpoint().is_err());
    }
}
