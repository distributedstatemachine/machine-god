use super::*;

fn feed(editor: &mut Composer, mut bytes: &[u8], active: bool) -> Vec<ComposerEvent> {
    let mut events = Vec::new();
    while !bytes.is_empty() {
        let (consumed, event) = editor.feed(
            bytes,
            ComposerContext {
                active_response: active,
            },
        );
        assert!(consumed > 0 && consumed <= MAX_COMPOSER_STEP_BYTES && consumed <= bytes.len());
        assert!(editor.text().len() <= MAX_COMPOSER_BYTES);
        assert!(editor.text().is_char_boundary(editor.cursor()));
        bytes = &bytes[consumed..];
        events.extend(event);
    }
    events
}

fn error(events: &[ComposerEvent], expected: ComposerInputError) -> bool {
    events
        .iter()
        .any(|event| matches!(event, ComposerEvent::InputError(actual) if *actual == expected))
}
fn submitted(events: &[ComposerEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            ComposerEvent::Submit(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl_d_exits_only_empty_idle_and_never_means_physical_eof() {
    let mut editor = Composer::default();
    assert!(matches!(
        feed(&mut editor, &[4], false).as_slice(),
        [ComposerEvent::ExitRequested]
    ));
    assert!(feed(&mut editor, &[4], true).is_empty());
    feed(&mut editor, b"abc", false);
    assert!(
        feed(&mut editor, &[4], false).is_empty(),
        "draft at end must not exit"
    );
    feed(&mut editor, b"\x1b[D", false);
    assert!(matches!(
        feed(&mut editor, &[4], false).as_slice(),
        [ComposerEvent::Changed]
    ));
    assert_eq!(editor.text(), "ab");
    assert_eq!(editor.cursor(), 2);
    feed(&mut editor, b"\x01\x04", true);
    assert_eq!(editor.text(), "b");
    assert_eq!(editor.cursor(), 0);
}

#[test]
fn empty_calls_are_inert_and_submit_stops_before_remainder() {
    let mut editor = Composer::default();
    assert!(editor.is_empty());
    assert_eq!(editor.feed(&[], ComposerContext::default()).0, 0);
    assert!(editor.feed(&[], ComposerContext::default()).1.is_none());
    let (consumed, event) = editor.feed(b"hello\r\nnext", ComposerContext::default());
    assert_eq!(consumed, 5);
    assert!(matches!(event, Some(ComposerEvent::Changed)));
    let (consumed, event) = editor.feed(b"\r\nnext", ComposerContext::default());
    assert_eq!(consumed, 1);
    assert!(matches!(event, Some(ComposerEvent::Submit(text)) if text == "hello"));
    let events = feed(&mut editor, b"\nnext\n", false);
    assert_eq!(submitted(&events), ["next"]);
}

#[test]
fn incremental_utf8_and_cursor_insertion_are_safe_at_every_split() {
    for split in 0..="a🦀e\u{301}z".len() {
        let mut editor = Composer::default();
        let text = "a🦀e\u{301}z".as_bytes();
        feed(&mut editor, &text[..split], false);
        feed(&mut editor, &text[split..], false);
        assert_eq!(editor.text(), "a🦀e\u{301}z");
        feed(&mut editor, b"\x1b[D\x1b[D", false);
        assert_eq!(editor.cursor(), "a🦀".len());
        feed(&mut editor, "插".as_bytes(), false);
        assert_eq!(editor.text(), "a🦀插e\u{301}z");
        feed(&mut editor, &[127], false);
        assert_eq!(editor.text(), "a🦀e\u{301}z");
    }
}

#[test]
fn pinned_display_units_group_combining_rgi_keycaps_and_variation_selectors() {
    for unit in ["e\u{301}", "👩‍💻", "🇺🇸", "1\u{fe0f}\u{20e3}", "☀\u{fe0f}"] {
        let mut editor = Composer::default();
        feed(&mut editor, format!("a{unit}z").as_bytes(), false);
        feed(&mut editor, b"\x1b[D\x1b[D", false);
        assert_eq!(editor.cursor(), 1, "{unit}");
        feed(&mut editor, b"\x1b[3~", false);
        assert_eq!(editor.text(), "az");
        feed(&mut editor, b"\x1b[H\x1b[C", false);
        assert_eq!(editor.cursor(), 1);
    }
}

#[test]
fn home_end_ss3_and_control_shortcuts_match_cursor_edits() {
    let mut editor = Composer::default();
    feed(&mut editor, b"abcd\x1bOHx\x1bOFy", false);
    assert_eq!(editor.text(), "xabcdy");
    feed(&mut editor, b"\x01\x06\x04\x05\x02\x08", false);
    assert_eq!(editor.text(), "xbcy");
    assert_eq!(editor.cursor(), 3);
    feed(&mut editor, b"\x1b[7~\x7f\x1b[8~\x1b[3~", false);
    assert_eq!(editor.text(), "xbcy");
}

#[test]
fn ctrl_c_preserves_active_draft_clears_idle_and_interrupts_partial_decoders() {
    let mut editor = Composer::default();
    feed(&mut editor, b"draft\x1b[", false);
    assert!(matches!(
        feed(&mut editor, &[3], true).as_slice(),
        [ComposerEvent::CancelRequested]
    ));
    assert_eq!(editor.text(), "draft");
    feed(&mut editor, &[0xf0], false);
    assert!(matches!(
        feed(&mut editor, &[3], false).as_slice(),
        [ComposerEvent::CancelRequested]
    ));
    assert!(editor.is_empty());
    feed(&mut editor, b"ok\n", false);
    assert!(editor.is_empty());
}

#[test]
fn ordinary_invalid_utf8_and_nul_preserve_draft_and_drain_without_submission() {
    for (bad, expected) in [
        (vec![0xff], ComposerInputError::InvalidUtf8),
        (vec![0], ComposerInputError::ContainsNul),
    ] {
        let mut editor = Composer::default();
        feed(&mut editor, b"keep", false);
        let events = feed(&mut editor, &bad, false);
        assert!(error(&events, expected));
        let events = feed(&mut editor, b"not a command\r\n", false);
        assert!(submitted(&events).is_empty());
        assert_eq!(editor.text(), "keep");
        let events = feed(&mut editor, b"\n", false);
        assert_eq!(submitted(&events), ["keep"]);
    }
}

#[test]
fn incomplete_utf8_at_enter_rejects_instead_of_submitting_prefix() {
    let mut editor = Composer::default();
    feed(&mut editor, b"keep\xf0\x9f", false);
    let events = feed(&mut editor, b"\r\n", false);
    assert!(error(&events, ComposerInputError::InvalidUtf8));
    assert!(submitted(&events).is_empty());
    assert_eq!(editor.text(), "keep");
    assert_eq!(submitted(&feed(&mut editor, b"\n", false)), ["keep"]);
}

#[test]
fn exact_limit_accepts_and_overflow_rejects_the_whole_attempt_without_truncation() {
    let mut editor = Composer::default();
    feed(&mut editor, &vec![b'x'; MAX_COMPOSER_BYTES], false);
    assert_eq!(editor.text().len(), MAX_COMPOSER_BYTES);
    let events = feed(&mut editor, b"extra\n", false);
    assert!(error(&events, ComposerInputError::TooLong));
    assert!(submitted(&events).is_empty());
    assert_eq!(editor.text().len(), MAX_COMPOSER_BYTES);
    feed(&mut editor, &[127], false);
    assert_eq!(editor.text().len(), MAX_COMPOSER_BYTES - 1);
    let events = feed(&mut editor, b"\n", false);
    assert_eq!(submitted(&events)[0].len(), MAX_COMPOSER_BYTES - 1);
}

#[test]
fn bracketed_paste_is_atomic_across_all_marker_splits_and_controls_are_data() {
    let paste = b"\x1b[200~hello\r\nworld\r!\x03\x04\x1b[201~";
    for split in 0..=paste.len() {
        let mut editor = Composer::default();
        let mut events = feed(&mut editor, &paste[..split], false);
        events.extend(feed(&mut editor, &paste[split..], false));
        assert!(
            events
                .iter()
                .all(|event| matches!(event, ComposerEvent::Changed))
        );
        assert_eq!(editor.text(), "hello\nworld\n!\x03\x04");
        assert_eq!(editor.cursor(), editor.text().len());
    }
}

#[test]
fn paste_does_not_publish_a_prefix_before_the_end_and_inserts_at_cursor() {
    let mut editor = Composer::default();
    feed(&mut editor, b"ac\x1b[D\x1b[200~b", false);
    assert_eq!(editor.text(), "ac");
    assert_eq!(editor.cursor(), 1);
    feed(&mut editor, b"\x1b[201~", false);
    assert_eq!(editor.text(), "abc");
    assert_eq!(editor.cursor(), 2);
    feed(&mut editor, b"\x1b[200~\x1b[20x\x1b\x1b[201~", false);
    assert_eq!(editor.text(), "ab\x1b[20x\x1bc");
}

#[test]
fn invalid_and_overlong_paste_discard_to_marker_preserving_original_draft() {
    for bad in [vec![b'z'; MAX_COMPOSER_BYTES], vec![0xff], vec![0]] {
        let mut editor = Composer::default();
        feed(&mut editor, b"keep\x1b[200~", false);
        let events = feed(&mut editor, &bad, false);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ComposerEvent::InputError(_)))
                .count(),
            1
        );
        assert_eq!(editor.text(), "keep");
        let events = feed(&mut editor, b"\n\x03\x04\x1b[201~", false);
        assert!(
            events
                .iter()
                .all(|event| matches!(event, ComposerEvent::Changed))
        );
        assert_eq!(editor.text(), "keep");
        assert_eq!(submitted(&feed(&mut editor, b"\n", false)), ["keep"]);
    }
}

#[test]
fn incomplete_paste_utf8_and_exact_capacity_are_checked_at_atomic_commit() {
    let mut editor = Composer::default();
    feed(&mut editor, b"keep\x1b[200~\xf0\x9f", false);
    let events = feed(&mut editor, b"\x1b[201~", false);
    assert!(error(&events, ComposerInputError::InvalidUtf8));
    assert_eq!(editor.text(), "keep");
    feed(&mut editor, b"\x1b[200~", false);
    feed(&mut editor, &vec![b'a'; MAX_COMPOSER_BYTES - 4], false);
    feed(&mut editor, b"\x1b[201~", false);
    assert_eq!(editor.text().len(), MAX_COMPOSER_BYTES);
}

#[test]
fn bounded_escape_recovery_never_submits_unknown_sequence_suffixes() {
    let mut editor = Composer::default();
    feed(&mut editor, b"keep\x1b[", false);
    let events = feed(&mut editor, &vec![b'1'; 2 * MAX_COMPOSER_STEP_BYTES], false);
    assert!(error(&events, ComposerInputError::InvalidEscape));
    assert!(submitted(&feed(&mut editor, b"malformed\r\n", false)).is_empty());
    assert_eq!(editor.text(), "keep");
    assert!(error(
        &feed(&mut editor, b"\x1b[A", false),
        ComposerInputError::InvalidEscape
    ));
    assert!(submitted(&feed(&mut editor, b"\x1b]\x1b]bad\r\n", false)).is_empty());
    assert_eq!(editor.text(), "keep");
}

#[test]
fn reset_discards_every_partial_state_and_debug_redacts_text() {
    for suffix in [
        b"\xf0".as_slice(),
        b"\x1b[",
        b"\x1b[200~private",
        b"\0ignored",
    ] {
        let mut editor = Composer::default();
        feed(&mut editor, b"secret", false);
        feed(&mut editor, suffix, false);
        assert!(!format!("{editor:?}").contains("secret"));
        editor.reset();
        assert!(editor.is_empty());
        assert_eq!(editor.cursor(), 0);
        let events = feed(&mut editor, b"new\n", false);
        assert_eq!(submitted(&events), ["new"]);
        assert!(!format!("{:?}", events.last().unwrap()).contains("new"));
    }
}

#[test]
fn byte_corpus_keeps_cursor_and_draft_bounds_and_reset_recovers() {
    for byte in 0..=u8::MAX {
        for prefix in [b"".as_slice(), b"\xf0", b"\x1b[", b"\x1b]", b"\x1b[200~"] {
            let mut editor = Composer::default();
            feed(&mut editor, "a🦀".as_bytes(), false);
            feed(&mut editor, prefix, false);
            feed(&mut editor, &[byte, b'\r', b'\n', 3, b'x'], true);
            editor.reset();
            assert_eq!(submitted(&feed(&mut editor, b"safe\n", false)), ["safe"]);
        }
    }
}

#[test]
fn plain_text_batch_budget_and_paste_end_leave_exact_remainders() {
    let mut editor = Composer::default();
    let bytes = vec![b'x'; MAX_COMPOSER_STEP_BYTES + 20];
    let (consumed, event) = editor.feed(&bytes, ComposerContext::default());
    assert_eq!(consumed, MAX_COMPOSER_STEP_BYTES);
    assert!(matches!(event, Some(ComposerEvent::Changed)));
    assert_eq!(editor.text().len(), consumed);
    editor.reset();
    let input = b"\x1b[200~draft\x1b[201~\ntrailing";
    let (consumed, event) = editor.feed(input, ComposerContext::default());
    assert_eq!(&input[consumed..], b"\ntrailing");
    assert!(matches!(event, Some(ComposerEvent::Changed)));
    assert_eq!(editor.text(), "draft");
}
