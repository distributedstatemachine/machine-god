use super::*;
use machine_god_core::{
    Message, SessionId, SessionIncarnationId, ToolCall, ToolCallId, ToolName, ToolOutput,
};
use serde_json::json;

fn record(messages: Vec<Message>) -> Arc<SessionRecord> {
    let mut record = SessionRecord::empty(
        SessionId::new("clipboard").unwrap(),
        SessionIncarnationId::new("clipboard-life").unwrap(),
    );
    record.messages = messages;
    Arc::new(record)
}
fn select(
    record: Arc<SessionRecord>,
) -> Result<NativeClipboardReplyStep, NativeClipboardReplyError> {
    let mut selection = NativeClipboardReplySelection::new(record);
    for _ in 0..10_000 {
        match selection.next_step()? {
            NativeClipboardReplyStep::Progress => {}
            result => return Ok(result),
        }
    }
    panic!("bounded fixture failed to finish")
}
fn selected(record: Arc<SessionRecord>) -> Arc<str> {
    match select(record).unwrap() {
        NativeClipboardReplyStep::Selected(text) => text,
        other => panic!("expected text: {other:?}"),
    }
}
fn call() -> ContentBlock {
    ContentBlock::ToolCall {
        call: ToolCall {
            id: ToolCallId::new("same-id").unwrap(),
            name: ToolName::new("tool").unwrap(),
            arguments: json!({"text":"not a reply"}),
        },
    }
}

#[test]
fn latest_eligible_assistant_preserves_exact_multiple_block_bytes() {
    let record = record(vec![
        Message::text(Role::Assistant, "older"),
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "  **reply**\n\0\u{1b}[31m".into(),
                },
                ContentBlock::Json {
                    value: json!({"text":"ignored"}),
                },
                ContentBlock::Text {
                    text: "🙂\r\n\u{85}\u{202e} ".into(),
                },
                ContentBlock::ToolResult {
                    call_id: ToolCallId::new("same-id").unwrap(),
                    output: ToolOutput::success(json!("ignored result")),
                },
            ],
        },
        Message::text(Role::User, "later user"),
        Message::text(Role::System, "later system"),
        Message::text(Role::Tool, "later tool"),
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "not final".into(),
                },
                call(),
            ],
        },
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: String::new(),
                },
                ContentBlock::Json {
                    value: json!("ignored"),
                },
            ],
        },
    ]);
    assert_eq!(
        &*selected(record),
        "  **reply**\n\0\u{1b}[31m🙂\r\n\u{85}\u{202e} "
    );
}

#[test]
fn empty_and_whitespace_have_distinct_semantics_and_completion_is_fused() {
    let mut empty = NativeClipboardReplySelection::new(record(vec![
        Message::text(Role::User, "user"),
        Message::text(Role::Assistant, ""),
    ]));
    for _ in 0..3 {
        assert!(matches!(
            empty.next_step(),
            Ok(NativeClipboardReplyStep::Empty)
        ));
    }
    let mut selected =
        NativeClipboardReplySelection::new(record(vec![Message::text(Role::Assistant, " \n\t")]));
    let result = loop {
        if let NativeClipboardReplyStep::Selected(result) = selected.next_step().unwrap() {
            break result;
        }
    };
    let NativeClipboardReplyStep::Selected(again) = selected.next_step().unwrap() else {
        panic!("not fused")
    };
    assert!(Arc::ptr_eq(&result, &again));
    assert_eq!(&*result, " \n\t");
}

#[test]
fn constructor_and_each_metadata_scan_are_bounded_without_payload_allocation() {
    let snapshot = record(vec![Message::text(Role::User, "ignored"); 10_000]);
    let mut selection = NativeClipboardReplySelection::new(snapshot.clone());
    assert!(Arc::ptr_eq(&selection.record, &snapshot));
    assert_eq!(selection.message, 10_000);
    assert_eq!(selection.buffer.capacity(), 0);
    assert!(matches!(
        selection.next_step(),
        Ok(NativeClipboardReplyStep::Progress)
    ));
    assert_eq!(selection.message, 10_000 - SCAN_STEPS);
    assert_eq!(selection.buffer.capacity(), 0);

    let mut content = vec![
        ContentBlock::Text {
            text: "candidate".into()
        };
        300
    ];
    content.push(call());
    let mut selection = NativeClipboardReplySelection::new(record(vec![
        Message::text(Role::Assistant, "older"),
        Message {
            role: Role::Assistant,
            content,
        },
    ]));
    assert!(matches!(
        selection.next_step(),
        Ok(NativeClipboardReplyStep::Progress)
    ));
    assert_eq!(selection.buffer.capacity(), 0);
    assert_eq!(selection.block, SCAN_STEPS - 1);
    assert_eq!(&*selected(selection.record.clone()), "older");
}

#[test]
fn payload_copy_is_chunked_at_utf8_boundaries_without_separators() {
    let first = "x".repeat(COPY_BYTES - 1);
    let expected = format!("{first}🙂{}", "é".repeat(6000));
    let snapshot = record(vec![Message {
        role: Role::Assistant,
        content: vec![
            ContentBlock::Text { text: first },
            ContentBlock::Text {
                text: format!("🙂{}", "é".repeat(6000)),
            },
        ],
    }]);
    let mut selection = NativeClipboardReplySelection::new(snapshot);
    assert!(matches!(
        selection.next_step(),
        Ok(NativeClipboardReplyStep::Progress)
    ));
    assert_eq!(selection.buffer.len(), 0);
    for _ in 0..20 {
        let before = selection.buffer.len();
        match selection.next_step().unwrap() {
            NativeClipboardReplyStep::Progress => {
                assert!(selection.buffer.len() - before <= COPY_BYTES);
            }
            NativeClipboardReplyStep::Selected(result) => {
                assert_eq!(&*result, expected);
                return;
            }
            NativeClipboardReplyStep::Empty => panic!("lost reply"),
        }
    }
    panic!("copy did not finish")
}

#[test]
fn byte_cap_is_inclusive_and_oversized_latest_never_falls_back() {
    assert_eq!(
        selected(record(vec![Message::text(
            Role::Assistant,
            "x".repeat(MAX_FILE_SESSION_BYTES)
        )]))
        .len(),
        MAX_FILE_SESSION_BYTES
    );
    let mut selection = NativeClipboardReplySelection::new(record(vec![
        Message::text(Role::Assistant, "older"),
        Message::text(Role::Assistant, "x".repeat(MAX_FILE_SESSION_BYTES + 1)),
    ]));
    for _ in 0..3 {
        assert!(matches!(
            selection.next_step(),
            Err(NativeClipboardReplyError::ResourceLimit)
        ));
        assert_eq!(selection.buffer.capacity(), 0);
    }
    let content = vec![
        ContentBlock::Text {
            text: "x".repeat(MAX_FILE_SESSION_BYTES + 1),
        },
        call(),
    ];
    assert_eq!(
        &*selected(record(vec![
            Message::text(Role::Assistant, "older"),
            Message {
                role: Role::Assistant,
                content
            }
        ])),
        "older"
    );
}

#[test]
fn metadata_state_and_payload_json_never_guess_reply_kind() {
    for state in [
        "running",
        "failed",
        "cancelled",
        "interrupted",
        "completed",
        "unknown",
    ] {
        let mut snapshot = (*record(vec![Message::text(Role::Assistant, "saved reply")])).clone();
        snapshot.metadata.insert(
            "machine_god.conversation_history".into(),
            json!({"groups":[{"state":state}]}),
        );
        assert_eq!(&*selected(Arc::new(snapshot)), "saved reply");
    }
    assert!(matches!(
        select(record(vec![Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Json {
                value: json!({"reply":"do not parse me"})
            }]
        }])),
        Ok(NativeClipboardReplyStep::Empty)
    ));
}

#[test]
fn postaccept_changes_do_not_retarget_or_mutate_snapshot_and_debug_is_redacted() {
    let mut current = record(vec![Message::text(Role::Assistant, "private-original")]);
    let original = current.clone();
    let mut selection = NativeClipboardReplySelection::new(current.clone());
    Arc::make_mut(&mut current)
        .messages
        .push(Message::text(Role::Assistant, "later"));
    assert_eq!(&*selected(selection.record.clone()), "private-original");
    assert_eq!(&*selected(current), "later");
    assert_eq!(original.messages.len(), 1);
    assert!(!format!("{selection:?}").contains("private-original"));
    let _ = selection.next_step().unwrap();
    let result = selection.next_step().unwrap();
    assert!(!format!("{result:?}").contains("private-original"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod conversation;
