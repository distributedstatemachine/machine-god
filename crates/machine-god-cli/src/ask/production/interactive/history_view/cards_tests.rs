use super::*;
use machine_god_native::{
    NATIVE_CONVERSATION_HISTORY_KEY, NativeConversationHistory, NativeHistoryBackground,
    NativeHistoryFileAction, NativeHistoryFileEvidence, NativeHistoryFileSource,
    NativeHistoryFileStatus,
};

fn typed_call(name: &str, arguments: serde_json::Value) -> ContentBlock {
    ContentBlock::ToolCall {
        call: ToolCall {
            id: ToolCallId::new("same").unwrap(),
            name: ToolName::new(name).unwrap(),
            arguments,
        },
    }
}

#[test]
fn command_cards_use_native_schema_and_preserve_requested_not_invented_cwd() {
    let mut view = HistoryView::new(record(vec![Message {
        role: Role::Assistant,
        content: vec![
            typed_call(
                "terminal",
                json!({"action":"exec","command":"printf '\u{1b}'\nnext","cwd":"../requested"}),
            ),
            typed_call("terminal", json!({"action":"exec","command":"without cwd"})),
            typed_call(
                "terminal",
                json!({"action":"exec","command":"malformed secret","extra":true}),
            ),
        ],
    }]));
    let rendered = output(&mut view);
    assert!(rendered.contains("command: printf '\\u001b'\\nnext\nrequested cwd: ../requested"));
    assert!(rendered.contains("[default; resolved cwd not recorded]"));
    assert_eq!(rendered.matches("[recorded command request;").count(), 2);
    assert!(!rendered.contains("malformed secret"));
    assert!(!rendered.contains("__historical_unresolved__"));
}

fn observed_record() -> SessionRecord {
    let mut source = record(vec![
        Message::text(Role::User, "saved question"),
        Message {
            role: Role::Assistant,
            content: vec![typed_call(
                "edit_file",
                json!({"path":"file.rs","old_string":"old\ntext","new_string":"new\u{1b}text"}),
            )],
        },
    ]);
    source.next_turn_sequence = 2;
    let mut history = NativeConversationHistory::default();
    history.begin(0, 1).unwrap();
    history
        .upsert_file(
            0,
            1,
            NativeHistoryFileEvidence::new("file.rs", NativeHistoryFileAction::Edit, false)
                .unwrap()
                .with_execution(
                    NativeHistoryFileSource::new(
                        1,
                        0,
                        ToolCallId::new("same").unwrap(),
                        ToolName::new("edit_file").unwrap(),
                    )
                    .unwrap(),
                    NativeHistoryFileStatus::Unknown,
                    None,
                    false,
                )
                .unwrap(),
        )
        .unwrap();
    history
        .set_background(
            0,
            1,
            Some(
                NativeHistoryBackground::new(
                    "/saved/log\npath",
                    Some("https://saved.invalid"),
                    true,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    source
        .metadata
        .insert(NATIVE_CONVERSATION_HISTORY_KEY.into(), history.to_value());
    source
}

#[test]
fn typed_files_replacements_and_background_are_historical_not_effect_claims() {
    let mut view = HistoryView::new(observed_record());
    let rendered = output(&mut view);
    assert!(
        rendered.contains("action: edit\npath: file.rs\nstatus: unknown; effects not inferred")
    );
    assert!(rendered.contains(
        "[recorded requested replacement; not a full-file diff]\n- old\\ntext\n+ new\\u001btext"
    ));
    assert!(rendered.contains("[recorded background observation; current liveness unknown]"));
    assert!(rendered.contains("log: /saved/log\\npath\nurl: https://saved.invalid"));
    assert!(
        rendered.find("recorded tool call").unwrap()
            < rendered.find("recorded file observation").unwrap()
    );
    assert!(matches!(view.next_chunk(), HistoryViewStep::Done));
}

#[test]
fn invalid_exact_source_suppresses_all_observations_without_raw_metadata() {
    let mut source = observed_record();
    source.messages[1].content[0] = typed_call("read_file", json!({"path":"different"}));
    let rendered = output(&mut HistoryView::new(source));
    assert!(rendered.contains("[historical observations invalid; details not displayed]"));
    assert!(!rendered.contains("saved.invalid"));
    assert!(!rendered.contains("old\\ntext"));
}

#[test]
fn large_requested_fragment_streams_across_chunks_and_keeps_following_background() {
    let mut source = observed_record();
    let ContentBlock::ToolCall { call } = &mut source.messages[1].content[0] else {
        panic!()
    };
    let text = "界\u{1b}\n".repeat(10_000);
    call.arguments["new_string"] = json!(text);
    let captured = chunks(&mut HistoryView::new(source));
    assert!(captured.len() > 20);
    let rendered = String::from_utf8(captured.concat()).unwrap();
    assert!(rendered.contains(&"界\\u001b\\n".repeat(10_000)));
    assert!(rendered.ends_with("url: https://saved.invalid\n"));
}
