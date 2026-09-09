use super::{CHUNK_BYTES, HistoryView, HistoryViewStep, SCAN_STEPS};
use machine_god_core::{
    ContentBlock, Message, Role, SessionId, SessionIncarnationId, SessionRecord, ToolCall,
    ToolCallId, ToolName, ToolOutput,
};
use serde_json::json;

#[path = "cards_tests.rs"]
mod cards_tests;

fn record(messages: Vec<Message>) -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("saved").unwrap(),
        SessionIncarnationId::new("incarnation").unwrap(),
    );
    record.messages = messages;
    record
}

fn chunks(view: &mut HistoryView) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    loop {
        match view.next_chunk() {
            HistoryViewStep::Chunk(chunk) => {
                assert!(!chunk.is_empty());
                assert!(chunk.len() <= CHUNK_BYTES);
                std::str::from_utf8(&chunk).unwrap();
                chunks.push(chunk);
            }
            HistoryViewStep::Progress => {}
            HistoryViewStep::Done => break,
        }
    }
    chunks
}

fn output(view: &mut HistoryView) -> String {
    String::from_utf8(chunks(view).concat()).unwrap()
}

fn call(name: &str, id: &str) -> ContentBlock {
    ContentBlock::ToolCall {
        call: ToolCall {
            id: ToolCallId::new(id).unwrap(),
            name: ToolName::new(name).unwrap(),
            arguments: json!({"secret_arguments":"never printed"}),
        },
    }
}

fn result(id: &str, is_error: bool, content: serde_json::Value) -> ContentBlock {
    ContentBlock::ToolResult {
        call_id: ToolCallId::new(id).unwrap(),
        output: ToolOutput { content, is_error },
    }
}

#[test]
fn canonical_user_and_assistant_text_is_complete_in_original_order() {
    let mut view = HistoryView::new(record(vec![
        Message::text(Role::System, "not a user-visible instruction"),
        Message::text(Role::User, "first question\nsecond line"),
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "first block".into(),
                },
                ContentBlock::Text {
                    text: "second block".into(),
                },
            ],
        },
        Message::text(Role::User, "next question"),
        Message::text(Role::Assistant, "# Saved Markdown\n```rust\nanswer()\n```"),
    ]));
    assert_eq!(
        output(&mut view),
        "\n[user]\nfirst question\nsecond line\n\n[assistant]\nfirst block\nsecond block\n\n[user]\nnext question\n\n[assistant]\n# Saved Markdown\n```rust\nanswer()\n```\n"
    );
    assert!(matches!(view.next_chunk(), HistoryViewStep::Done));
    assert!(matches!(view.next_chunk(), HistoryViewStep::Done));
}

#[test]
fn empty_history_is_fused_and_system_content_and_metadata_are_never_rendered() {
    let mut empty = HistoryView::new(record(Vec::new()));
    assert!(matches!(empty.next_chunk(), HistoryViewStep::Done));
    let mut source = record(vec![Message {
        role: Role::System,
        content: vec![
            call("private-tool", "private-id"),
            ContentBlock::Json {
                value: json!({"private":"system"}),
            },
        ],
    }]);
    source
        .metadata
        .insert("secret".into(), json!(["private metadata"]));
    let mut view = HistoryView::new(source);
    assert_eq!(output(&mut view), "");
}

#[test]
fn structured_variants_and_tool_text_are_explicitly_collapsed_without_raw_payloads() {
    let mut source = record(vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Json {
                value: json!({"image":"secret_image_bytes"}),
            }],
        },
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Json {
                    value: json!({"internal":"secret_json"}),
                },
                call("read_file", "a"),
            ],
        },
        Message {
            role: Role::Tool,
            content: vec![
                result("a", false, json!({"api_key":"secret_output"})),
                ContentBlock::Text {
                    text: "secret_tool_text".into(),
                },
                ContentBlock::Json {
                    value: json!(["secret_other"]),
                },
            ],
        },
    ]);
    source
        .metadata
        .insert("secret_metadata".into(), json!("do not display"));
    let mut view = HistoryView::new(source);
    let rendered = output(&mut view);
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("never printed"));
    assert_eq!(
        rendered
            .matches("[structured detail not displayed]")
            .count(),
        3
    );
    assert!(rendered.contains("[text detail not displayed]"));
    assert!(rendered.contains("[recorded tool call: read_file; id=a; completion not implied]"));
    assert!(
        rendered.contains("[recorded tool result: id=a; status=success; detail not displayed]")
    );
}

#[test]
fn unknown_result_requires_the_exact_core_placeholder_not_arbitrary_output_keys() {
    let unknown = json!({"code":"tool_result_unknown","message":"tool result status is unknown"});
    let mut view = HistoryView::new(record(vec![Message {
        role: Role::Tool,
        content: vec![
            result("unknown", true, unknown.clone()),
            result("success", false, unknown),
            result(
                "wrong-message",
                true,
                json!({"code":"tool_result_unknown","message":"different"}),
            ),
            result(
                "extra",
                true,
                json!({"code":"tool_result_unknown","message":"tool result status is unknown","other":true}),
            ),
            result(
                "partial",
                true,
                json!({"stdout":"private command","effects":true}),
            ),
        ],
    }]));
    let rendered = output(&mut view);
    assert_eq!(rendered.matches("status=unknown").count(), 1);
    assert_eq!(rendered.matches("status=success").count(), 1);
    assert_eq!(
        rendered
            .matches("status=error; effects may be partial")
            .count(),
        3
    );
    assert!(!rendered.contains("private command"));
}

#[test]
fn reused_and_unpaired_call_ids_never_acquire_a_guessed_global_association() {
    let mut view = HistoryView::new(record(vec![
        Message {
            role: Role::Assistant,
            content: vec![call("first", "same"), call("missing-result", "missing")],
        },
        Message {
            role: Role::Tool,
            content: vec![
                result("same", true, json!("error")),
                result("unpaired", false, json!("extra")),
            ],
        },
        Message {
            role: Role::Assistant,
            content: vec![call("second", "same")],
        },
        Message {
            role: Role::Tool,
            content: vec![result("same", false, json!("success"))],
        },
    ]));
    let rendered = output(&mut view);
    assert_eq!(rendered.matches("completion not implied").count(), 3);
    assert!(rendered.contains("id=unpaired; status=success"));
    assert_eq!(rendered.matches("id=same; status=").count(), 2);
    assert!(rendered.find("tool call: first").unwrap() < rendered.find("status=error").unwrap());
    assert!(
        rendered.find("tool call: second").unwrap() < rendered.rfind("status=success").unwrap()
    );
}

#[test]
fn large_text_stream_has_no_global_output_limit_or_prefix_truncation() {
    let text = "line α😀\n".repeat(100_000);
    let mut view = HistoryView::new(record(vec![Message::text(Role::Assistant, text.clone())]));
    let captured = chunks(&mut view);
    assert!(captured.len() > 256);
    assert_eq!(
        String::from_utf8(captured.concat()).unwrap(),
        format!("\n[assistant]\n{text}\n")
    );
}

#[test]
fn escapes_are_atomic_at_output_boundaries_and_all_control_text_is_preserved() {
    let prefix = "x".repeat(CHUNK_BYTES - "\n[user]\n".len() - 1);
    let text = format!("{prefix}\x1b\r\t\0\u{8}\u{c}\u{7f}\u{85}\u{061c}\u{202e}\u{2066}\nTAIL");
    let mut view = HistoryView::new(record(vec![Message::text(Role::User, text)]));
    let captured = chunks(&mut view);
    assert_eq!(captured[0].len(), CHUNK_BYTES - 1);
    assert!(captured[0].ends_with(b"x"));
    assert!(captured[1].starts_with(b"\\u001b"));
    let rendered = String::from_utf8(captured.concat()).unwrap();
    assert_eq!(
        rendered,
        format!(
            "\n[user]\n{prefix}\\u001b\\r\\t\\u0000\\b\\f\\u007f\\u0085\\u061c\\u202e\\u2066\nTAIL\n"
        )
    );
    assert!(!rendered.contains(['\x1b', '\r', '\u{202e}']));
}

#[test]
fn multibyte_scalars_at_chunk_edges_are_complete_and_not_lost() {
    for spare in 0..4 {
        let prefix = "x".repeat(CHUNK_BYTES - "\n[user]\n".len() - spare);
        let text = format!("{prefix}😀界e\u{301}END");
        let mut view = HistoryView::new(record(vec![Message::text(Role::User, text.clone())]));
        let captured = chunks(&mut view);
        assert_eq!(
            String::from_utf8(captured.concat()).unwrap(),
            format!("\n[user]\n{text}\n")
        );
    }
}

#[test]
fn identity_piece_defensively_escapes_line_breaks_without_injecting_headers() {
    let mut output = Vec::new();
    let text = "tool\n[user]\x1b";
    let consumed = super::append_piece(super::Piece::identity(text), 0, &mut output);
    assert_eq!(consumed, text.len());
    assert_eq!(output, b"tool\\n[user]\\u001b");
}

#[test]
fn maximal_valid_identifiers_are_fully_chunked_without_formatted_history_copies() {
    let name = "n".repeat(128);
    let id = "i".repeat(128);
    let mut view = HistoryView::new(record(vec![Message {
        role: Role::Assistant,
        content: (0..64).map(|_| call(&name, &id)).collect(),
    }]));
    let captured = chunks(&mut view);
    assert!(captured.len() > 4);
    let expected =
        format!("[recorded tool call: {name}; id={id}; completion not implied]\n").repeat(64);
    assert_eq!(
        String::from_utf8(captured.concat()).unwrap(),
        format!("\n[assistant]\n{expected}")
    );
}

#[test]
fn skipped_history_has_a_per_call_scan_budget_and_no_false_done() {
    let mut messages = vec![Message::text(Role::System, "hidden"); SCAN_STEPS * 2 + 1];
    messages.push(Message::text(Role::User, "visible later"));
    let mut view = HistoryView::new(record(messages));
    assert_eq!(view.message, 0);
    assert!(matches!(view.next_chunk(), HistoryViewStep::Progress));
    assert_eq!(view.message, SCAN_STEPS);
    assert!(matches!(view.next_chunk(), HistoryViewStep::Progress));
    assert_eq!(view.message, SCAN_STEPS * 2);
    assert_eq!(output(&mut view), "\n[user]\nvisible later\n");
}

#[test]
fn snapshot_is_moved_not_cloned_or_mutated_and_debug_is_redacted() {
    let mut source = record(vec![Message::text(Role::User, "private transcript")]);
    source
        .metadata
        .insert("private metadata".into(), json!({"nested":[1,2,3]}));
    let expected = source.clone();
    let messages = source.messages.as_ptr();
    let mut view = HistoryView::new(source);
    assert_eq!(view.record.messages.as_ptr(), messages);
    assert_eq!(format!("{view:?}"), "HistoryView { .. }");
    let first = view.next_chunk();
    assert_eq!(format!("{first:?}"), "HistoryViewStep::Chunk(..)");
    assert_eq!(view.record, expected);
    assert!(matches!(view.next_chunk(), HistoryViewStep::Done));
}
