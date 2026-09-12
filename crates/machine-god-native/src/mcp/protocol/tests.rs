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
fn only_modern_discovery_is_selected_for_each_transport() {
    for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
        for versions in [
            json!(["2026-07-28"]),
            json!(["2025-11-25", "2026-07-28", "future"]),
        ] {
            let (mut machine, action) = Negotiation::new(transport);
            assert_eq!(action, NegotiationAction::SendDiscover);
            assert_eq!(
                reply(&mut machine, &discover(versions)),
                selected(transport, ProtocolVersion::Modern)
            );
            assert_eq!(
                reply(&mut machine, &discover(json!(["2026-07-28"]))),
                NegotiationAction::Failed(NegotiationFailure::WrongState)
            );
        }
        for version in [
            "2025-11-25",
            "2025-06-18",
            "2025-03-26",
            "2024-11-05",
            "future",
        ] {
            assert_eq!(ProtocolVersion::parse_for(transport, version), None);
            let (mut machine, _) = Negotiation::new(transport);
            assert_eq!(
                reply(&mut machine, &discover(json!([version]))),
                NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
            );
        }
        assert_eq!(
            ProtocolVersion::parse_for(transport, "2026-07-28"),
            Some(ProtocolVersion::Modern)
        );
        let (mut machine, _) = Negotiation::new(transport);
        assert_eq!(
            reply(&mut machine, &discover(json!([]))),
            NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
        );
    }
}

#[test]
fn malformed_discovery_success_is_terminal() {
    for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
        for result in [
            json!(null),
            json!({}),
            json!({"protocolVersion":"2024-11-05", "capabilities":{}}),
            json!({"resultType":"input_required","supportedVersions":["2026-07-28"],"capabilities":{}}),
            json!({"resultType":"complete","supportedVersions":["2026-07-28"]}),
            json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":null}),
            json!({"resultType":"complete","supportedVersions":["2026-07-28",42],"capabilities":{}}),
            json!({"resultType":"complete","supportedVersions":"2026-07-28","capabilities":{}}),
        ] {
            let (mut machine, _) = Negotiation::new(transport);
            assert_eq!(
                reply(&mut machine, &success(result)),
                NegotiationAction::Failed(NegotiationFailure::InvalidDiscovery)
            );
        }
    }
}

#[test]
fn errors_and_old_version_hints_never_initialize_or_restart() {
    for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
        for code in [-32601, -32602, -32021, -32022] {
            let (mut machine, _) = Negotiation::new(transport);
            assert_eq!(
                reply(
                    &mut machine,
                    &error(
                        code,
                        json!({"requested":"2026-07-28","supported":["2025-11-25","2024-11-05"]})
                    )
                ),
                NegotiationAction::Failed(NegotiationFailure::ProtocolError(code))
            );
        }
        for failure in [
            NegotiationFailure::Deadline,
            NegotiationFailure::Transport,
            NegotiationFailure::Cancelled,
        ] {
            let (mut machine, _) = Negotiation::new(transport);
            assert_eq!(machine.abort(failure), NegotiationAction::Failed(failure));
            assert_eq!(
                reply(&mut machine, &discover(json!(["2026-07-28"]))),
                NegotiationAction::Failed(NegotiationFailure::WrongState)
            );
        }
    }
}

#[test]
fn http_null_error_and_single_same_modern_retry_preserve_exact_evidence() {
    let signal = envelope(&json!({"jsonrpc":"2.0","id":null,"error":{
    "code":-32022,"message":"unsupported","data":{
        "requested":"2026-07-28","supported":["2026-07-28"]
    }}}));
    for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
        let (mut machine, _) = Negotiation::new(transport);
        let action = machine.response(
            &signal,
            &RpcId::Integer(7),
            HttpDiscoveryStatus::VersionError,
        );
        if transport == TransportKind::Stdio {
            assert!(matches!(
                action,
                NegotiationAction::Failed(NegotiationFailure::Wire(_))
            ));
        } else {
            assert_eq!(action, NegotiationAction::SendDiscover);
            assert_eq!(
                machine.response(
                    &signal,
                    &RpcId::Integer(8),
                    HttpDiscoveryStatus::VersionError
                ),
                NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
            );
        }
    }
    let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
    assert_eq!(
        machine.response(
            &signal,
            &RpcId::Integer(7),
            HttpDiscoveryStatus::VersionError
        ),
        NegotiationAction::SendDiscover
    );
    assert_eq!(
        reply(&mut machine, &discover(json!(["2026-07-28"]))),
        selected(TransportKind::StreamableHttp, ProtocolVersion::Modern)
    );
    let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
    assert_eq!(
        machine.response(&signal, &RpcId::Integer(7), HttpDiscoveryStatus::Ordinary),
        NegotiationAction::Failed(NegotiationFailure::UnsupportedVersion)
    );
}

#[test]
fn http_retry_requires_exact_requested_and_all_string_supported_versions() {
    for data in [
        json!(null),
        json!({}),
        json!({"requested":"2025-11-25","supported":["2026-07-28"]}),
        json!({"requested":"2026-07-28","supported":["2025-11-25"]}),
        json!({"requested":"2026-07-28","supported":["2026-07-28",null]}),
        json!({"requested":"2026-07-28","supported":"2026-07-28"}),
    ] {
        let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
        assert_eq!(
            machine.response(
                &error(-32022, data),
                &RpcId::Integer(1),
                HttpDiscoveryStatus::VersionError
            ),
            NegotiationAction::Failed(NegotiationFailure::ProtocolError(-32022))
        );
    }
    let (mut machine, _) = Negotiation::new(TransportKind::StreamableHttp);
    assert_eq!(
        machine.response(
            &discover(json!(["2026-07-28"])),
            &RpcId::Integer(1),
            HttpDiscoveryStatus::VersionError
        ),
        NegotiationAction::Failed(NegotiationFailure::InvalidDiscovery)
    );
    let (mut machine, _) = Negotiation::new(TransportKind::Stdio);
    assert_eq!(
        machine.response(
            &discover(json!(["2026-07-28"])),
            &RpcId::Integer(1),
            HttpDiscoveryStatus::VersionError
        ),
        NegotiationAction::Failed(NegotiationFailure::WrongState)
    );
}

#[test]
fn modern_http_header_is_transport_specific() {
    for transport in [TransportKind::Stdio, TransportKind::StreamableHttp] {
        let protocol = NegotiatedProtocol {
            transport,
            version: ProtocolVersion::Modern,
        };
        assert_eq!(
            protocol.sends_http_protocol_header(),
            transport == TransportKind::StreamableHttp
        );
    }
}

#[test]
fn modern_metadata_is_present_without_progress_and_preserves_selected_modes() {
    for (progress, form, url) in [
        (None, false, false),
        (Some(u64::MAX), true, false),
        (Some(0), false, true),
        (None, true, true),
    ] {
        let metadata =
            McpClientMetadata::for_protocol(ProtocolVersion::Modern, progress, form, url);
        let value = serde_json::to_value(metadata).unwrap();
        assert_eq!(
            value["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(
            value["io.modelcontextprotocol/clientInfo"]["name"],
            "machine-god"
        );
        assert_eq!(
            value.get("progressToken"),
            progress.as_ref().map(|_| &value["progressToken"])
        );
        if let Some(progress) = progress {
            assert_eq!(value["progressToken"], progress);
        }
        let capabilities = &value["io.modelcontextprotocol/clientCapabilities"];
        assert_eq!(capabilities.get("elicitation").is_some(), form || url);
        if form || url {
            assert_eq!(capabilities["elicitation"].get("form").is_some(), form);
            assert_eq!(capabilities["elicitation"].get("url").is_some(), url);
        }
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
