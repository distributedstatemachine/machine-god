use super::*;
use serde_json::json;

#[test]
fn text_and_embedded_resources_preserve_order_without_uri_authority() {
    let prompt = decode_prompt_input(&json!({"prompt":[
        {"type":"text","text":"Read this"},
        {"type":"resource","resource":{"uri":"https://example.test/secret","text":"supplied body"}},
        {"type":"text","text":"Then explain"}
    ]}))
    .unwrap();
    assert_eq!(
        prompt.prompt().text,
        "Read this\nFile: https://example.test/secret\nsupplied body\nThen explain"
    );
    assert!(prompt.prompt().options.metadata.is_empty());
}

#[test]
fn malformed_or_unsupported_content_is_not_silently_omitted() {
    for input in [
        json!({}),
        json!({"prompt":null}),
        json!({"prompt":[]}),
        json!({"prompt":[{"type":"text"}]}),
        json!({"prompt":[{"type":"resource","resource":{"uri":"file:///private/x"}}]}),
        json!({"prompt":[{"type":"text","text":"valid"},{"type":"image","data":"secret"}]}),
        json!({"prompt":[{"type":"resource","resource":{"uri":"file:///x\nspoof","text":"x"}}]}),
    ] {
        assert!(decode_prompt_input(&input).is_err());
    }
}

#[test]
fn prompt_bounds_include_joined_resource_labels_and_separators() {
    let full = "x".repeat(MAX_ACP_PROMPT_BYTES);
    assert!(decode_prompt_input(&json!({"prompt":[{"type":"text","text":full}]})).is_ok());
    assert!(matches!(
        decode_prompt_input(
            &json!({"prompt":[{"type":"text","text":full},{"type":"text","text":""}]})
        ),
        Err(AcpPromptError::Limit)
    ));
    assert!(matches!(
        decode_prompt_input(
            &json!({"prompt":[{"type":"resource","resource":{"uri":"file:///x","text":full}}]})
        ),
        Err(AcpPromptError::Limit)
    ));
    assert!(matches!(
        decode_prompt_input(
            &json!({"prompt":vec![json!({"type":"text","text":"x"});MAX_ACP_PROMPT_BLOCKS+1]})
        ),
        Err(AcpPromptError::Limit)
    ));
}

#[test]
fn decoding_errors_are_typed_and_data_free() {
    let errors = [
        (json!({}), AcpPromptError::InvalidPrompt),
        (
            json!({"prompt":[{"type":"image","data":"private"}]}),
            AcpPromptError::UnsupportedContent,
        ),
        (
            json!({"prompt":[{"type":"text","text":"private".repeat(MAX_ACP_PROMPT_BYTES)}]}),
            AcpPromptError::Limit,
        ),
    ];
    for (input, expected) in errors {
        let error = decode_prompt_input(&input).unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("private"));
        assert!(!format!("{error:?}").contains("private"));
        assert!(std::error::Error::source(&error).is_none());
    }
}
