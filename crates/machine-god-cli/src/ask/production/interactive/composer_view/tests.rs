use super::{ComposerViewError, MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, clear_row, render};
use machine_god_native::native_terminal_display_unit_at;

fn decoded(bytes: &[u8]) -> (&str, usize) {
    assert!(bytes.len() <= MAX_OUTPUT_BYTES);
    let text = std::str::from_utf8(bytes).unwrap();
    assert!(text.starts_with(std::str::from_utf8(clear_row()).unwrap()));
    let (body, position) = text[clear_row().len()..].rsplit_once("\x1b[").unwrap();
    assert!(!body.contains(['\r', '\n', '\x1b']));
    (body, position.strip_suffix('G').unwrap().parse().unwrap())
}

fn width(text: &str) -> usize {
    let mut index = 0;
    let mut cells = 0;
    while index < text.len() {
        let unit = native_terminal_display_unit_at(text, index).unwrap();
        cells += usize::from(unit.cell_width);
        index += unit.byte_len;
    }
    cells
}

#[test]
fn empty_and_narrow_views_reserve_the_final_column() {
    for (columns, expected, position) in [(1, "", 1), (2, ">", 2), (3, "> ", 3), (80, "> ", 3)] {
        let rendered = render("", 0, columns).unwrap();
        assert_eq!(decoded(&rendered), (expected, position));
        assert!(width(expected) < usize::from(columns));
    }
    for columns in 1..=3 {
        let rendered = render("not discarded\n", 4, columns).unwrap();
        let (body, position) = decoded(&rendered);
        assert!(width(body) < usize::from(columns));
        assert!(position <= usize::from(columns));
    }
}

#[test]
fn validation_is_bounded_and_errors_do_not_disclose_the_draft() {
    assert_eq!(
        render("private", 0, 0),
        Err(ComposerViewError::InvalidColumns)
    );
    assert_eq!(
        render("private", 8, 80),
        Err(ComposerViewError::InvalidCursor)
    );
    assert_eq!(render("é", 1, 80), Err(ComposerViewError::InvalidCursor));
    assert_eq!(
        render("", usize::MAX, 80),
        Err(ComposerViewError::InvalidCursor)
    );
    assert_eq!(
        render(&"s".repeat(MAX_INPUT_BYTES + 1), 0, 80),
        Err(ComposerViewError::InputLimit)
    );
    assert_eq!(
        ComposerViewError::InvalidCursor.to_string(),
        "composer view unavailable"
    );
    assert_eq!(format!("{:?}", ComposerViewError::InputLimit), "InputLimit");
}

#[test]
fn small_ascii_draft_and_byte_cursor_have_exact_row_positions() {
    for cursor in 0..=6 {
        let rendered = render("abcdef", cursor, 80).unwrap();
        assert_eq!(decoded(&rendered), ("> abcdef", cursor + 3));
    }
    assert_eq!(clear_row(), b"\r\x1b[2K");
}

#[test]
fn huge_draft_view_follows_the_real_middle_and_end_instead_of_truncating_a_prefix() {
    let text = format!("{}CURSOR{}END", "l".repeat(8192), "r".repeat(8192));
    let rendered = render(&text, 8192, 23).unwrap();
    let (body, position) = decoded(&rendered);
    assert!(body.contains("CURSOR"));
    assert_eq!(&body[position - 1..position - 1 + 6], "CURSOR");
    assert_eq!(position, 13);
    let rendered = render(&text, text.len(), 23).unwrap();
    let (body, position) = decoded(&rendered);
    assert!(body.ends_with("END"));
    assert_eq!(position, width(body) + 1);
    assert_eq!(text.len(), 16_393);
}

#[test]
fn pasted_controls_and_standalone_formats_are_visible_non_executable_text() {
    let text = "x\n\r\t\x1b[2J\0\u{7f}\u{85}\u{202e}\u{2066}\u{200d}\u{fe0f}\\";
    let rendered = render(text, 0, 200).unwrap();
    let (body, position) = decoded(&rendered);
    assert_eq!(
        body,
        "> x\\n\\r\\t\\u{1b}[2J\\u{0}\\u{7f}\\u{85}\\u{202e}\\u{2066}\\u{200d}\\u{fe0f}\\\\"
    );
    assert_eq!(position, 3);
    assert!(!body.chars().any(char::is_control));
}

#[test]
fn format_ranges_and_supplementary_scalars_are_escaped_without_splitting_utf8() {
    let text = "\u{ad}\u{600}\u{61c}\u{6dd}\u{70f}\u{890}\u{8e2}\u{180e}\u{200b}\u{2028}\u{2060}\u{feff}\u{fff9}\u{110bd}\u{110cd}\u{13430}\u{1bca0}\u{1d173}\u{e0001}\u{e0020}\u{e0100}";
    let rendered = render(text, 0, 500).unwrap();
    let (body, _) = decoded(&rendered);
    assert!(body.is_ascii());
    assert!(body.ends_with("\\u{e0100}"));
    assert_eq!(body.matches("\\u{").count(), text.chars().count());
}

#[test]
fn native_emoji_sequences_keep_their_width_and_internal_cursors_map_to_the_leading_cell() {
    let emoji = "👩‍💻";
    for cursor in [0, 4, 7] {
        let rendered = render(emoji, cursor, 20).unwrap();
        assert_eq!(decoded(&rendered), ("> 👩‍💻", 3));
    }
    let rendered = render(emoji, emoji.len(), 20).unwrap();
    assert_eq!(decoded(&rendered), ("> 👩‍💻", 5));
    for text in ["1\u{fe0f}\u{20e3}", "❤️", "👩🏽‍💻", "🇬🇧"] {
        let rendered = render(text, text.len(), 20).unwrap();
        let (body, position) = decoded(&rendered);
        assert_eq!(body, format!("> {text}"));
        assert_eq!(position, 5);
    }
}

#[test]
fn combining_marks_stay_with_their_base_and_never_modify_the_prompt() {
    let text = "a\u{301}b";
    let rendered = render(text, 1, 20).unwrap();
    assert_eq!(decoded(&rendered), ("> a\u{301}b", 4));
    let rendered = render("\u{301}a", 0, 20).unwrap();
    assert_eq!(decoded(&rendered), ("> \\u{301}a", 3));
    let text = "aaaaaZ\u{301}b";
    let rendered = render(text, text.len(), 5).unwrap();
    assert_eq!(decoded(&rendered), ("> Z\u{301}b", 5));
}

#[test]
fn oversized_combining_cluster_has_an_explicit_marker_and_bounded_storage() {
    let text = format!("a{}", "\u{301}".repeat((MAX_INPUT_BYTES - 1) / 2));
    let rendered = render(&text, text.len(), 80).unwrap();
    assert_eq!(decoded(&rendered), ("> …", 4));
    let rendered = render(&text, 0, 80).unwrap();
    assert_eq!(decoded(&rendered), ("> …", 3));
}

#[test]
fn cells_and_bytes_are_independently_bounded_for_every_cursor_and_narrow_width() {
    let text = "A界👩‍💻e\u{301}\n\u{202e}1\u{fe0f}\u{20e3}Z";
    for columns in 1..=32 {
        for cursor in text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
        {
            let rendered = render(text, cursor, columns).unwrap();
            let (body, position) = decoded(&rendered);
            assert!(width(body) < usize::from(columns), "{body:?} at {columns}");
            assert!((1..=usize::from(columns)).contains(&position));
            assert!(!body.contains(['\u{202e}', '\n']));
        }
    }
}

#[test]
fn maximum_input_and_column_count_never_exceed_the_output_byte_bound() {
    for text in [
        "a".repeat(MAX_INPUT_BYTES),
        "界".repeat(MAX_INPUT_BYTES / 3),
        "\0".repeat(MAX_INPUT_BYTES),
    ] {
        for cursor in [0, text.len() / 2 / 3 * 3, text.len()] {
            if !text.is_char_boundary(cursor) {
                continue;
            }
            let rendered = render(&text, cursor, u16::MAX).unwrap();
            let (body, position) = decoded(&rendered);
            assert!(rendered.len() <= MAX_OUTPUT_BYTES);
            assert!(position <= width(body) + 1);
        }
    }
}

#[test]
fn a_unit_wider_than_the_entire_viewport_uses_a_marker_not_half_a_cell() {
    for text in ["界", "\u{202e}", "👩‍💻"] {
        let rendered = render(text, 0, 4).unwrap();
        assert_eq!(decoded(&rendered), ("> …", 3));
        let rendered = render(text, text.len(), 4).unwrap();
        assert_eq!(decoded(&rendered), ("> …", 4));
    }
}
