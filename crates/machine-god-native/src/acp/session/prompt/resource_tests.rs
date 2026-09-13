use super::*;
use serde_json::json;

#[test]
fn uri_only_resources_leave_canonical_text_unchanged() {
    let input = decode_prompt_input(&json!({"prompt":[
        {"type":"text","text":"Inspect this"},
        {"type":"resource","resource":{"uri":"file:///work/a%20b.rs"}},
        {"type":"text","text":"then explain"}
    ]}))
    .unwrap();
    assert_eq!(input.prompt.text, "Inspect this\nthen explain");
    assert_eq!(input.resource_targets, [PathBuf::from("/work/a b.rs")]);
    assert!(input.omissions.is_empty());
}

#[test]
fn embedded_text_preserves_order_even_for_omitted_uri() {
    let input = decode_prompt_input(&json!({"prompt":[
        {"type":"text","text":"before"},
        {"type":"resource","resource":{"uri":"https://host/content","text":"literal embedded bytes"}},
        {"type":"text","text":"after"}
    ]})).unwrap();
    assert_eq!(
        input.prompt.text,
        "before\nFile: https://host/content\nliteral embedded bytes\nafter"
    );
    assert!(input.resource_targets.is_empty());
    assert_eq!(input.omissions.len(), 1);
}

#[test]
fn unsafe_uri_forms_are_explicit_omissions() {
    for uri in [
        "https://host/a",
        "file://localhost/work/a",
        "file://user@host/a",
        "file:relative",
        "file:///work/../outside",
        "file:///work/%2e%2e/outside",
        "file:///a?query",
        "file:///a#fragment",
        "file:///a%00b",
        "file:///a%0Ab",
        "file:///a%C2%85b",
        "file:///a%",
        "file:///a%GG",
        "file:///a%FF",
        "file:///a%5cb",
        "file://host:80/a",
    ] {
        let input = decode_prompt_input(&json!({"prompt":[{"type":"text","text":"inspect"},{"type":"resource","resource":{"uri":uri}}]})).unwrap();
        assert!(input.resource_targets.is_empty(), "{uri}");
        assert_eq!(input.omissions.len(), 1, "{uri}");
        assert_eq!(
            input.omissions[0].reason,
            NativeAcpResourceOmissionReason::UnsafeTarget
        );
    }
}

#[test]
fn local_targets_are_normalized_and_deduplicated_without_io() {
    let input = decode_prompt_input(&json!({"prompt":[
        {"type":"text","text":"inspect"},
        {"type":"resource","resource":{"uri":"file:///missing/a/./b"}},
        {"type":"resource","resource":{"uri":"FILE:/missing/a/b"}},
        {"type":"resource","resource":{"uri":"file:///missing/a/%62"}}
    ]}))
    .unwrap();
    assert_eq!(input.resource_targets, [PathBuf::from("/missing/a/b")]);
}

#[test]
fn targets_and_omissions_have_independent_retained_bounds() {
    let mut blocks = vec![json!({"type":"text","text":"inspect"})];
    for index in 0..(MAX_ACP_RESOURCE_TARGETS + MAX_ACP_RESOURCE_OMISSIONS + 7) {
        blocks.push(json!({"type":"resource","resource":{"uri":format!("file:///work/{index}")}}));
    }
    let input = decode_prompt_input(&json!({"prompt":blocks})).unwrap();
    assert_eq!(input.resource_targets.len(), MAX_ACP_RESOURCE_TARGETS);
    assert_eq!(input.omissions.len(), MAX_ACP_RESOURCE_OMISSIONS);
    assert_eq!(input.omitted_records, 7);
}

#[test]
fn malformed_binary_empty_and_oversized_inputs_fail() {
    for blocks in [
        json!([]),
        json!([{"type":"resource","resource":{"uri":"file:///work/a"}}]),
        json!([{"type":"resource","resource":{"uri":"file:///work/a","text":null}}]),
        json!([{"type":"resource","resource":{"uri":"file:///work/a","blob":"abc"}}]),
        json!([{"type":"image","data":"abc"}]),
    ] {
        assert!(decode_prompt_input(&json!({"prompt":blocks})).is_err());
    }
    assert!(
        decode_prompt_input(
            &json!({"prompt":[{"type":"text","text":"x".repeat(MAX_ACP_PROMPT_BYTES + 1)}]})
        )
        .is_err()
    );
    assert!(
        decode_prompt_input(
            &json!({"prompt":vec![json!({"type":"text","text":"a"}); MAX_ACP_PROMPT_BLOCKS + 1]})
        )
        .is_err()
    );
}
