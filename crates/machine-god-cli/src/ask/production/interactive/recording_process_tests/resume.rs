//! Startup resume uses the same native host and owned PTY as recording scenarios.

use super::{Fixture, Gateway, Terminal, bounded_file, launch, sessions, tapes};
use machine_god_core::{
    ContentBlock, Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionStore,
    ToolCall, ToolCallId, ToolName, ToolOutput,
};
use machine_god_native::{
    FileSessionStore, NATIVE_CONVERSATION_HISTORY_KEY, NATIVE_MODEL_PREFERENCES_KEY,
    NATIVE_SESSION_METADATA_KEY, NativeConversationHistory, NativeHistoryFileAction,
    NativeHistoryFileEvidence, NativeHistoryFileSource, NativeHistoryFileStatus,
    NativeHistoryState, NativeModelPreferences, NativeReasoningEffort, NativeSessionMetadata,
    NativeSessionOrigin,
};
use serde_json::json;
use std::{
    fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::atomic::Ordering, time::Duration,
};

struct Saved {
    id: &'static str,
    timestamp: i64,
    model: &'static str,
    effort: &'static str,
    fast: bool,
}

const OLDER: Saved = Saved {
    id: "z-older-exact",
    timestamp: 100,
    model: "saved/exact",
    effort: "high",
    fast: true,
};
const LATEST: Saved = Saved {
    id: "a-newer-latest",
    timestamp: 200,
    model: "saved/latest",
    effort: "low",
    fast: false,
};

fn seed(fixture: &Fixture, saved: &Saved) {
    let root = fixture.state.join("machine-god");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let store = FileSessionStore::open(&root).unwrap();
    let mut record = SessionRecord::empty(
        SessionId::new(saved.id).unwrap(),
        SessionIncarnationId::new(format!("incarnation-{}", saved.id)).unwrap(),
    );
    record.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.into(),
        NativeSessionMetadata::new(
            &fixture.workspace,
            saved.timestamp,
            NativeSessionOrigin::Cli,
        )
        .unwrap()
        .to_value(),
    );
    record.metadata.insert(
        NATIVE_MODEL_PREFERENCES_KEY.into(),
        NativeModelPreferences::new(
            saved.model,
            NativeReasoningEffort::parse(saved.effort).unwrap(),
            saved.fast,
        )
        .unwrap()
        .to_value(),
    );
    let call_id = ToolCallId::new("historical-write").unwrap();
    let tool_name = ToolName::new("write_file").unwrap();
    let path = format!("{}.txt", saved.id);
    record.messages = vec![
        Message::text(Role::User, format!("saved question {}", saved.id)),
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                call: ToolCall {
                    id: call_id.clone(),
                    name: tool_name.clone(),
                    arguments: json!({"path":path,"content":"original historical content"}),
                },
            }],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: call_id.clone(),
                output: ToolOutput {
                    content: json!({"path":path,"written":true}),
                    is_error: false,
                },
            }],
        },
        Message::text(Role::Assistant, format!("saved answer {}", saved.id)),
    ];
    record.next_turn_sequence = 2;
    let mut history = NativeConversationHistory::default();
    history.begin(0, 1).unwrap();
    history
        .upsert_file(
            0,
            1,
            NativeHistoryFileEvidence::new(&path, NativeHistoryFileAction::Write, false)
                .unwrap()
                .with_execution(
                    NativeHistoryFileSource::new(1, 0, call_id, tool_name).unwrap(),
                    NativeHistoryFileStatus::Success,
                    None,
                    false,
                )
                .unwrap(),
        )
        .unwrap();
    history.finish(0, 1, NativeHistoryState::Completed).unwrap();
    record
        .metadata
        .insert(NATIVE_CONVERSATION_HISTORY_KEY.into(), history.to_value());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), store.save(record, None))
            .await
            .unwrap()
            .unwrap();
    });
    // A current external edit must survive replay of the saved successful write.
    fs::write(
        fixture.workspace.join(path),
        b"externally changed after historical write",
    )
    .unwrap();
}

fn snapshots(fixture: &Fixture) -> Vec<(PathBuf, Vec<u8>)> {
    let mut records: Vec<_> = sessions(fixture)
        .into_iter()
        .map(|path| {
            let bytes = bounded_file(&path);
            (path, bytes)
        })
        .collect();
    records.sort_by(|a, b| a.0.cmp(&b.0));
    records
}

fn resume(selection: &str, selected: &Saved, other: &Saved) {
    let fixture = Fixture::new();
    // Creation order and lexical ID order disagree with latest timestamp order.
    seed(&fixture, &LATEST);
    seed(&fixture, &OLDER);
    let before = snapshots(&fixture);
    assert_eq!(before.len(), 2);
    let gateway = Gateway::new();
    let mut command = launch(&fixture, &gateway, false);
    command
        .env("RECORDING_TEST_SELECTION", selection)
        .env("RECORDING_TEST_SESSION", selected.id);
    let mut terminal = Terminal::spawn(&mut command);
    terminal.wait_for(format!("saved answer {}", selected.id).as_bytes());
    terminal.wait_for(b"> ");
    terminal.send(b"/status\r");
    let preferences = format!(
        "model={} requested_effort={} requested_fast={}",
        selected.model, selected.effort, selected.fast
    );
    terminal.wait_for(preferences.as_bytes());
    terminal.send(b"/quit\r");
    // finish requires actual child reap, restored termios, and PTY EOF; an input
    // helper retaining the slave fails this same bounded cleanup assertion.
    let (status, output) = terminal.finish();
    assert_eq!(status.code(), Some(0));
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(&format!("[session] {}", selected.id)));
    assert!(output.contains(&format!("saved question {}", selected.id)));
    assert!(output.contains(&format!(
        "path: {}.txt\nstatus: recorded success",
        selected.id
    )));
    assert!(!output.contains("historical observations invalid"));
    assert!(!output.contains(&format!("saved answer {}", other.id)));
    assert!(!output.contains(&format!("saved question {}", other.id)));
    assert!(output.contains("session closed"));
    assert_eq!(gateway.inference.load(Ordering::Acquire), 0);
    assert_eq!(
        snapshots(&fixture),
        before,
        "no new session or persisted replay effects"
    );
    for saved in [&OLDER, &LATEST] {
        assert_eq!(
            bounded_file(&fixture.workspace.join(format!("{}.txt", saved.id))),
            b"externally changed after historical write"
        );
    }
    assert!(tapes(&fixture).is_empty());
    gateway.finish();
}

#[test]
fn latest_startup_restores_timestamp_selected_identity_preferences_and_history_without_effects() {
    resume("latest", &LATEST, &OLDER);
}

#[test]
fn exact_startup_restores_older_identity_preferences_and_history_without_effects() {
    resume("exact", &OLDER, &LATEST);
}
