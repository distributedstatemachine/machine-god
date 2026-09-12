use super::*;

fn decoder() -> SseDecoder {
    SseDecoder::new(SseLimits::default()).unwrap()
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

fn complete(text: &str) -> Vec<SseEvent> {
    let mut decoder = decoder();
    let events = feed(&mut decoder, text.as_bytes()).unwrap();
    decoder.finish().unwrap();
    events
}

#[test]
fn modern_data_and_ignored_metadata_all_two_chunk_partitions() {
    for delimiter in ["\n", "\r", "\r\n"] {
        let text = format!(
            "event: message{delimiter}data: one{delimiter}data: two{delimiter}id: event-1{delimiter}retry: 90000{delimiter}{delimiter}"
        );
        for split in 0..=text.len() {
            let mut decoder = decoder();
            let mut events = feed(&mut decoder, &text.as_bytes()[..split]).unwrap();
            events.extend(feed(&mut decoder, &text.as_bytes()[split..]).unwrap());
            decoder.finish().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].data(), "one\ntwo");
        }
    }
}

#[test]
fn exhaustive_partitions_preserve_crlf_and_empty_blocks() {
    let bytes = b"data:x\r\n\r\n";
    {
        for mask in 0..(1 << (bytes.len() - 1)) {
            let mut decoder = decoder();
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
fn modern_empty_data_and_metadata_are_ignored() {
    let text = "data:\ndata:\ndata: x\ndata:\n\n";
    assert_eq!(complete(text)[0].data(), "x\n");
    assert!(complete("data:\n\n").is_empty());
    assert!(complete("event: endpoint\nid: 1\nretry: 5\n\n").is_empty());
}

#[test]
fn fields_are_case_sensitive_and_strip_only_one_ascii_space() {
    let events = complete(
        "Data: ignored\nevent: old\nevent:  endpoint\nid: old\nid: new\ndata:  one\ndata:\ttwo\ndata\n\n",
    );
    assert_eq!(events[0].data(), " one\n\ttwo\n");
}

#[test]
fn bom_is_pinned_unknown_field_and_utf8_chunks_are_validated_once_complete() {
    {
        let text = "\u{feff}data: ignored\ndata: 🦀é\n\n";
        for split in 0..=text.len() {
            let mut decoder = decoder();
            let mut events = feed(&mut decoder, &text.as_bytes()[..split]).unwrap();
            events.extend(feed(&mut decoder, &text.as_bytes()[split..]).unwrap());
            decoder.finish().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].data(), "🦀é");
        }
        assert_eq!(complete("data: \u{feff}kept\n\n")[0].data(), "\u{feff}kept");
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
        let mut decoder = decoder();
        assert_eq!(feed(&mut decoder, bytes), Err(SseError::InvalidUtf8));
        assert_eq!(decoder.push(b"data: x\n\n"), Err(SseError::Closed));
        assert_eq!(decoder.finish(), Err(SseError::Closed));
        assert!(format!("{decoder:?}").contains("line_bytes: 0"));
    }
    let mut decoder = decoder();
    decoder.push(b"data:\xf0\x9f").unwrap();
    assert_eq!(decoder.finish(), Err(SseError::InvalidUtf8));
}

#[test]
fn incomplete_eof_never_dispatches_and_cr_is_already_a_terminator() {
    {
        for text in ["data: x", "data: x\n", "data: x\r", ": partial"] {
            let mut decoder = decoder();
            assert!(feed(&mut decoder, text.as_bytes()).unwrap().is_empty());
            assert_eq!(decoder.finish(), Err(SseError::IncompleteStream));
        }
        for text in ["", ": done\n", "unknown\r", "data: x\r\r"] {
            let mut decoder = decoder();
            feed(&mut decoder, text.as_bytes()).unwrap();
            decoder.finish().unwrap();
            assert_eq!(decoder.push(b""), Err(SseError::Closed));
        }
    }
    let mut modern = decoder();
    feed(&mut modern, b"data:\n").unwrap();
    modern.finish().unwrap();
}

#[test]
fn single_push_returns_only_one_event_and_only_charges_consumed_tail() {
    let text = b"data:x\n\ndata:y\n\n";
    let limits = SseLimits {
        max_total_bytes: 8,
        ..SseLimits::default()
    };
    let mut decoder = SseDecoder::new(limits).unwrap();
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
    let mut decoder = SseDecoder::new(limits).unwrap();
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
fn exact_line_data_and_ignored_field_bounds() {
    let limits = SseLimits {
        max_line_bytes: 8,
        max_data_bytes: 3,
        ..SseLimits::default()
    };
    let mut decoder = SseDecoder::new(limits).unwrap();
    let event = feed(&mut decoder, b"data:abc\nid:xy\n\n")
        .unwrap()
        .remove(0);
    assert_eq!(event.data(), "abc");
    for (text, expected) in [
        ("unknownxx", SseError::LineLimit),
        ("data:abcd\n", SseError::LineLimit),
        ("data:a\ndata:bc\n", SseError::DataLimit),
        ("event:xyz\n", SseError::LineLimit),
    ] {
        let mut decoder = SseDecoder::new(limits).unwrap();
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
    let mut decoder = SseDecoder::new(limits).unwrap();
    assert!(feed(&mut decoder, b":one\nunknown\n\n").unwrap().is_empty());
    assert_eq!(feed(&mut decoder, b"data:x\n\n").unwrap().len(), 1);
    assert_eq!(feed(&mut decoder, b"data:y\n\n"), Err(SseError::EventLimit));
    let mut decoder = SseDecoder::new(limits).unwrap();
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
            SseDecoder::new(limits).unwrap_err(),
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
    let mut decoder = SseDecoder::new(limits).unwrap();
    decoder.push(b"data:").unwrap();
    allocation_counter::measure(|| {});
    let allocations = allocation_counter::measure(|| {
        for _ in 0..4000 {
            decoder.push(b"x").unwrap();
        }
    });
    assert!(allocations.count_total <= 12, "{allocations:?}");
    assert_eq!(feed(&mut decoder, b"\n\n").unwrap()[0].data().len(), 4000);
    let mut decoder = SseDecoder::new(limits).unwrap();
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
    let mut decoder = decoder();
    decoder
        .push(b"data: secret-token\nid: secret-id\nevent: secret-kind\n")
        .unwrap();
    assert!(!format!("{decoder:?}").contains("secret"));
    let progress = decoder.push(b"\n").unwrap();
    assert!(!format!("{progress:?}").contains("secret"));
    assert!(!format!("{}", SseError::InvalidUtf8).contains("secret"));
}
