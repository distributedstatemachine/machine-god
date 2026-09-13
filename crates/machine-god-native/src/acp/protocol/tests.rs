use super::*;

fn json(text: &str) -> Value {
    machine_god_core::json::from_str(text).unwrap()
}

fn request(params: Value) -> AcpMessage {
    AcpMessage::Request {
        id: AcpId::Integer(1),
        method: "session/prompt".into(),
        params: Some(params),
    }
}

#[test]
fn exact_numbers_private_keys_and_unicode_survive_wire_round_trip() {
    let source = br#"{"jsonrpc":"2.0","id":"request\u263a","method":"session/prompt","params":{"huge":123456789012345678901234567890,"negativeZero":-0,"exponent":1.2300E+999,"$serde_json::private::Number":"literal","$serde_json::private::RawValue":{"secret":true}}}"#;
    let decoded = decode_frame(source).unwrap();
    let encoded = encode_frame(&decoded).unwrap();
    let text = std::str::from_utf8(&encoded).unwrap();
    assert!(text.contains("123456789012345678901234567890"));
    assert!(text.contains("\"negativeZero\":-0"));
    assert!(text.contains("1.2300E+999"));
    assert_eq!(decode_frame(&encoded).unwrap(), decoded);
}

#[test]
fn duplicate_keys_are_rejected_at_every_level() {
    for source in [
        r#"{"jsonrpc":"2.0","id":1,"id":2,"method":"x"}"#,
        r#"{"jsonrpc":"2.0","method":"x","params":{"a":1,"\u0061":2}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"a":{"x":1,"x":2}}}"#,
        r#"{"jsonrpc":"2.0","method":"x","extension":{"x":1,"x":2}}"#,
    ] {
        assert_eq!(
            decode_frame(source.as_bytes()),
            Err(AcpProtocolError::ParseError)
        );
    }
}

#[test]
fn envelopes_and_identifiers_are_strict() {
    for source in [
        "[]",
        "null",
        "{}",
        r#"{"method":"x"}"#,
        r#"{"jsonrpc":"1.0","method":"x"}"#,
        r#"{"jsonrpc":"2.0","method":""}"#,
        r#"{"jsonrpc":"2.0","method":"a\nb"}"#,
        r#"{"jsonrpc":"2.0","method":3}"#,
        r#"{"jsonrpc":"2.0","method":"x","params":null}"#,
        r#"{"jsonrpc":"2.0","method":"x","params":"text"}"#,
        r#"{"jsonrpc":"2.0","method":"x","id":null}"#,
        r#"{"jsonrpc":"2.0","method":"x","id":1.0}"#,
        r#"{"jsonrpc":"2.0","method":"x","id":1e0}"#,
        r#"{"jsonrpc":"2.0","method":"x","id":9223372036854775808}"#,
        r#"{"jsonrpc":"2.0","method":"x","id":false}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":true,"error":{}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":true,"method":"x"}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":true,"params":{}}"#,
        r#"{"jsonrpc":"2.0","result":true}"#,
        r#"{"jsonrpc":"2.0","id":null,"result":true}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":1.0,"message":"x"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":3}}"#,
    ] {
        assert_eq!(
            decode_frame(source.as_bytes()),
            Err(AcpProtocolError::InvalidRequest),
            "{source}"
        );
    }
    assert!(decode_frame(b"\xff").is_err());
}

#[test]
fn requests_notifications_and_error_null_ids_have_distinct_shapes() {
    let notification =
        decode_frame(br#"{"jsonrpc":"2.0","method":"session/cancel","params":{}}"#).unwrap();
    assert!(matches!(notification, AcpMessage::Notification { .. }));
    for id in [
        "-9223372036854775808",
        "9223372036854775807",
        "0",
        "-0",
        "\"\"",
    ] {
        let source = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"x"}}"#);
        assert!(matches!(
            decode_frame(source.as_bytes()).unwrap(),
            AcpMessage::Request { .. }
        ));
    }
    let error = AcpMessage::Response {
        id: None,
        outcome: Err(AcpRpcError {
            code: -32700,
            message: "parse error".into(),
            data: Some(json(r#"{"number":-0}"#)),
        }),
    };
    assert_eq!(decode_frame(&encode_frame(&error).unwrap()).unwrap(), error);
    let success = AcpMessage::Response {
        id: Some(AcpId::Integer(2)),
        outcome: Ok(Value::Null),
    };
    assert_eq!(
        decode_frame(&encode_frame(&success).unwrap()).unwrap(),
        success
    );
}

#[test]
fn only_protocol_one_is_supported() {
    assert_eq!(
        validate_protocol_version(&json(r#"{"protocolVersion":1}"#)),
        Ok(())
    );
    for source in [
        "{}",
        "null",
        r#"{"protocolVersion":0}"#,
        r#"{"protocolVersion":2}"#,
        r#"{"protocolVersion":1.0}"#,
        r#"{"protocolVersion":"1"}"#,
    ] {
        assert_eq!(
            validate_protocol_version(&json(source)),
            Err(AcpProtocolError::UnsupportedVersion)
        );
    }
}

#[test]
fn arbitrary_fragmentation_and_multiple_frames_preserve_backpressure() {
    let message = request(json(r#"{"text":"héllo\nworld"}"#));
    let frame = encode_frame(&message).unwrap();
    for split in 0..=frame.len() {
        let mut decoder = AcpFrameDecoder::new();
        let mut first = &frame[..split];
        let mut second = &frame[split..];
        let a = decoder.next(&mut first);
        let b = decoder.next(&mut second);
        assert!(first.is_empty() && second.is_empty());
        assert_eq!(a.or(b), Some(Ok(message.clone())));
        assert_eq!(decoder.finish(), None);
    }
    let stream = [frame.clone(), frame].concat();
    let mut input = stream.as_slice();
    let mut decoder = AcpFrameDecoder::new();
    assert_eq!(decoder.next(&mut input), Some(Ok(message.clone())));
    assert_eq!(input.len(), stream.len() / 2);
    assert_eq!(decoder.next(&mut input), Some(Ok(message)));
    assert!(input.is_empty());
}

#[test]
fn oversize_frames_drain_once_and_next_frame_recovers() {
    let good = encode_frame(&request(json("{}"))).unwrap();
    let mut decoder = AcpFrameDecoder::new();
    let prefix = vec![b' '; ACP_MAX_FRAME_BYTES];
    assert_eq!(decoder.next(&mut prefix.as_slice()), None);
    assert_eq!(
        decoder.next(&mut &b"x"[..]),
        Some(Err(AcpProtocolError::FrameTooLarge))
    );
    assert_eq!(decoder.next(&mut &b"ignored"[..]), None);
    let tail = [b"more\n".as_slice(), &good].concat();
    let mut input = tail.as_slice();
    assert_eq!(decoder.next(&mut input), Some(Ok(request(json("{}")))));
    assert!(input.is_empty());
    assert_eq!(decoder.finish(), None);

    let oversized = [vec![b' '; ACP_MAX_FRAME_BYTES + 1], b"\n".to_vec(), good].concat();
    let mut input = oversized.as_slice();
    assert_eq!(
        decoder.next(&mut input),
        Some(Err(AcpProtocolError::FrameTooLarge))
    );
    assert_eq!(decoder.next(&mut input), Some(Ok(request(json("{}")))));
}

#[test]
fn eof_rejects_even_valid_unterminated_json_and_releases_partial_bytes() {
    let mut decoder = AcpFrameDecoder::new();
    assert_eq!(
        decoder.next(&mut &br#"{"jsonrpc":"2.0","method":"x"}"#[..]),
        None
    );
    assert_eq!(decoder.finish(), Some(AcpProtocolError::TruncatedFrame));
    assert_eq!(decoder.finish(), None);
    assert_eq!(
        decoder.next(&mut &b"\n"[..]),
        Some(Err(AcpProtocolError::ParseError))
    );
    assert_eq!(
        decoder.next(&mut &b" \r\n"[..]),
        Some(Err(AcpProtocolError::ParseError))
    );
    assert!(
        decoder
            .next(&mut &b"{\"jsonrpc\":\"2.0\",\"method\":\"x\"}\r\n"[..])
            .unwrap()
            .is_ok()
    );
}

#[test]
fn lexical_preflight_bounds_nodes_depth_and_bytes_before_decoding() {
    let wide = format!("[{}null]", "null,".repeat(ACP_MAX_JSON_NODES));
    assert_eq!(
        decode_frame(wide.as_bytes()),
        Err(AcpProtocolError::JsonBudgetExceeded)
    );
    let deep = "[".repeat(ACP_MAX_JSON_DEPTH + 1);
    assert_eq!(
        decode_frame(deep.as_bytes()),
        Err(AcpProtocolError::JsonBudgetExceeded)
    );
    let large = vec![b' '; ACP_MAX_FRAME_BYTES + 1];
    assert_eq!(decode_frame(&large), Err(AcpProtocolError::FrameTooLarge));
    // Brackets and scalar-looking bytes inside strings do not charge nodes.
    assert!(
        decode_frame(br#"{"jsonrpc":"2.0","method":"x","params":{"text":"[[[\" null,true"}}"#)
            .is_ok()
    );
}

#[test]
fn exact_depth_bound_matches_encoding_and_decoding() {
    let mut value = Value::Null;
    for _ in 1..ACP_MAX_JSON_DEPTH {
        value = Value::Array(vec![value]);
    }
    let message = request(value);
    let encoded = encode_frame(&message).unwrap();
    assert_eq!(decode_frame(&encoded).unwrap(), message);
}

#[test]
fn malformed_frame_does_not_consume_its_successor() {
    let good = encode_frame(&request(json("{}"))).unwrap();
    let stream = [b"not-json\n".as_slice(), &good].concat();
    let mut input = stream.as_slice();
    let mut decoder = AcpFrameDecoder::new();
    assert_eq!(
        decoder.next(&mut input),
        Some(Err(AcpProtocolError::ParseError))
    );
    assert_eq!(decoder.next(&mut input), Some(Ok(request(json("{}")))));
    let oversized = vec![b' '; ACP_MAX_FRAME_BYTES + 1];
    assert_eq!(
        decoder.next(&mut oversized.as_slice()),
        Some(Err(AcpProtocolError::FrameTooLarge))
    );
    assert_eq!(decoder.finish(), None);
}

#[test]
fn rejected_lexical_budgets_allocate_no_json_tree() {
    let wide = format!("[{}null]", "null,".repeat(ACP_MAX_JSON_NODES));
    let deep = "[".repeat(ACP_MAX_JSON_DEPTH + 1);
    let oversized = vec![b' '; ACP_MAX_FRAME_BYTES + 1];
    allocation_counter::measure(|| {});
    for bytes in [wide.as_bytes(), deep.as_bytes(), &oversized] {
        let allocations = allocation_counter::measure(|| {
            assert!(decode_frame(bytes).is_err());
        });
        assert_eq!(allocations.count_total, 0, "{allocations:?}");
    }
}

#[test]
fn tiny_input_fragments_have_amortized_bounded_storage() {
    let mut decoder = AcpFrameDecoder::new();
    allocation_counter::measure(|| {});
    let allocations = allocation_counter::measure(|| {
        for _ in 0..4096 {
            assert_eq!(decoder.next(&mut &b"x"[..]), None);
        }
    });
    assert!(allocations.count_total <= 14, "{allocations:?}");
    assert_eq!(decoder.finish(), Some(AcpProtocolError::TruncatedFrame));
}

#[test]
fn encoder_checks_constructed_data_and_exact_output_bound() {
    let malformed = AcpMessage::Response {
        id: None,
        outcome: Ok(Value::Null),
    };
    assert_eq!(
        encode_frame(&malformed),
        Err(AcpProtocolError::InvalidRequest)
    );
    let id = AcpId::String("x".repeat(ACP_MAX_ID_BYTES + 1));
    let malformed = AcpMessage::Request {
        id,
        method: "x".into(),
        params: None,
    };
    assert_eq!(
        encode_frame(&malformed),
        Err(AcpProtocolError::InvalidRequest)
    );
    let mut deep = Value::Null;
    for _ in 0..ACP_MAX_JSON_DEPTH {
        deep = Value::Array(vec![deep]);
    }
    assert_eq!(
        encode_frame(&request(deep)),
        Err(AcpProtocolError::JsonBudgetExceeded)
    );
    let wide = Value::Array(vec![Value::Null; ACP_MAX_JSON_NODES + 1]);
    assert_eq!(
        encode_frame(&request(wide)),
        Err(AcpProtocolError::JsonBudgetExceeded)
    );
    let overhead = encode_frame(&request(json(r#"{"text":""}"#)))
        .unwrap()
        .len()
        - 1;
    let params = json(&format!(
        r#"{{"text":"{}"}}"#,
        "x".repeat(ACP_MAX_FRAME_BYTES - overhead)
    ));
    let exact = encode_frame(&request(params)).unwrap();
    assert_eq!(exact.len(), ACP_MAX_FRAME_BYTES + 1);
    assert!(decode_frame(&exact[..exact.len() - 1]).is_ok());
    let over = request(Value::Array(vec![Value::String(
        "\n".repeat(ACP_MAX_FRAME_BYTES / 2),
    )]));
    assert_eq!(encode_frame(&over), Err(AcpProtocolError::FrameTooLarge));
}

#[test]
fn unchecked_number_tokens_cannot_inject_wire_fields() {
    for token in [
        "0,\"injected\":true",
        "null",
        "0 true",
        "0\n",
        "0 ",
        "1e",
        "[0]",
        "\"text\"",
    ] {
        let params = Value::Array(vec![Value::Number(
            serde_json::Number::from_string_unchecked(token.into()),
        )]);
        assert_eq!(
            encode_frame(&request(params)),
            Err(AcpProtocolError::InvalidRequest)
        );
    }
}

#[test]
fn oversized_constructed_strings_reject_before_output_allocation() {
    let message = request(Value::Array(vec![Value::String(
        "x".repeat(ACP_MAX_FRAME_BYTES + 1),
    )]));
    allocation_counter::measure(|| {});
    let allocations = allocation_counter::measure(|| {
        assert_eq!(encode_frame(&message), Err(AcpProtocolError::FrameTooLarge));
    });
    assert!(allocations.bytes_total <= 1024, "{allocations:?}");
    let error = AcpMessage::Response {
        id: None,
        outcome: Err(AcpRpcError {
            code: -1,
            message: "x".repeat(ACP_MAX_FRAME_BYTES + 1),
            data: None,
        }),
    };
    let allocations = allocation_counter::measure(|| {
        assert_eq!(encode_frame(&error), Err(AcpProtocolError::FrameTooLarge));
    });
    assert_eq!(allocations.count_total, 0, "{allocations:?}");
}

fn scope(session: u64) -> AcpScope {
    AcpScope {
        session,
        turn: 2,
        operation: 3,
        round: 4,
    }
}

#[test]
fn pending_requests_enforce_capacity_exact_scope_and_single_settlement() {
    let mut pending = AcpPendingRequests::new();
    let ids: Vec<_> = (0..ACP_MAX_PENDING_REQUESTS)
        .map(|_| pending.reserve(scope(1)).unwrap())
        .collect();
    assert_eq!(pending.len(), ACP_MAX_PENDING_REQUESTS);
    assert_eq!(
        pending.reserve(scope(1)),
        Err(AcpProtocolError::PendingLimit)
    );
    for wrong in [
        scope(2),
        AcpScope {
            turn: 9,
            ..scope(1)
        },
        AcpScope {
            operation: 9,
            ..scope(1)
        },
        AcpScope {
            round: 9,
            ..scope(1)
        },
    ] {
        assert_eq!(
            pending.complete(&ids[0], wrong),
            Err(AcpProtocolError::StaleResponse)
        );
        assert_eq!(pending.scope(&ids[0]), Some(scope(1)));
    }
    assert_eq!(pending.complete(&ids[0], scope(1)), Ok(scope(1)));
    assert_eq!(
        pending.complete(&ids[0], scope(1)),
        Err(AcpProtocolError::UnknownResponse)
    );
    let replacement = pending.reserve(scope(1)).unwrap();
    assert!(!ids.contains(&replacement));
}

#[test]
fn invalidation_returns_owned_ids_and_never_reuses_them() {
    let mut pending = AcpPendingRequests::new();
    let a = pending.reserve(scope(1)).unwrap();
    let b = pending.reserve(scope(2)).unwrap();
    let c_scope = AcpScope {
        turn: 9,
        ..scope(1)
    };
    let c = pending.reserve(c_scope).unwrap();
    assert_eq!(pending.invalidate_turn(1, 2), vec![(a.clone(), scope(1))]);
    assert_eq!(
        pending.complete(&a, scope(1)),
        Err(AcpProtocolError::UnknownResponse)
    );
    assert_eq!(pending.invalidate_session(1), vec![(c, c_scope)]);
    assert_eq!(pending.scope(&b), Some(scope(2)));
    assert_eq!(pending.clear(), vec![(b, scope(2))]);
    assert!(pending.is_empty());
    assert_ne!(pending.reserve(scope(1)).unwrap(), a);
}

#[test]
fn diagnostics_do_not_expose_ids_methods_or_payloads() {
    let secret = "sentinel-secret";
    let message = AcpMessage::Request {
        id: AcpId::String(secret.into()),
        method: secret.into(),
        params: Some(json(&format!(r#"{{"secret":"{secret}"}}"#))),
    };
    let error = AcpRpcError {
        code: -1,
        message: secret.into(),
        data: Some(Value::String(secret.into())),
    };
    assert!(!format!("{message:?} {error:?} {:?}", AcpId::String(secret.into())).contains(secret));
    let mut decoder = AcpFrameDecoder::new();
    assert_eq!(decoder.next(&mut secret.as_bytes()), None);
    assert!(!format!("{decoder:?}").contains(secret));
}
