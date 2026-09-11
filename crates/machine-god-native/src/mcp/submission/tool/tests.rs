use super::*;
use crate::mcp::{
    protocol::{NegotiatedProtocol, ProtocolVersion, TransportKind},
    schema::{McpSchema, McpSchemaLimits},
};

fn fixture(raw: &str) -> (Fixture, McpSchema) {
    let schema = McpSchema::parse(raw.as_bytes(), McpSchemaLimits::default()).unwrap();
    let mut fixture = Fixture::new();
    fixture.runtime = fixture
        .runtime_owner
        .install(
            McpSubmissionRuntimeBinding::new(
                "secret-server",
                fixture.tool(),
                "secret-tool",
                b"secret-config",
                schema.raw_json().as_bytes(),
                b"secret-auth",
            )
            .unwrap(),
        )
        .unwrap();
    (fixture, schema)
}

fn invocation(request: &PermissionRequest) -> PermissionInvocation<'_> {
    let Capability::Tool {
        name,
        call_id,
        arguments,
    } = &request.capability
    else {
        panic!("expected tool invocation")
    };
    PermissionInvocation {
        tool_name: name,
        call_id,
        arguments,
    }
}

fn arguments(request: &mut PermissionRequest, value: Value) {
    let Capability::Tool { arguments, .. } = &mut request.capability else {
        panic!("expected tool invocation")
    };
    *arguments = value;
}

fn options(version: ProtocolVersion, transport: TransportKind) -> McpToolCallOptions {
    McpToolCallOptions::new(NegotiatedProtocol { transport, version }, 42).unwrap()
}

fn project(
    fixture: &Fixture,
    schema: &McpSchema,
    request: &PermissionRequest,
    options: McpToolCallOptions,
) -> Result<McpToolRequest> {
    McpToolRequest::new(
        fixture.runtime.clone(),
        schema,
        invocation(request),
        options,
    )
}

fn prepare(
    fixture: &Fixture,
    request: &PermissionRequest,
    projection: McpToolRequest,
) -> Result<PreparedMcpSubmission> {
    block_on(fixture.registry.prepare_tool(
        request,
        invocation(request),
        projection,
        CancellationToken::new(),
    ))
}

#[test]
fn typed_options_survive_preparation_admission_and_claim() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let request = fixture.request("call");
    let selected = options(ProtocolVersion::Modern, TransportKind::Stdio)
        .with_progress_token(87)
        .with_elicitation(true, true);
    let prepared = prepare(
        &fixture,
        &request,
        project(&fixture, &schema, &request, selected).unwrap(),
    )
    .unwrap();
    assert_eq!(prepared.data.tool_options, Some(selected));
    fixture.admission("call", prepared).admit().unwrap();
    let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    assert_eq!(submission.tool_options(), Some(selected));
}

#[test]
fn typed_peer_reservation_survives_each_admission_stage_and_releases_on_drop() {
    for stage in 0..6 {
        let (mut fixture, schema) = fixture(r#"{"type":"object"}"#);
        let request = fixture.request("call");
        let (pending, lease) = McpPendingToolReservation::leased(RpcId::Integer(42));
        let projection = project(
            &fixture,
            &schema,
            &request,
            options(ProtocolVersion::Modern, TransportKind::Stdio),
        )
        .unwrap()
        .with_reservation(lease)
        .unwrap();
        assert!(pending.is_live());
        if stage == 0 {
            drop(projection);
        } else {
            let future = fixture.registry.prepare_tool(
                &request,
                invocation(&request),
                projection,
                CancellationToken::new(),
            );
            assert!(pending.is_live());
            if stage == 1 {
                drop(future);
            } else {
                let prepared = block_on(future).unwrap();
                assert!(pending.is_live());
                if stage == 2 {
                    drop(prepared);
                } else {
                    let admission = fixture.admission("call", prepared);
                    assert!(pending.is_live());
                    if stage == 3 {
                        drop(admission);
                    } else {
                        admission.admit().unwrap();
                        assert!(pending.is_live());
                        if stage == 4 {
                            let submission =
                                block_on(fixture.claim("call", CancellationToken::new())).unwrap();
                            assert!(
                                pending.matches(submission.rpc_id(), submission.tool_reservation())
                            );
                            drop(submission);
                        } else {
                            fixture.close();
                        }
                    }
                }
            }
        }
        assert!(!pending.is_live(), "reservation retained at stage {stage}");
    }
}

#[test]
fn peer_reservation_attachment_rejects_wrong_id_and_replacement() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let request = fixture.request("call");
    let project = || {
        project(
            &fixture,
            &schema,
            &request,
            options(ProtocolVersion::Modern, TransportKind::Stdio),
        )
        .unwrap()
    };
    let (wrong_pending, wrong) = McpPendingToolReservation::leased(RpcId::Integer(43));
    assert!(project().with_reservation(wrong).is_err());
    assert!(!wrong_pending.is_live());
    let (pending, lease) = McpPendingToolReservation::leased(RpcId::Integer(42));
    let (other_pending, other) = McpPendingToolReservation::leased(RpcId::Integer(42));
    assert!(
        project()
            .with_reservation(lease)
            .unwrap()
            .with_reservation(other)
            .is_err()
    );
    assert!(!pending.is_live());
    assert!(!other_pending.is_live());
}

#[test]
fn modern_and_legacy_envelopes_use_only_selected_metadata() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    for (index, version) in [
        ProtocolVersion::Modern,
        ProtocolVersion::Legacy20251125,
        ProtocolVersion::Legacy20250618,
        ProtocolVersion::Legacy20241105,
    ]
    .into_iter()
    .enumerate()
    {
        let request = fixture.request(&format!("call-{index}"));
        let projection = project(
            &fixture,
            &schema,
            &request,
            options(version, TransportKind::Stdio),
        )
        .unwrap();
        assert_eq!(projection.schema().raw_json(), schema.raw_json());
        let prepared = prepare(&fixture, &request, projection).unwrap();
        let wire = &prepared.data.wire;
        assert_eq!(wire.last(), Some(&b'\n'));
        let envelope: Value = serde_json::from_slice(wire).unwrap();
        assert_eq!(envelope["id"], 42);
        assert_eq!(envelope["params"]["name"], "secret-tool");
        assert_eq!(envelope["params"]["arguments"], json!({"secret":1}));
        if version == ProtocolVersion::Modern {
            let meta = &envelope["params"]["_meta"];
            assert_eq!(
                meta["io.modelcontextprotocol/protocolVersion"],
                version.as_str()
            );
            assert_eq!(
                meta["io.modelcontextprotocol/clientInfo"]["name"],
                "machine-god"
            );
            assert_eq!(
                meta["io.modelcontextprotocol/clientCapabilities"],
                json!({})
            );
            assert!(meta.get("progressToken").is_none());
        } else {
            assert!(envelope["params"].get("_meta").is_none());
        }
    }
}

#[test]
fn progress_and_elicitation_advertisements_do_not_add_continuation_payloads() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    for version in [ProtocolVersion::Modern, ProtocolVersion::Legacy20251125] {
        let request = fixture.request("call");
        let projection = project(
            &fixture,
            &schema,
            &request,
            options(version, TransportKind::Stdio)
                .with_progress_token(17)
                .with_elicitation(true, false),
        )
        .unwrap();
        let prepared = prepare(&fixture, &request, projection).unwrap();
        let envelope: Value = serde_json::from_slice(&prepared.data.wire).unwrap();
        let params = &envelope["params"];
        assert!(params.get("inputResponses").is_none());
        assert!(params.get("requestState").is_none());
        let meta = &params["_meta"];
        assert_eq!(meta["progressToken"], 17);
        if version == ProtocolVersion::Modern {
            assert_eq!(
                meta["io.modelcontextprotocol/clientCapabilities"],
                json!({"elicitation":{"form":{}}})
            );
        } else {
            assert_eq!(meta, &json!({"progressToken":17}));
        }
    }
}

#[test]
fn schema_identity_validation_and_runtime_retirement_precede_reservation() {
    let (fixture, schema) = fixture(r#"{"type":"object","properties":{"secret":{"const":1}}}"#);
    let mut request = fixture.request("call");
    let opts = options(ProtocolVersion::Modern, TransportKind::Stdio);
    let other = McpSchema::parse(br#"{"type":"object"}"#, McpSchemaLimits::default()).unwrap();
    assert!(matches!(
        project(&fixture, &other, &request, opts),
        Err(McpSubmissionError::Denied)
    ));
    arguments(&mut request, json!({"secret":2}));
    assert!(matches!(
        project(&fixture, &schema, &request, opts),
        Err(McpSubmissionError::Invalid)
    ));
    arguments(&mut request, json!({"secret":1}));
    let projection = project(&fixture, &schema, &request, opts).unwrap();
    let future = fixture.registry.prepare_tool(
        &request,
        invocation(&request),
        projection,
        CancellationToken::new(),
    );
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    drop(future);
    assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    fixture.runtime_owner.retire();
    assert!(project(&fixture, &schema, &request, opts).is_err());
}

#[test]
fn typed_projection_cannot_be_rebound_to_different_arguments_or_call() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let request = fixture.request("call");
    for changed_call in [false, true] {
        let projection = project(
            &fixture,
            &schema,
            &request,
            options(ProtocolVersion::Modern, TransportKind::Stdio),
        )
        .unwrap();
        let mut changed = if changed_call {
            fixture.request("other")
        } else {
            request.clone()
        };
        if !changed_call {
            arguments(&mut changed, json!({"secret":2}));
        }
        assert!(matches!(
            prepare(&fixture, &changed, projection),
            Err(McpSubmissionError::Denied)
        ));
        assert!(fixture.registry.state.lock().unwrap().slots.is_empty());
    }
}

#[test]
fn optional_top_level_header_null_is_validation_only_and_required_null_stays_invalid() {
    for required in [false, true] {
        let raw = format!(
            r#"{{"type":"object","properties":{{"secret":{{"type":"integer","x-mcp-header":"Secret"}}}},"required":{}}}"#,
            if required { r#"["secret"]"# } else { "[]" }
        );
        let (fixture, schema) = fixture(&raw);
        let mut request = fixture.request("call");
        arguments(&mut request, json!({"secret":null}));
        let projection = project(
            &fixture,
            &schema,
            &request,
            options(ProtocolVersion::Modern, TransportKind::Stdio),
        );
        if required {
            assert!(matches!(projection, Err(McpSubmissionError::Invalid)));
        } else {
            let prepared = prepare(&fixture, &request, projection.unwrap()).unwrap();
            let envelope: Value = serde_json::from_slice(&prepared.data.wire).unwrap();
            assert_eq!(envelope["params"]["arguments"], json!({"secret":null}));
        }
    }
}

#[test]
fn typed_requests_still_require_concrete_one_shot_permission_admission() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let request = fixture.request("call");
    let projection = project(
        &fixture,
        &schema,
        &request,
        options(ProtocolVersion::Modern, TransportKind::Stdio),
    )
    .unwrap();
    let prepared = prepare(&fixture, &request, projection).unwrap();
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_err());
    fixture.admission("call", prepared).admit().unwrap();
    let submission = block_on(fixture.claim("call", CancellationToken::new())).unwrap();
    assert!(submission.belongs_to_runtime(&fixture.runtime));
    assert!(block_on(fixture.claim("call", CancellationToken::new())).is_err());
    drop(submission);
}

#[test]
fn application_ids_and_transport_versions_are_checked() {
    assert!(
        McpToolCallOptions::new(
            NegotiatedProtocol {
                transport: TransportKind::Stdio,
                version: ProtocolVersion::Modern
            },
            -1
        )
        .is_err()
    );
    assert!(
        McpToolCallOptions::new(
            NegotiatedProtocol {
                transport: TransportKind::LegacySse,
                version: ProtocolVersion::Modern
            },
            1
        )
        .is_err()
    );
    assert!(
        McpToolCallOptions::new(
            NegotiatedProtocol {
                transport: TransportKind::Stdio,
                version: ProtocolVersion::Legacy20250326
            },
            1
        )
        .is_err()
    );
}

#[test]
fn server_authoritative_schema_does_not_partially_validate_siblings() {
    let (fixture, schema) =
        fixture(r#"{"type":"object","properties":{"secret":{"type":"string","pattern":"(?=a)"}}}"#);
    let request = fixture.request("call");
    let projection = project(
        &fixture,
        &schema,
        &request,
        options(ProtocolVersion::Modern, TransportKind::Stdio),
    )
    .unwrap();
    assert_eq!(
        projection.schema().assessment(),
        crate::mcp::schema::McpSchemaAssessment::ServerAuthoritative
    );
    let prepared = prepare(&fixture, &request, projection).unwrap();
    let payload: Value = serde_json::from_slice(&prepared.data.wire).unwrap();
    assert_eq!(payload["params"]["arguments"], json!({"secret":1}));
}

#[test]
fn nested_optional_header_null_is_not_omitted_by_top_level_fallback() {
    let (fixture, schema) = fixture(
        r#"{"type":"object","properties":{"nested":{"type":"object","properties":{"secret":{"type":"integer","x-mcp-header":"Secret"}}}}}"#,
    );
    let mut request = fixture.request("call");
    arguments(&mut request, json!({"nested":{"secret":null}}));
    assert!(matches!(
        project(
            &fixture,
            &schema,
            &request,
            options(ProtocolVersion::Modern, TransportKind::Stdio)
        ),
        Err(McpSubmissionError::Invalid)
    ));
}

#[test]
fn exact_numeric_and_literal_private_keys_survive_permission_and_wire_projection() {
    let (fixture, schema) = fixture(r#"{"type":"object"}"#);
    let source = r#"{"$serde_json::private::Number":"literal","$serde_json::private::RawValue":"null","big":9007199254740993.0001,"zero":-0,"huge":1E+400,"tiny":1e-400}"#;
    let value = machine_god_core::json::from_str(source).unwrap();
    let mut request = fixture.request("call");
    arguments(&mut request, value.clone());
    let projection = project(
        &fixture,
        &schema,
        &request,
        options(ProtocolVersion::Modern, TransportKind::Stdio),
    )
    .unwrap();
    let prepared = prepare(&fixture, &request, projection).unwrap();
    let envelope = machine_god_core::json::from_slice(&prepared.data.wire).unwrap();
    let projected = &envelope["params"]["arguments"];
    assert_eq!(
        serde_json::to_string(projected).unwrap(),
        serde_json::to_string(&value).unwrap()
    );
    assert_eq!(projected["zero"].to_string(), "-0");
    assert_eq!(projected["huge"].to_string(), "1E+400");
    assert_eq!(projected["big"].to_string(), "9007199254740993.0001");
    assert_eq!(projected["$serde_json::private::Number"], "literal");
    assert_eq!(projected["$serde_json::private::RawValue"], "null");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "header_tests.rs"]
mod header_tests;
