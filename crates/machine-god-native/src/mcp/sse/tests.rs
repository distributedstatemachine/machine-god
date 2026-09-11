use super::*;

fn decoder(mode: SseMode) -> SseDecoder {
    SseDecoder::new(mode, SseLimits::default()).unwrap()
}

fn feed(decoder: &mut SseDecoder, mut bytes: &[u8]) -> Result<Vec<SseEvent>, SseError> {
    let mut events = Vec::new();
    while !bytes.is_empty() {
        let progress = decoder.push(bytes)?;
        assert!(progress.consumed > 0 && progress.consumed <= bytes.len());
        bytes = &bytes[progress.consumed..];
        if let Some(event) = progress.event {
            events.push(event);
        }
    }
    Ok(events)
}

fn complete(mode: SseMode, text: &str) -> Vec<SseEvent> {
    let mut decoder = decoder(mode);
    let events = feed(&mut decoder, text.as_bytes()).unwrap();
    decoder.finish().unwrap();
    events
}

#[test]
fn pinned_legacy_fixture_all_two_chunk_partitions() {
    // legacy_sse.zig:203 and 246 at the exact fx pin.
    for delimiter in ["\n", "\r", "\r\n"] {
        let text = format!(
            "event: message{delimiter}data: one{delimiter}data: two{delimiter}id: event-1{delimiter}retry: 90000{delimiter}{delimiter}"
        );
        for split in 0..=text.len() {
            let mut decoder = decoder(SseMode::Legacy);
            let mut events = feed(&mut decoder, &text.as_bytes()[..split]).unwrap();
            events.extend(feed(&mut decoder, &text.as_bytes()[split..]).unwrap());
            decoder.finish().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].data(), "one\ntwo");
            assert_eq!(events[0].event(), Some("message"));
            assert_eq!(events[0].id(), Some("event-1"));
            assert_eq!(events[0].retry_ms(), Some(60_000));
        }
    }
}

#[test]
fn exhaustive_partitions_preserve_crlf_and_empty_blocks() {
    let bytes = b"data:x\r\n\r\n";
    for mode in [SseMode::Modern, SseMode::Legacy] {
        for mask in 0..(1 << (bytes.len() - 1)) {
            let mut decoder = decoder(mode);
            let mut start = 0;
            let mut events = Vec::new();
            for end in 1..=bytes.len() {
                if end == bytes.len() || mask & (1 << (end - 1)) != 0 {
                    events.extend(feed(&mut decoder, &bytes[start..end]).unwrap());
                    start = end;
                }
            }
            decoder.finish().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].data(), "x");
        }
    }
}

#[test]
fn modern_pinned_empty_data_quirk_does_not_leak_into_legacy() {
    let text = "data:\ndata:\ndata: x\ndata:\n\n";
    assert_eq!(complete(SseMode::Modern, text)[0].data(), "x\n");
    assert_eq!(complete(SseMode::Legacy, text)[0].data(), "\n\nx\n");
    assert!(complete(SseMode::Modern, "data:\n\n").is_empty());
    assert!(complete(SseMode::Modern, "event: endpoint\nid: 1\nretry: 5\n\n").is_empty());
}

#[test]
fn legacy_control_frames_are_explicit_not_phantom_json_messages() {
    // Producer priming frame: mcp-legacy-remote.ts:532.
    let events = complete(SseMode::Legacy, "id:\ndata:\n\nevent:\n\nretry: 5\n\n");
    assert_eq!(events.len(), 3);
    for event in &events {
        assert_eq!(event.class(), SseEventClass::Control);
    }
    assert_eq!(events[0].id(), Some(""));
    assert!(events[0].had_data_field());
    assert_eq!(events[1].event(), Some(""));
    assert!(!events[1].had_data_field());
    assert_eq!(events[2].retry_ms(), Some(5));
    for mode in [SseMode::Modern, SseMode::Legacy] {
        assert!(complete(mode, "\n\r\n: heartbeat\nunknown: value\n\n").is_empty());
    }
}

#[test]
fn legacy_endpoint_fixture_is_data_not_an_admitted_url() {
    // Producer endpoint discovery: mcp-legacy-remote.ts:737.
    let events = complete(
        SseMode::Legacy,
        "event: endpoint\ndata: /messages\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n",
    );
    assert_eq!(events[0].event(), Some("endpoint"));
    assert_eq!(events[0].data(), "/messages");
    assert_eq!(events[0].class(), SseEventClass::Data);
    let response = crate::mcp::protocol::parse_envelope(
        events[1].data().as_bytes(),
        crate::mcp::protocol::WireLimits::default(),
    )
    .unwrap();
    response
        .correlate(&crate::mcp::protocol::RpcId::Integer(1), false)
        .unwrap();
}

#[test]
fn ids_never_inherit_and_invalid_ids_do_not_overwrite_observation() {
    let events = complete(
        SseMode::Legacy,
        "id: first\nid: rejected\0value\ndata: a\n\ndata: b\n\nid:\ndata: c\n\n",
    );
    assert_eq!(events[0].id(), Some("first"));
    assert_eq!(events[1].id(), None);
    assert_eq!(events[2].id(), Some(""));
    assert!(complete(SseMode::Legacy, "id: rejected\0value\n\n").is_empty());
    let mut decoder = decoder(SseMode::Legacy);
    assert!(feed(&mut decoder, b"id: uncommitted\n").unwrap().is_empty());
    assert_eq!(decoder.finish(), Err(SseError::IncompleteStream));
}

#[test]
fn fields_are_case_sensitive_and_strip_only_one_ascii_space() {
    let events = complete(
        SseMode::Legacy,
        "Data: ignored\nevent: old\nevent:  endpoint\nid: old\nid: new\ndata:  one\ndata:\ttwo\ndata\n\n",
    );
    assert_eq!(events[0].event(), Some(" endpoint"));
    assert_eq!(events[0].id(), Some("new"));
    assert_eq!(events[0].data(), " one\n\ttwo\n");
}

#[test]
fn retry_is_bounded_integer_evidence_not_a_reconnect_action() {
    for (value, expected) in [
        ("0", Some(0)),
        ("59999", Some(59999)),
        ("90000", Some(60_000)),
        ("4294967295", Some(60_000)),
        ("4294967296", None),
        ("-1", None),
        ("+5", Some(5)),
        ("-0", Some(0)),
        ("1__0", Some(10)),
        ("_5", None),
        ("5_", None),
        ("", None),
        (" 5", None),
        ("5 ", None),
        ("5.0", None),
        ("0x10", None),
        ("１", None),
    ] {
        let events = complete(SseMode::Legacy, &format!("retry: {value}\ndata: x\n\n"));
        assert_eq!(events[0].retry_ms(), expected, "{value}");
    }
    assert_eq!(
        complete(SseMode::Legacy, "retry: 7\nretry: invalid\ndata: x\n\n")[0].retry_ms(),
        Some(7)
    );
}

#[test]
fn bom_is_pinned_unknown_field_and_utf8_chunks_are_validated_once_complete() {
    for mode in [SseMode::Modern, SseMode::Legacy] {
        let text = "\u{feff}data: ignored\ndata: 🦀é\n\n";
        for split in 0..=text.len() {
            let mut decoder = decoder(mode);
            let mut events = feed(&mut decoder, &text.as_bytes()[..split]).unwrap();
            events.extend(feed(&mut decoder, &text.as_bytes()[split..]).unwrap());
            decoder.finish().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].data(), "🦀é");
        }
        assert_eq!(
            complete(mode, "data: \u{feff}kept\n\n")[0].data(),
            "\u{feff}kept"
        );
    }
}

#[test]
fn invalid_encoding_even_in_ignored_fields_poison_and_release() {
    for bytes in [
        b"data:\xff\n".as_slice(),
        b":\xff\n",
        b"unknown:\xc0\xaf\n",
        b"id:\xed\xa0\x80\n",
    ] {
        let mut decoder = decoder(SseMode::Modern);
        assert_eq!(feed(&mut decoder, bytes), Err(SseError::InvalidUtf8));
        assert_eq!(decoder.push(b"data: x\n\n"), Err(SseError::Closed));
        assert_eq!(decoder.finish(), Err(SseError::Closed));
        assert!(format!("{decoder:?}").contains("line_bytes: 0"));
    }
    let mut decoder = decoder(SseMode::Legacy);
    decoder.push(b"data:\xf0\x9f").unwrap();
    assert_eq!(decoder.finish(), Err(SseError::InvalidUtf8));
}

#[test]
fn incomplete_eof_never_dispatches_and_cr_is_already_a_terminator() {
    for mode in [SseMode::Modern, SseMode::Legacy] {
        for text in ["data: x", "data: x\n", "data: x\r", ": partial"] {
            let mut decoder = decoder(mode);
            assert!(feed(&mut decoder, text.as_bytes()).unwrap().is_empty());
            assert_eq!(decoder.finish(), Err(SseError::IncompleteStream));
        }
        for text in ["", ": done\n", "unknown\r", "data: x\r\r"] {
            let mut decoder = decoder(mode);
            feed(&mut decoder, text.as_bytes()).unwrap();
            decoder.finish().unwrap();
            assert_eq!(decoder.push(b""), Err(SseError::Closed));
        }
    }
    let mut modern = decoder(SseMode::Modern);
    feed(&mut modern, b"data:\n").unwrap();
    modern.finish().unwrap();
    let mut legacy = decoder(SseMode::Legacy);
    feed(&mut legacy, b"data:\n").unwrap();
    assert_eq!(legacy.finish(), Err(SseError::IncompleteStream));
}

#[test]
fn single_push_returns_only_one_event_and_only_charges_consumed_tail() {
    let text = b"data:x\n\ndata:y\n\n";
    let limits = SseLimits {
        max_total_bytes: 8,
        ..SseLimits::default()
    };
    let mut decoder = SseDecoder::new(SseMode::Modern, limits).unwrap();
    let progress = decoder.push(text).unwrap();
    assert_eq!(progress.consumed, 8);
    assert_eq!(progress.event.unwrap().data(), "x");
    assert_eq!(decoder.push(&text[8..]), Err(SseError::TotalLimit));
}

#[test]
fn ignored_bytes_and_empty_lines_have_finite_per_push_and_total_budgets() {
    let limits = SseLimits {
        max_push_bytes: 3,
        max_total_bytes: 7,
        ..SseLimits::default()
    };
    let mut decoder = SseDecoder::new(SseMode::Modern, limits).unwrap();
    for _ in 0..2 {
        assert_eq!(
            decoder.push(b"\n\n\n\n\n"),
            Ok(SseProgress {
                consumed: 3,
                event: None
            })
        );
    }
    assert_eq!(
        decoder.push(b"\n"),
        Ok(SseProgress {
            consumed: 1,
            event: None
        })
    );
    assert_eq!(
        decoder.push(b""),
        Ok(SseProgress {
            consumed: 0,
            event: None
        })
    );
    assert_eq!(decoder.push(b"\n"), Err(SseError::TotalLimit));
}

#[test]
fn exact_line_data_and_metadata_bounds() {
    let limits = SseLimits {
        max_line_bytes: 8,
        max_data_bytes: 3,
        max_field_bytes: 2,
        ..SseLimits::default()
    };
    let mut decoder = SseDecoder::new(SseMode::Legacy, limits).unwrap();
    let event = feed(&mut decoder, b"data:abc\nid:xy\n\n")
        .unwrap()
        .remove(0);
    assert_eq!(event.data(), "abc");
    assert_eq!(event.id(), Some("xy"));
    for (text, expected) in [
        ("unknownxx", SseError::LineLimit),
        ("data:abcd\n", SseError::LineLimit),
        ("data:a\ndata:bc\n", SseError::DataLimit),
        ("id:xyz\n", SseError::FieldLimit),
        ("event:xyz\n", SseError::LineLimit),
    ] {
        let mut decoder = SseDecoder::new(SseMode::Legacy, limits).unwrap();
        assert_eq!(feed(&mut decoder, text.as_bytes()), Err(expected));
    }
}

#[test]
fn ignored_fields_count_and_blank_separators_reset_only_block_budget() {
    let limits = SseLimits {
        max_fields: 2,
        max_events: 1,
        ..SseLimits::default()
    };
    let mut decoder = SseDecoder::new(SseMode::Modern, limits).unwrap();
    assert!(feed(&mut decoder, b":one\nunknown\n\n").unwrap().is_empty());
    assert_eq!(feed(&mut decoder, b"data:x\n\n").unwrap().len(), 1);
    assert_eq!(feed(&mut decoder, b"data:y\n\n"), Err(SseError::EventLimit));
    let mut decoder = SseDecoder::new(SseMode::Modern, limits).unwrap();
    assert_eq!(
        feed(&mut decoder, b":one\nunknown\n:three\n"),
        Err(SseError::FieldCountLimit)
    );
}

#[test]
fn limits_reject_zero_and_values_above_every_hard_ceiling() {
    let base = SseLimits::default();
    let invalid = [
        SseLimits {
            max_line_bytes: 0,
            ..base
        },
        SseLimits {
            max_line_bytes: 16 * 1024 * 1024 + 1,
            ..base
        },
        SseLimits {
            max_data_bytes: 0,
            ..base
        },
        SseLimits {
            max_data_bytes: 16 * 1024 * 1024 + 1,
            ..base
        },
        SseLimits {
            max_field_bytes: 0,
            ..base
        },
        SseLimits {
            max_field_bytes: 65537,
            ..base
        },
        SseLimits {
            max_fields: 0,
            ..base
        },
        SseLimits {
            max_fields: 16385,
            ..base
        },
        SseLimits {
            max_events: 0,
            ..base
        },
        SseLimits {
            max_events: 65537,
            ..base
        },
        SseLimits {
            max_total_bytes: 0,
            ..base
        },
        SseLimits {
            max_total_bytes: 1024 * 1024 * 1024 + 1,
            ..base
        },
        SseLimits {
            max_push_bytes: 0,
            ..base
        },
        SseLimits {
            max_push_bytes: 65537,
            ..base
        },
    ];
    for limits in invalid {
        assert_eq!(
            SseDecoder::new(SseMode::Modern, limits).unwrap_err(),
            SseError::InvalidLimits
        );
    }
}

#[test]
fn tiny_chunks_and_many_data_lines_allocate_geometrically() {
    let limits = SseLimits {
        max_line_bytes: 4096,
        max_data_bytes: 4096,
        max_fields: 4096,
        ..SseLimits::default()
    };
    let mut decoder = SseDecoder::new(SseMode::Modern, limits).unwrap();
    decoder.push(b"data:").unwrap();
    allocation_counter::measure(|| {});
    let allocations = allocation_counter::measure(|| {
        for _ in 0..4000 {
            decoder.push(b"x").unwrap();
        }
    });
    assert!(allocations.count_total <= 12, "{allocations:?}");
    assert_eq!(feed(&mut decoder, b"\n\n").unwrap()[0].data().len(), 4000);
    let mut decoder = SseDecoder::new(SseMode::Legacy, limits).unwrap();
    let allocations = allocation_counter::measure(|| {
        for _ in 0..2000 {
            decoder.push(b"data:x\n").unwrap();
        }
    });
    assert!(allocations.count_total <= 20, "{allocations:?}");
    assert_eq!(
        decoder.push(b"\n").unwrap().event.unwrap().data().len(),
        3999
    );
}

#[test]
fn debug_and_errors_do_not_expose_payloads_or_identity() {
    let mut decoder = decoder(SseMode::Legacy);
    decoder
        .push(b"data: secret-token\nid: secret-id\nevent: secret-kind\n")
        .unwrap();
    assert!(!format!("{decoder:?}").contains("secret"));
    let progress = decoder.push(b"\n").unwrap();
    assert!(!format!("{progress:?}").contains("secret"));
    assert!(!format!("{}", SseError::InvalidUtf8).contains("secret"));
}
