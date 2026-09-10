use super::*;

fn parse(text: &str) -> Result<NativeSkillMetadata, NativeSkillMetadataError> {
    parse_skill_metadata(text.as_bytes(), "fallback")
}

#[test]
fn plain_body_is_opaque_and_uses_only_valid_basename() {
    let metadata = parse_skill_metadata(b"opaque\xff body", "test skill").unwrap();
    assert_eq!(metadata.name, "test skill");
    assert!(!metadata.has_frontmatter);
    assert_eq!(metadata.body_offset, 0);
    assert_eq!(
        parse_skill_metadata(b"body", "../oops"),
        Err(NativeSkillMetadataError::InvalidName)
    );
}

#[test]
fn exact_delimiters_and_body_offset() {
    let input = "---\r\nname: 'hello world'\r\ndescription: \"quoted\"\r\n---\r\n\nBody";
    let metadata = parse(input).unwrap();
    assert_eq!(metadata.name, "hello world");
    assert_eq!(metadata.description, "quoted");
    assert_eq!(&input[metadata.body_offset..], "\nBody");
    assert_eq!(parse(" ---\nname: bad\n---").unwrap().name, "fallback");
    assert_eq!(parse("---\nname: okay\n---").unwrap().name, "okay");
    assert_eq!(
        parse("---\nname: okay\n---\r"),
        Err(NativeSkillMetadataError::MissingClosingDelimiter)
    );
}

#[test]
fn supported_block_descriptions_preserve_pinned_semantics() {
    for (style, expected) in [
        (">", "first second\n\nparagraph\n"),
        (">-", "first second\n\nparagraph"),
        ("|", "first\nsecond\n\nparagraph\n"),
    ] {
        let input = format!(
            "---\nname: valid\ndescription: {style}\n  first\n  second\n\n  paragraph\n\n---\nbody"
        );
        assert_eq!(parse(&input).unwrap().description, expected);
    }
    assert_eq!(
        parse("---\ndescription: >-\n  first\n    indented\nname: after\n---\n")
            .unwrap()
            .description,
        "first   indented"
    );
    assert_eq!(
        parse("---\nname: empty\ndescription: >\n\n---\n")
            .unwrap()
            .description,
        ""
    );
}

#[test]
fn malformed_recognized_fields_never_fall_back() {
    use NativeSkillMetadataError as E;
    for (input, expected) in [
        ("---", E::MissingClosingDelimiter),
        ("---\ndescription: only\n---", E::MissingName),
        ("---\nname: a\nname: b\n---", E::DuplicateRecognizedKey),
        (
            "---\nname: a\ndescription: x\ndescription: y\n---",
            E::DuplicateRecognizedKey,
        ),
        ("---\nname: a\ndescription: 'bad\n---", E::MalformedQuote),
        (
            "---\nname: a\ndescription: >+\n  bad\n---",
            E::UnsupportedMultiline,
        ),
        ("---\nname: >\n  bad\n---", E::UnsupportedMultiline),
        ("---\nname: a\n  continuation\n---", E::UnsupportedMultiline),
        (
            "---\nname: a\ndescription: >\n   first\n  less\n---",
            E::UnsupportedMultiline,
        ),
        (
            "---\nname: a\ndescription: >\n\tbad\n---",
            E::UnsupportedMultiline,
        ),
        ("---\nname: a\ndescription: \u{b}bad\n---", E::ControlByte),
    ] {
        assert_eq!(parse(input), Err(expected), "{input:?}");
    }
}

#[test]
fn unknown_fields_and_body_do_not_become_metadata() {
    let input = b"---\nunknown: \xff\nname: valid\n---\ninvalid body \xff";
    assert_eq!(
        parse_skill_metadata(input, "fallback").unwrap().name,
        "valid"
    );
    assert_eq!(
        parse_skill_metadata(b"---\nname: \xff\n---", "fallback"),
        Err(NativeSkillMetadataError::InvalidUtf8)
    );
}

#[test]
fn quotes_do_not_add_yaml_escape_processing() {
    assert_eq!(
        parse("---\nname: okay\ndescription: \"literal\\n\\t\"\n---")
            .unwrap()
            .description,
        "literal\\n\\t"
    );
    assert_eq!(
        parse("---\nname: okay\ndescription: keeps  internal   spaces\n---")
            .unwrap()
            .description,
        "keeps  internal   spaces"
    );
}

#[test]
fn independent_inclusive_name_description_and_header_bounds() {
    let name = "n".repeat(MAX_NATIVE_SKILL_METADATA_NAME_BYTES);
    let description = "d".repeat(MAX_NATIVE_SKILL_DESCRIPTION_BYTES);
    assert!(
        parse(&format!(
            "---\nname: {name}\ndescription: {description}\n---"
        ))
        .is_ok()
    );
    assert_eq!(
        parse(&format!("---\nname: {name}x\n---")),
        Err(NativeSkillMetadataError::NameTooLong)
    );
    assert_eq!(
        parse(&format!("---\nname: n\ndescription: {description}x\n---")),
        Err(NativeSkillMetadataError::DescriptionTooLong)
    );
    let prefix = "---\nname: n\n#";
    let suffix = "\n---\n";
    let input = format!(
        "{prefix}{}{suffix}",
        "x".repeat(MAX_NATIVE_SKILL_HEADER_BYTES - prefix.len() - suffix.len())
    );
    assert_eq!(input.len(), MAX_NATIVE_SKILL_HEADER_BYTES);
    assert!(parse(&input).is_ok());
    assert_eq!(
        parse(&input.replace("#x", "#xx")),
        Err(NativeSkillMetadataError::HeaderTooLong)
    );
}

#[test]
fn debug_and_error_forms_do_not_disclose_text() {
    let metadata = parse("---\nname: private-name\ndescription: private-description\n---").unwrap();
    let debug = format!("{metadata:?}");
    assert!(!debug.contains("private"));
    assert!(!format!("{}", NativeSkillMetadataError::InvalidName).contains("private"));
}
