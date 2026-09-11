use super::*;

fn pattern(source: &str, text: &str, valid: bool) {
    let schema = serde_json::to_string(&serde_json::json!({"pattern": source})).unwrap();
    check(&schema, &serde_json::to_string(text).unwrap(), valid);
}

#[test]
fn pinned_unicode_letter_whitespace_and_absolute_anchors() {
    for text in ["a", "é", "λ", "Ж", "中", "𐐀"] {
        pattern(r"^\p{L}+$", text, true);
        pattern(r"^\P{Letter}+$", text, false);
    }
    for text in ["1", "_", "😀", "\u{0870}"] {
        // U+0870 became a letter after the producer's Unicode 13 table.
        pattern(r"^\p{Letter}+$", text, false);
        pattern(r"^\P{L}+$", text, true);
    }
    for codepoint in [
        0x9, 0xA, 0xB, 0xC, 0xD, 0x20, 0xA0, 0x1680, 0x2000, 0x200A, 0x2028, 0x2029, 0x202F,
        0x205F, 0x3000, 0xFEFF,
    ] {
        let text = char::from_u32(codepoint).unwrap().to_string();
        pattern(r"^\s$", &text, true);
        pattern(r"^[\S]$", &text, false);
    }
    for text in ["\u{0085}", "\u{180e}", "x"] {
        pattern(r"^\S$", text, true);
        pattern(r"^[\s]$", text, false);
    }
    for text in ["\n", "\r", "\u{2028}", "\u{2029}"] {
        pattern(".", text, false);
    }
    pattern("a$", "a\n", false);
    pattern("^a", "\na", false);
    pattern(r"^\w+$", "AZ_09", true);
    pattern(r"^\w+$", "é", false);
    pattern(r"^\u{1F600}$", "😀", true);
    pattern(r"^[\u0041-\x5a]+$", "AZ", true);
}

#[test]
fn bounded_thompson_repetition_and_sparse_matching() {
    for (source, text, valid) in [
        ("a{0}", "", true),
        ("^a{2,4}$", "aaa", true),
        ("^a{2,4}$", "aaaaa", false),
        ("^(ab|c)+?$", "abcab", true),
        ("^a{2,}$", "a", false),
        ("^a{2,}$", "aaaa", true),
        ("^(()*)*$", "", true),
        ("^(()*)*$", "x", false),
        ("a|", "", true),
        ("[-a]+", "-aa", true),
    ] {
        pattern(source, text, valid);
    }
    pattern("^a{1024}$", &"a".repeat(1024), true);
    pattern("z{1024}", &"a".repeat(1000), false);
    let limits = McpSchemaLimits {
        max_pattern_states: 5,
        ..McpSchemaLimits::default()
    };
    McpSchema::parse(br#"{"pattern":"abc"}"#, limits).unwrap();
    assert_eq!(
        McpSchema::parse(br#"{"pattern":"abcd"}"#, limits).unwrap_err(),
        McpSchemaError::SchemaLimitExceeded
    );
}
