use serde_json::{Value, json};

use super::*;

fn envelope(value: &Value) -> RpcEnvelope {
    parse_envelope(&serde_json::to_vec(value).unwrap(), WireLimits::default()).unwrap()
}

fn success(result: Value) -> RpcEnvelope {
    let mut value = json!({"jsonrpc":"2.0", "id":1});
    value["result"] = result;
    envelope(&value)
}
fn error(code: i64, data: Value) -> RpcEnvelope {
    let mut value =
        json!({"jsonrpc":"2.0", "id":1, "error":{"code":code,"message":"untrusted secret"}});
    value["error"]["data"] = data;
    envelope(&value)
}
fn discover(versions: Value) -> RpcEnvelope {
    let mut value = json!({"resultType":"complete", "capabilities":{}});
    value["supportedVersions"] = versions;
    success(value)
}
fn reply(machine: &mut Negotiation, response: &RpcEnvelope) -> NegotiationAction {
    machine.response(response, &RpcId::Integer(1), HttpDiscoveryStatus::Ordinary)
}
fn selected(transport: TransportKind, version: ProtocolVersion) -> NegotiationAction {
    NegotiationAction::Ready(NegotiatedProtocol { transport, version })
}

#[test]
fn framing_every_chunk_boundary_and_multiple_frames() {
    let input = b"\n\r\n{\"id\":1}\r\r\n{\"id\":2}\n";
    for split in 0..=input.len() {
        let mut decoder = NdjsonDecoder::new(WireLimits::default()).unwrap();
        let mut frames = Vec::new();
        for mut chunk in [&input[..split], &input[split..]] {
            while !chunk.is_empty() {
                let progress = decoder.push(chunk).unwrap();
                assert!(progress.consumed > 0);
                chunk = &chunk[progress.consumed..];
                if let Some(frame) = progress.frame {
                    frames.push(frame);
                }
            }
        }
        assert_eq!(frames, [b"{\"id\":1}".to_vec(), b"{\"id\":2}".to_vec()]);
        decoder.finish().unwrap();
        assert_eq!(decoder.push(b"x"), Err(WireError::ClosedDecoder));
    }
}

#[test]
fn framing_exact_bound_oversize_partial_and_poison() {
    let limits = WireLimits {
        max_frame_bytes: 4,
        ..WireLimits::default()
    };
    let mut decoder = NdjsonDecoder::new(limits).unwrap();
    assert_eq!(
        decoder.push(b"1234\nmany more bytes").unwrap().frame,
        Some(b"1234".to_vec())
    );
    decoder.push(b"1234").unwrap();
    assert_eq!(decoder.push(b"5\n"), Err(WireError::FrameTooLarge));
    assert_eq!(decoder.finish(), Err(WireError::ClosedDecoder));
    assert_eq!(decoder.push(b"\n"), Err(WireError::ClosedDecoder));
    let mut decoder = NdjsonDecoder::new(limits).unwrap();
    decoder.push(b"{}").unwrap();
    assert_eq!(decoder.finish(), Err(WireError::IncompleteFrame));
    let mut decoder = NdjsonDecoder::new(limits).unwrap();
    assert_eq!(
        decoder.push(b""),
        Ok(FrameProgress {
            consumed: 0,
            frame: None
        })
    );
    decoder.finish().unwrap();
}

#[test]
fn one_byte_chunks_are_amortized_and_utf8_is_not_decoded_prematurely() {
    let limits = WireLimits {
        max_frame_bytes: 4096,
        ..WireLimits::default()
    };
    let mut decoder = NdjsonDecoder::new(limits).unwrap();
    allocation_counter::measure(|| {});
    let allocations = allocation_counter::measure(|| {
        for _ in 0..4096 {
            decoder.push(b"x").unwrap();
        }
    });
    assert!(allocations.count_total <= 14, "{allocations:?}");
    assert_eq!(decoder.push(b"\n").unwrap().frame.unwrap().len(), 4096);
    let text = "{\"jsonrpc\":\"2.0\",\"id\":\"🦀\",\"result\":{}}\n";
    for split in 0..text.len() {
        let mut decoder = NdjsonDecoder::new(limits).unwrap();
        assert!(
            decoder
                .push(&text.as_bytes()[..split])
                .unwrap()
                .frame
                .is_none()
        );
        let frame = decoder
            .push(&text.as_bytes()[split..])
            .unwrap()
            .frame
            .unwrap();
        assert_eq!(
            parse_envelope(&frame, limits).unwrap().id(),
            Some(&RpcId::String("🦀".into()))
        );
    }
}

#[test]
fn limits_have_fixed_hard_maxima() {
    for limits in [
        WireLimits {
            max_frame_bytes: 0,
            ..WireLimits::default()
        },
        WireLimits {
            max_frame_bytes: usize::MAX,
            ..WireLimits::default()
        },
        WireLimits {
            max_depth: 0,
            ..WireLimits::default()
        },
        WireLimits {
            max_depth: 65,
            ..WireLimits::default()
        },
        WireLimits {
            max_nodes: 0,
            ..WireLimits::default()
        },
        WireLimits {
            max_nodes: 262_145,
            ..WireLimits::default()
        },
    ] {
        assert_eq!(
            NdjsonDecoder::new(limits).unwrap_err(),
            WireError::InvalidLimits
        );
        assert_eq!(
            parse_envelope(b"{}", limits).unwrap_err(),
            WireError::InvalidLimits
        );
    }
}

#[test]
fn envelopes_are_disjoint_and_ids_exact() {
    let cases = [
        (
            json!({"jsonrpc":"2.0","method":"tools/list","id":1,"params":{}}),
            RpcKind::Request,
        ),
        (
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            RpcKind::Notification,
        ),
        (
            json!({"jsonrpc":"2.0","id":"1","result":null}),
            RpcKind::Success,
        ),
        (
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32601,"message":"missing"}}),
            RpcKind::Error,
        ),
    ];
    for (value, kind) in cases {
        assert_eq!(envelope(&value).kind(), kind);
    }
    let response = success(json!({}));
    response.correlate(&RpcId::Integer(1), false).unwrap();
    for stale in [RpcId::Integer(2), RpcId::String("1".into()), RpcId::Null] {
        assert_eq!(
            response.correlate(&stale, true),
            Err(WireError::MismatchedId)
        );
    }
    let null_error = envelope(&json!({"jsonrpc":"2.0","id":null,"error":{"code":-1,"message":""}}));
    assert_eq!(
        null_error.correlate(&RpcId::Integer(1), false),
        Err(WireError::MismatchedId)
    );
    null_error.correlate(&RpcId::Integer(1), true).unwrap();
}

#[test]
fn invalid_envelope_matrix() {
    for value in [
        json!([]),
        json!({}),
        json!({"jsonrpc":"1.0","id":1,"result":{}}),
        json!({"jsonrpc":"2.0","id":1}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"error":{"code":-1,"message":"ambiguous"}}),
        json!({"jsonrpc":"2.0","id":null,"result":{}}),
        json!({"jsonrpc":"2.0","id":1.0,"result":{}}),
        json!({"jsonrpc":"2.0","id":u64::MAX,"result":{}}),
        json!({"jsonrpc":"2.0","id":true,"result":{}}),
        json!({"jsonrpc":"2.0","result":{}}),
        json!({"jsonrpc":"2.0","method":"x","id":null}),
        json!({"jsonrpc":"2.0","method":"x","params":null}),
        json!({"jsonrpc":"2.0","method":"x","result":{}}),
        json!({"jsonrpc":"2.0","method":""}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":1.1,"message":""}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":1,"message":null}}),
        json!({"jsonrpc":"2.0","id":1,"error":null}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"params":{}}),
        json!({"jsonrpc":"2.0","method":"x".repeat(257)}),
        json!({"jsonrpc":"2.0","id":"x".repeat(1025),"result":{}}),
    ] {
        assert_eq!(
            parse_envelope(&serde_json::to_vec(&value).unwrap(), WireLimits::default())
                .unwrap_err(),
            WireError::InvalidEnvelope,
            "{value}"
        );
    }
}

#[test]
fn duplicate_keys_at_every_depth_escaped_aliases_and_trailing_bytes_reject() {
    for text in [
        r#"{"jsonrpc":"2.0","id":1,"id":1,"result":{}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"a":1,"\u0061":2}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":[{"a":1,"a":2}]}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{}} {}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":"secret"} trailing-secret"#,
        r#"{"jsonrpc":"2.0","id":1,"result":1e+}"#,
    ] {
        assert_eq!(
            parse_envelope(text.as_bytes(), WireLimits::default()).unwrap_err(),
            WireError::InvalidJson
        );
    }
    assert_eq!(
        parse_envelope(b"\xff", WireLimits::default()).unwrap_err(),
        WireError::InvalidJson
    );
}

#[test]
fn depth_nodes_and_bytes_admit_exact_boundary_only() {
    let bytes = br#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
    let limits = WireLimits {
        max_frame_bytes: bytes.len(),
        max_depth: 2,
        max_nodes: 7,
    };
    parse_envelope(bytes, limits).unwrap();
    for limits in [
        WireLimits {
            max_frame_bytes: bytes.len() - 1,
            ..limits
        },
        WireLimits {
            max_depth: 1,
            ..limits
        },
        WireLimits {
            max_nodes: 6,
            ..limits
        },
    ] {
        assert!(parse_envelope(bytes, limits).is_err());
    }
    let deep = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}0{}}}",
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    assert_eq!(
        parse_envelope(deep.as_bytes(), WireLimits::default()).unwrap_err(),
        WireError::InvalidJson
    );
}

#[test]
fn debug_and_errors_never_echo_wire_data() {
    let response = error(-32601, json!({"token":"untrusted secret"}));
    assert!(!format!("{response:?} {:?}", response.protocol_error().unwrap()).contains("secret"));
    assert!(!format!("{:?}", RpcId::String("untrusted secret".into())).contains("secret"));
    let mut decoder = NdjsonDecoder::new(WireLimits::default()).unwrap();
    decoder.push(b"untrusted secret").unwrap();
    assert!(!format!("{decoder:?}").contains("secret"));
    assert!(!format!("{:?}", decoder.push(b"\n").unwrap()).contains("secret"));
}

#[test]
fn producer_derived_modern_and_legacy_discovery_selection() {
    // fx tests/e2e/mcp-stdio.test.ts:1198, 1231, 1297; protocol_negotiation.zig.
    for code in [-32601, -32602, -32603, 1] {
        let (mut machine, first) = Negotiation::new(TransportKind::Stdio);
        assert_eq!(first, NegotiationAction::SendDiscover);
        assert_eq!(
            reply(&mut machine, &error(code, Value::Null)),
            NegotiationAction::RestartInitialize(ProtocolVersion::Legacy20251125)
        );
    }
    let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
    assert_eq!(
        reply(
            &mut machine,
            &discover(json!(["2024-11-05", "2025-06-18", "2025-11-25"]))
        ),
        NegotiationAction::RestartInitialize(ProtocolVersion::Legacy20251125)
    );
    for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
        let (mut machine, _) = Negotiation::new(transport);
        assert_eq!(
            reply(&mut machine, &discover(json!(["2025-11-25", "2026-07-28"]))),
            selected(transport, ProtocolVersion::Modern)
        );
        assert_eq!(
            reply(&mut machine, &discover(json!(["2026-07-28"]))),
            NegotiationAction::Failed(NegotiationFailure::WrongState)
        );
    }
}

#[test]
fn malformed_discovery_success_never_downgrades() {
    for result in [
        Value::Null,
        json!({}),
        json!({"resultType":"input_required","supportedVersions":["2025-11-25"],"capabilities":{}}),
        json!({"resultType":"complete","supportedVersions":["2026-07-28",null],"capabilities":{}}),
        json!({"resultType":"complete","supportedVersions":[],"capabilities":[]}),
    ] {
        for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
            let (mut machine, _) = Negotiation::new(transport);
            assert_eq!(
                reply(&mut machine, &success(result.clone())),
                NegotiationAction::Failed(NegotiationFailure::InvalidDiscovery)
            );
            assert_eq!(
                machine.stdio_discovery_unavailable(),
                NegotiationAction::Failed(NegotiationFailure::WrongState)
            );
        }
    }
}

#[test]
fn stdio_protocol_error_evidence_requires_exact_requested_and_all_string_versions() {
    let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
    assert_eq!(
        reply(
            &mut machine,
            &error(
                -32021,
                json!({"supported":["2025-11-25"],"requested":"2026-07-28"})
            )
        ),
        NegotiationAction::Failed(NegotiationFailure::ProtocolError(-32021))
    );
    for data in [
        Value::Null,
        json!({"supported":["2025-11-25"],"requested":"wrong"}),
        json!({"supported":["2025-11-25", 1],"requested":"2026-07-28"}),
        json!({"supported":["2026-07-28"],"requested":"2026-07-28"}),
    ] {
        let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
        assert_eq!(
            reply(&mut machine, &error(-32022, data)),
            NegotiationAction::Failed(NegotiationFailure::ProtocolError(-32022))
        );
    }
    let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
    assert_eq!(
        reply(
            &mut machine,
            &error(
                -32022,
                json!({"requested":"2026-07-28","supported":["2024-11-05","2025-06-18"]})
            )
        ),
        NegotiationAction::RestartInitialize(ProtocolVersion::Legacy20250618)
    );
}

#[test]
fn exhaustive_stdio_initialize_transition_matrix() {
    let versions = [
        ProtocolVersion::Legacy20251125,
        ProtocolVersion::Legacy20250618,
        ProtocolVersion::Legacy20241105,
    ];
    for (offered_index, offered) in versions.iter().copied().enumerate() {
        for (hint_index, hint) in versions.iter().copied().enumerate() {
            let mut machine = stdio_offer(offered);
            let response = error(
                -32022,
                json!({"requested":offered.as_str(), "supported":[hint.as_str()]}),
            );
            let expected = if hint_index > offered_index {
                NegotiationAction::RestartInitialize(hint)
            } else {
                NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
            };
            assert_eq!(reply(&mut machine, &response), expected);
            // Accepted versions may be newer than the offered version at the pin.
            let mut machine = stdio_offer(offered);
            assert_eq!(
                reply(
                    &mut machine,
                    &success(json!({"protocolVersion":hint.as_str()}))
                ),
                selected(TransportKind::Stdio, hint)
            );
        }
        for closed in [true, false] {
            let mut machine = stdio_offer(offered);
            let action = if closed {
                machine.stdio_initialize_closed()
            } else {
                reply(&mut machine, &error(-32022, Value::Null))
            };
            let expected = versions.get(offered_index + 1).map_or(
                NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion),
                |v| NegotiationAction::RestartInitialize(*v),
            );
            assert_eq!(action, expected);
        }
    }
}

fn stdio_offer(offered: ProtocolVersion) -> Negotiation {
    let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
    assert_eq!(
        reply(&mut machine, &discover(json!([offered.as_str()]))),
        NegotiationAction::RestartInitialize(offered)
    );
    machine
}

#[test]
fn legacy_initialize_invalid_params_requires_a_downward_hint() {
    for (data, expected) in [
        (
            Value::Null,
            NegotiationAction::Failed(NegotiationFailure::ProtocolError(-32602)),
        ),
        (
            json!({"requested":"2025-11-25","supported":["2024-11-05"]}),
            NegotiationAction::RestartInitialize(ProtocolVersion::Legacy20241105),
        ),
        (
            json!({"requested":"2026-07-28","supported":["2024-11-05"]}),
            NegotiationAction::Failed(NegotiationFailure::ProtocolError(-32602)),
        ),
    ] {
        assert_eq!(
            reply(
                &mut stdio_offer(ProtocolVersion::Legacy20251125),
                &error(-32602, data)
            ),
            expected
        );
    }
}

#[test]
fn clean_discovery_unavailability_is_distinct_from_partial_frame_and_cancellation() {
    let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
    assert_eq!(
        machine.stdio_discovery_unavailable(),
        NegotiationAction::RestartInitialize(ProtocolVersion::Legacy20241105)
    );
    assert_eq!(
        machine.stdio_initialize_closed(),
        NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
    );
    for reason in [
        NegotiationFailure::Cancelled,
        NegotiationFailure::Deadline,
        NegotiationFailure::Transport,
        NegotiationFailure::Wire(WireError::IncompleteFrame),
    ] {
        let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
        assert_eq!(machine.abort(reason), NegotiationAction::Failed(reason));
        assert_eq!(
            machine.stdio_discovery_unavailable(),
            NegotiationAction::Failed(NegotiationFailure::WrongState)
        );
    }
}

#[test]
fn http_null_error_discovery_only_and_single_explicit_modern_retry() {
    let evidence = json!({"requested":"2026-07-28", "supported":["2026-07-28"]});
    let signal = error(-32022, evidence);
    let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
    assert_eq!(
        machine.response(
            &signal,
            &RpcId::Integer(1),
            HttpDiscoveryStatus::VersionError
        ),
        NegotiationAction::SendDiscover
    );
    assert_eq!(
        machine.response(
            &signal,
            &RpcId::Integer(1),
            HttpDiscoveryStatus::VersionError
        ),
        NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
    );
    let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
    assert_eq!(
        reply(&mut machine, &signal),
        NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
    );
    let null_error =
        envelope(&json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"invalid"}}));
    let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
    assert_eq!(
        reply(&mut machine, &null_error),
        NegotiationAction::Initialize(ProtocolVersion::Legacy20251125)
    );
    assert_eq!(
        reply(&mut machine, &null_error),
        NegotiationAction::Failed(NegotiationFailure::Wire(WireError::MismatchedId))
    );
    let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
    assert_eq!(
        machine.response(
            &signal,
            &RpcId::Integer(1),
            HttpDiscoveryStatus::VersionError
        ),
        NegotiationAction::SendDiscover
    );
    assert_eq!(
        reply(&mut machine, &null_error),
        NegotiationAction::Initialize(ProtocolVersion::Legacy20251125)
    );
    let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
    assert_eq!(
        reply(&mut machine, &null_error),
        NegotiationAction::Failed(NegotiationFailure::Wire(WireError::MismatchedId))
    );
}

#[test]
fn http_status_and_transport_specific_version_matrix() {
    for status in [404, 405] {
        let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
        assert_eq!(
            machine.http_discovery_mismatch(status),
            NegotiationAction::Initialize(ProtocolVersion::Legacy20251125)
        );
    }
    for status in [200, 301, 400, 401, 403, 429, 500] {
        let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
        assert_eq!(
            machine.http_discovery_mismatch(status),
            NegotiationAction::Failed(NegotiationFailure::WrongState)
        );
    }
    for transport in [
        TransportKind::Stdio,
        TransportKind::StreamableHttp,
        TransportKind::LegacySse,
    ] {
        for version in [
            ProtocolVersion::Legacy20251125,
            ProtocolVersion::Legacy20250618,
            ProtocolVersion::Legacy20250326,
            ProtocolVersion::Legacy20241105,
        ] {
            let (mut machine, first) = Negotiation::new(transport);
            if transport == TransportKind::LegacySse {
                assert_eq!(
                    first,
                    NegotiationAction::Initialize(ProtocolVersion::Legacy20241105)
                );
            } else {
                reply(&mut machine, &error(-32601, Value::Null));
            }
            let action = reply(
                &mut machine,
                &success(json!({"protocolVersion":version.as_str()})),
            );
            let supported = ProtocolVersion::parse_for(transport, version.as_str()).is_some();
            assert_eq!(
                action,
                if supported {
                    selected(transport, version)
                } else {
                    NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
                }
            );
        }
    }
}

#[test]
fn http_legacy_policies_do_not_leak_to_other_versions() {
    for version in [
        ProtocolVersion::Modern,
        ProtocolVersion::Legacy20251125,
        ProtocolVersion::Legacy20250618,
        ProtocolVersion::Legacy20250326,
    ] {
        let protocol = NegotiatedProtocol {
            transport: TransportKind::StreamableHttp,
            version,
        };
        assert_eq!(
            protocol.needs_initialized_notification(),
            version != ProtocolVersion::Modern
        );
        assert_eq!(
            protocol.sends_http_protocol_header(),
            version != ProtocolVersion::Legacy20250326
        );
        assert_eq!(
            protocol.allows_legacy_http_poll_close(),
            version == ProtocolVersion::Legacy20251125
        );
    }
}

#[test]
fn http_discovery_evidence_and_retry_success_match_pinned_transport_rules() {
    let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
    let signal = error(
        -32022,
        json!({"requested":"2026-07-28","supported":["2026-07-28"]}),
    );
    assert_eq!(
        machine.response(
            &signal,
            &RpcId::Integer(1),
            HttpDiscoveryStatus::VersionError
        ),
        NegotiationAction::SendDiscover
    );
    assert_eq!(
        reply(&mut machine, &discover(json!(["2026-07-28"]))),
        selected(TransportKind::StreamableHttp, ProtocolVersion::Modern)
    );
    // HTTP ordinary errors include missing-capability errors, unlike stdio.
    for code in [-32021, -32601, -32000] {
        let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
        assert_eq!(
            reply(&mut machine, &error(code, Value::Null)),
            NegotiationAction::Initialize(ProtocolVersion::Legacy20251125)
        );
    }
    // Discovery uses the pin's shared legacy classifier; actual initialize
    // admission has a distinct HTTP set which includes 2025-03-26.
    for (versions, expected) in [
        (
            json!(["2024-11-05"]),
            NegotiationAction::Initialize(ProtocolVersion::Legacy20251125),
        ),
        (
            json!(["2025-03-26"]),
            NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion),
        ),
        (
            json!([]),
            NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion),
        ),
    ] {
        let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
        assert_eq!(reply(&mut machine, &discover(versions)), expected);
    }
}

#[test]
fn stale_id_and_invalid_wire_abort_without_downgrade() {
    for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
        let (mut machine, _) = Negotiation::new(transport);
        assert_eq!(
            machine.response(
                &error(-32601, Value::Null),
                &RpcId::Integer(2),
                HttpDiscoveryStatus::Ordinary
            ),
            NegotiationAction::Failed(NegotiationFailure::Wire(WireError::MismatchedId))
        );
    }
}
