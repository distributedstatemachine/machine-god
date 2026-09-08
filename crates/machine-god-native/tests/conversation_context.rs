use std::collections::BTreeMap;

use machine_god_core::{
    ContentBlock, Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionRevision,
    ToolCall, ToolCallId, ToolName, ToolOutput,
};
use machine_god_native::{
    MAX_FILE_SESSION_BYTES, NATIVE_CONTEXT_PREFERENCES_KEY, NATIVE_CONVERSATION_HISTORY_KEY,
    NativeContextError, NativeContextPreferences, NativeConversationHistory,
    NativeHistoryBackground, NativeHistoryFileAction, NativeHistoryFileEvidence,
    NativeHistoryFileSource, NativeHistoryFileStatus, NativeHistoryState,
};
use serde_json::{Value, json};

fn record(groups: &[(&str, &str)]) -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("context").unwrap(),
        SessionIncarnationId::new("incarnation").unwrap(),
    );
    record.revision = SessionRevision(1);
    record.next_turn_sequence = 100;
    for (user, assistant) in groups {
        record.messages.push(Message::text(Role::User, *user));
        record
            .messages
            .push(Message::text(Role::Assistant, *assistant));
    }
    record
}

fn preferences(first: usize, maximum: usize) -> NativeContextPreferences {
    NativeContextPreferences::from_metadata(&BTreeMap::from([(
        NATIVE_CONTEXT_PREFERENCES_KEY.to_owned(),
        json!({"schema_version":1,"first_retained_message":first,"max_history_turns":maximum}),
    )]))
    .unwrap()
}

fn summary(preferences: &NativeContextPreferences, record: &SessionRecord) -> String {
    preferences
        .projection(record)
        .unwrap()
        .unwrap()
        .prefix_summary
        .unwrap()
}

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new(name).unwrap(),
        arguments: json!({}),
    }
}

fn call_message(call: ToolCall) -> Message {
    Message {
        role: Role::Assistant,
        content: vec![ContentBlock::ToolCall { call }],
    }
}

fn result(id: &str, output: ToolOutput) -> Message {
    Message {
        role: Role::Tool,
        content: vec![ContentBlock::ToolResult {
            call_id: ToolCallId::new(id).unwrap(),
            output,
        }],
    }
}

#[test]
fn repeated_call_facts_consume_summary_quota_without_collapsing_saved_observations() {
    let mut record = record(&[]);
    record
        .messages
        .push(Message::text(Role::User, "read twice"));
    let mut history = NativeConversationHistory::default();
    history.begin(0, 1).unwrap();
    for index in [1, 3] {
        record.messages.extend([
            call_message(call("reused", "read_file")),
            result("reused", ToolOutput::success(json!("same contents"))),
        ]);
        let fact = NativeHistoryFileEvidence::new("file.rs", NativeHistoryFileAction::Read, false)
            .unwrap()
            .with_execution(
                NativeHistoryFileSource::new(
                    index,
                    0,
                    ToolCallId::new("reused").unwrap(),
                    ToolName::new("read_file").unwrap(),
                )
                .unwrap(),
                NativeHistoryFileStatus::Success,
                None,
                true,
            )
            .unwrap();
        history.upsert_file(0, 1, fact).unwrap();
    }
    history
        .upsert_file(
            0,
            1,
            NativeHistoryFileEvidence::new("over-quota.rs", NativeHistoryFileAction::Read, false)
                .unwrap(),
        )
        .unwrap();
    history.finish(0, 1, NativeHistoryState::Completed).unwrap();
    record.messages.extend([
        Message::text(Role::User, "keep"),
        Message::text(Role::Assistant, "kept"),
    ]);
    record.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
        history.to_value(),
    );
    let original = record.clone();
    let text = summary(&preferences(5, 0), &record);
    assert_eq!(text.matches("- file read: file.rs").count(), 1);
    assert!(text.contains("2 additional line(s) omitted"), "{text}");
    assert!(!text.contains("over-quota.rs"));
    assert_eq!(
        NativeConversationHistory::from_record(&record)
            .unwrap()
            .group(0)
            .unwrap()
            .files()
            .len(),
        3
    );
    assert_eq!(record, original);
    record
        .metadata
        .get_mut(NATIVE_CONVERSATION_HISTORY_KEY)
        .unwrap()["groups"][0]["files"][0]["source"]["content_block"] = json!(1);
    assert!(matches!(
        preferences(5, 0).projection(&record),
        Err(NativeContextError::InvalidHistory)
    ));
}

#[test]
fn typed_facts_supply_ordered_file_background_and_interruption_summary_sections() {
    let mut record = record(&[
        ("first", "one"),
        ("second", "two"),
        ("third", "three"),
        ("keep", "four"),
    ]);
    let mut history = NativeConversationHistory::default();
    for (first, sequence, state) in [
        (0, 1, NativeHistoryState::Cancelled),
        (2, 2, NativeHistoryState::Failed),
        (4, 3, NativeHistoryState::Interrupted),
    ] {
        history.begin(first, sequence).unwrap();
        history.finish(first, sequence, state).unwrap();
    }
    for path in ["z.rs", "a.rs"] {
        history
            .upsert_file(
                0,
                1,
                NativeHistoryFileEvidence::new(path, NativeHistoryFileAction::Edit, true).unwrap(),
            )
            .unwrap();
    }
    history
        .set_background(
            0,
            1,
            Some(NativeHistoryBackground::new("run.log", None, true).unwrap()),
        )
        .unwrap();
    record.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
        history.to_value(),
    );
    let original = record.clone();
    let text = summary(&preferences(6, 0), &record);
    assert!(
        text.contains(
            "- Tool execution evidence:\n- file edit: z.rs, stale\n- file edit: a.rs, stale"
        ),
        "{text}"
    );
    assert!(
        text.contains("- Background activity:\n- log=run.log, local server started (URL pending)"),
        "{text}"
    );
    assert!(text.contains("- Incomplete turns:\n- cancelled\n- failed\n- interrupted (terminal reason not recorded)"), "{text}");
    assert_eq!(record, original);
}

#[test]
fn full_tool_evidence_quota_does_not_suppress_typed_background_or_interruption() {
    let mut record = record(&[("first", "one"), ("keep", "two")]);
    let mut round = Vec::new();
    for id in ["a", "b", "c", "d"] {
        round.push(call_message(call(id, "tool")));
        round.push(result(id, ToolOutput::success(json!(null))));
    }
    record.messages.splice(2..2, round);
    let mut history = NativeConversationHistory::default();
    history.begin(0, 1).unwrap();
    history.finish(0, 1, NativeHistoryState::Failed).unwrap();
    history
        .set_background(
            0,
            1,
            Some(
                NativeHistoryBackground::new("out.log", Some("http://localhost:8080"), true)
                    .unwrap(),
            ),
        )
        .unwrap();
    record.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
        history.to_value(),
    );
    let text = summary(&preferences(10, 0), &record);
    assert_eq!(text.matches("stored bytes").count(), 1); // Four quota candidates deduplicate at final compression.
    assert!(text.contains("- Background activity:"), "{text}");
    assert!(text.contains("- Incomplete turns:\n- failed"), "{text}");
}

#[test]
fn typed_locator_controls_cannot_create_unbounded_summary_lines() {
    let mut record = record(&[("first", "one"), ("keep", "two")]);
    let mut history = NativeConversationHistory::default();
    history.begin(0, 1).unwrap();
    history.finish(0, 1, NativeHistoryState::Completed).unwrap();
    history
        .upsert_file(
            0,
            1,
            NativeHistoryFileEvidence::new("a\n\rb\\c", NativeHistoryFileAction::Read, false)
                .unwrap(),
        )
        .unwrap();
    history
        .set_background(
            0,
            1,
            Some(NativeHistoryBackground::new("log\nname", None, false).unwrap()),
        )
        .unwrap();
    record.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
        history.to_value(),
    );
    let text = summary(&preferences(2, 0), &record);
    assert!(text.contains("a\\n\\rb\\\\c"), "{text}");
    assert!(text.contains("log\\nname"), "{text}");
    assert!(text.lines().count() <= 24);
    assert!(text.len() <= 1200);
}

#[test]
fn default_and_strict_scalar_metadata_roundtrip() {
    let default = NativeContextPreferences::default();
    assert_eq!(default.first_retained_message(), 0);
    assert_eq!(default.max_history_turns(), 0);
    assert_eq!(
        NativeContextPreferences::from_metadata(&BTreeMap::new()).unwrap(),
        default
    );
    assert_eq!(
        default.to_value(),
        json!({"schema_version":1,"first_retained_message":0,"max_history_turns":0})
    );
    let mut prefs = preferences(2, 4);
    assert_eq!(prefs.first_retained_message(), 2);
    assert_eq!(prefs.max_history_turns(), 4);
    prefs.set_max_history_turns(MAX_FILE_SESSION_BYTES).unwrap();
    let original = prefs.clone();
    assert_eq!(
        prefs.set_max_history_turns(MAX_FILE_SESSION_BYTES + 1),
        Err(NativeContextError::PreferenceLimit)
    );
    assert_eq!(prefs, original);
    assert_eq!(format!("{prefs:?}"), "NativeContextPreferences { .. }");
}

#[test]
fn malformed_versions_keys_shapes_and_scalars_are_rejected_shallowly() {
    for value in [
        Value::Null,
        json!([]),
        json!({}),
        json!({"schema_version":1,"first_retained_message":0,"max_history_turns":0,"extra":true}),
        json!({"schema_version":1,"first_retained_message":-1,"max_history_turns":0}),
        json!({"schema_version":1,"first_retained_message":0.0,"max_history_turns":0}),
        json!({"schema_version":1,"first_retained_message":0,"max_history_turns":"private"}),
        json!({"schema_version":1,"first_retained_message":0,"max_history_turns":usize::MAX}),
        json!({"schema_version":2,"first_retained_message":0,"max_history_turns":0}),
    ] {
        let metadata = BTreeMap::from([(NATIVE_CONTEXT_PREFERENCES_KEY.to_owned(), value)]);
        let error = NativeContextPreferences::from_metadata(&metadata).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private"));
    }
}

#[test]
fn manual_compaction_has_an_exact_local_summary_and_preserves_canonical_history() {
    let original = record(&[
        ("first request", "first outcome"),
        ("second request", "second outcome"),
        ("kept request", "kept outcome"),
    ]);
    let untouched = original.clone();
    let mut prefs = NativeContextPreferences::default();
    assert!(prefs.projection(&original).unwrap().is_none());
    assert!(prefs.force_compact(&original).unwrap());
    assert_eq!(prefs.first_retained_message(), 4);
    assert_eq!(
        summary(&prefs, &original),
        "Conversation summary:\n- Earlier turns compacted: 2\n- Recent user requests:\n- first request\n- second request\n- Assistant outcomes:\n- first outcome\n- second outcome"
    );
    assert!(!prefs.force_compact(&original).unwrap());
    assert_eq!(original, untouched);
    assert_eq!(
        NativeContextPreferences::from_metadata(&BTreeMap::from([(
            NATIVE_CONTEXT_PREFERENCES_KEY.to_owned(),
            prefs.to_value()
        )]))
        .unwrap(),
        prefs
    );
}

#[test]
fn empty_and_single_group_compaction_are_noops() {
    for record in [record(&[]), record(&[("only", "answer")])] {
        let mut prefs = NativeContextPreferences::default();
        assert!(!prefs.force_compact(&record).unwrap());
        assert!(prefs.projection(&record).unwrap().is_none());
    }
}

#[test]
fn automatic_limits_follow_pinned_recent_group_selection() {
    let record = record(&[
        ("1", "a"),
        ("2", "b"),
        ("3", "c"),
        ("4", "d"),
        ("5", "e"),
        ("6", "f"),
        ("7", "g"),
    ]);
    for (maximum, first, has_summary) in [
        (0, 0, false),
        (1, 12, false),
        (2, 12, true),
        (3, 10, true),
        (4, 8, true),
        (5, 6, true),
        (6, 6, true),
        (7, 0, false),
        (4097, 0, false),
    ] {
        let prefs = preferences(0, maximum);
        match prefs.projection(&record).unwrap() {
            Some(projection) => {
                assert_eq!(projection.first_retained_message, first);
                assert_eq!(projection.prefix_summary.is_some(), has_summary);
            }
            None => assert_eq!(first, 0),
        }
    }
}

#[test]
fn manual_prefix_is_merged_before_automatic_compaction_and_deduplicated() {
    let record = record(&[
        ("one", "first"),
        ("two", "second"),
        ("three", "third"),
        ("four", "fourth"),
        ("five", "fifth"),
    ]);
    let prefs = preferences(2, 3);
    let projection = prefs.projection(&record).unwrap().unwrap();
    assert_eq!(projection.first_retained_message, 6);
    assert_eq!(
        projection.prefix_summary.unwrap(),
        "Conversation summary:\n- Earlier turns compacted: 2\n- Previously compacted context:\n- Earlier turns compacted: 1\n- Recent user requests:\n- one\n- Assistant outcomes:\n- first\n- two\n- three\n- second\n- third\n- ... 3 additional line(s) omitted."
    );
    let max_one = preferences(2, 1).projection(&record).unwrap().unwrap();
    assert_eq!(max_one.first_retained_message, 8);
    assert!(max_one.prefix_summary.is_none());
}

#[test]
fn repeated_manual_compaction_rebuilds_from_canonical_prefix() {
    let mut record = record(&[("first", "one"), ("second", "two")]);
    let mut prefs = NativeContextPreferences::default();
    prefs.force_compact(&record).unwrap();
    assert!(summary(&prefs, &record).contains("- Earlier turns compacted: 1"));
    record.messages.extend([
        Message::text(Role::User, "third"),
        Message::text(Role::Assistant, "three"),
    ]);
    prefs.force_compact(&record).unwrap();
    let summary = summary(&prefs, &record);
    assert!(summary.contains("- Earlier turns compacted: 2"));
    assert!(!summary.contains("Previously compacted"));
    assert_eq!(record.messages.len(), 6);
}

#[test]
fn whitespace_unicode_and_duplicate_quotas_match_pinned_byte_rules() {
    let record = record(&[
        (" \talpha\r\n beta ", " answer\n here "),
        ("alpha beta", "answer here"),
        ("alpha beta", "answer here"),
        ("alpha beta", "ignored fourth assistant"),
        ("later unique user", "ignored"),
        ("kept", "kept"),
    ]);
    let text = summary(&preferences(10, 0), &record);
    assert_eq!(
        text,
        "Conversation summary:\n- Earlier turns compacted: 5\n- Recent user requests:\n- alpha beta\n- Assistant outcomes:\n- answer here\n- ... 5 additional line(s) omitted."
    );
    let long = format!("{}★tail", "a".repeat(155));
    let unicode = record_with_two(&long, "é".repeat(79).as_str());
    let text = summary(&preferences(2, 0), &unicode);
    assert!(text.contains(&format!("- {}\n", "a".repeat(155))));
    assert!(!text.contains('★'));
    assert!(text.ends_with(&"é".repeat(78)));
    assert!(std::str::from_utf8(text.as_bytes()).is_ok());
    let uncommon = record_with_two("a\u{a0}b\u{b}c", "outcome");
    assert!(summary(&preferences(2, 0), &uncommon).contains("a\u{a0}b\u{b}c"));
}

fn record_with_two(user: &str, assistant: &str) -> SessionRecord {
    record(&[(user, assistant), ("kept", "kept")])
}

#[test]
fn summary_byte_and_line_limits_include_merged_prefix_and_omission_notice() {
    let entries: Vec<_> = (0..12)
        .map(|index| {
            (
                format!("user{index} {}", "🙂".repeat(50)),
                format!("assistant{index} {}", "漢".repeat(60)),
            )
        })
        .collect();
    let borrowed: Vec<_> = entries
        .iter()
        .map(|(user, assistant)| (user.as_str(), assistant.as_str()))
        .collect();
    let record = record(&borrowed);
    for prefs in [preferences(20, 0), preferences(8, 3)] {
        let text = summary(&prefs, &record);
        assert!(text.len() <= 1200);
        assert!(text.lines().count() <= 24);
        assert!(text.starts_with("Conversation summary:"));
        assert!(std::str::from_utf8(text.as_bytes()).is_ok());
    }
}

#[test]
fn user_groups_include_all_tool_rounds_and_no_input_continuations() {
    let mut record = record(&[("older", "old assistant"), ("latest", "first response")]);
    record.messages.extend([
        call_message(call("same", "action")),
        result("same", ToolOutput::success(json!(true))),
        Message::text(Role::Assistant, "continued latest outcome"),
    ]);
    let original = record.clone();
    let mut prefs = NativeContextPreferences::default();
    assert!(prefs.force_compact(&record).unwrap());
    assert_eq!(prefs.first_retained_message(), 2);
    assert_eq!(record.messages[2..].len(), 5);
    record.messages.extend([
        Message::text(Role::User, "newest"),
        Message::text(Role::Assistant, "answer"),
    ]);
    prefs.force_compact(&record).unwrap();
    let text = summary(&prefs, &record);
    assert!(text.contains("continued latest outcome"));
    assert!(!text.contains("first response"));
    assert_eq!(
        record.messages[..original.messages.len()],
        original.messages
    );
}

#[test]
fn tool_evidence_correlates_calls_preserves_unknowns_and_limits_in_order() {
    let mut record = record(&[("old", "before tools")]);
    let known = ToolOutput::success(json!({"confirmed": true}));
    let unknown = ToolOutput {
        is_error: true,
        content: json!({"code":"tool_result_unknown","message":"tool result status is unknown"}),
    };
    let failure = ToolOutput {
        is_error: true,
        content: json!({"code":"tool_failed"}),
    };
    for (id, name, output) in [
        ("reused", "read_file", known.clone()),
        ("unknown", "terminal", unknown.clone()),
        ("reused", "write_file", failure.clone()),
        ("four", "action", known.clone()),
        ("five", "fifth_tool", known.clone()),
    ] {
        record
            .messages
            .extend([call_message(call(id, name)), result(id, output)]);
    }
    let cut = record.messages.len();
    record.messages.extend([
        Message::text(Role::User, "kept"),
        Message::text(Role::Assistant, "kept answer"),
    ]);
    let original = record.clone();
    let text = summary(&preferences(cut, 0), &record);
    assert!(text.contains(&format!(
        "- read_file success ({} stored bytes)",
        serde_json::to_vec(&known).unwrap().len()
    )));
    assert!(text.contains(&format!(
        "- terminal unknown ({} stored bytes)",
        serde_json::to_vec(&unknown).unwrap().len()
    )));
    assert!(text.contains("- write_file failure"));
    assert!(text.contains("- action success"));
    assert!(!text.contains("fifth_tool"));
    assert!(!text.contains("Background activity"));
    assert!(!text.contains("Incomplete turns"));
    assert_eq!(record, original);
}

#[test]
fn native_archive_receipt_is_bounded_correlated_advisory_evidence_only() {
    let mut record = record(&[("old", "answer")]);
    let handle = format!("tool-archive-v1-{}-{}", "a".repeat(64), "b".repeat(64));
    let archive = json!({"type":"tool_result_archive", "archive": {"handle":handle,"source_total_bytes":100_000,"source_context":{"session_id":"context","session_incarnation_id":"incarnation","turn_id":"turn-1","call_id":"archived"}}, "preview":"\t retained\npreview \r"});
    record.messages.extend([
        call_message(call("archived", "terminal")),
        result("archived", ToolOutput::success(archive.clone())),
        Message::text(Role::User, "kept"),
    ]);
    let text = summary(&preferences(4, 0), &record);
    assert!(text.contains(&format!(
        "- terminal success (100000 stored bytes, handle={handle}, preview=retained preview)"
    )));
    let ContentBlock::ToolResult { output, .. } = &mut record.messages[3].content[0] else {
        unreachable!()
    };
    output.content["archive"]["source_context"]["session_id"] = json!("other");
    let text = summary(&preferences(4, 0), &record);
    assert!(!text.contains("handle="));
    assert!(text.contains("- terminal success"));
    assert_eq!(archive["archive"]["source_total_bytes"], json!(100_000));
}

#[test]
fn leading_systems_are_preserved_and_later_systems_cannot_be_dropped() {
    let mut record = record(&[("first", "one"), ("second", "two")]);
    record
        .messages
        .insert(0, Message::text(Role::System, "system authority"));
    record
        .messages
        .insert(0, Message::text(Role::System, "first authority"));
    let mut prefs = NativeContextPreferences::default();
    prefs.force_compact(&record).unwrap();
    assert_eq!(prefs.first_retained_message(), 4);
    assert!(!summary(&prefs, &record).contains("authority"));
    record
        .messages
        .insert(4, Message::text(Role::System, "must not drop"));
    assert_eq!(
        prefs.force_compact(&record),
        Err(NativeContextError::InvalidCursor)
    );
    assert!(preferences(0, 1).projection(&record).is_err());
}

#[test]
fn stale_cursors_and_malformed_tool_units_are_rejected_without_mutation() {
    let original = record(&[("old", "outcome"), ("kept", "outcome")]);
    for first in [1, 3, 4, 100] {
        assert!(preferences(first, 0).projection(&original).is_err());
    }
    let mut cases = Vec::new();
    let mut missing = original.clone();
    missing
        .messages
        .insert(1, call_message(call("missing", "tool")));
    cases.push(missing);
    let mut orphan = original.clone();
    orphan
        .messages
        .insert(1, result("orphan", ToolOutput::success(json!(null))));
    cases.push(orphan);
    let mut duplicate = original.clone();
    let call = call("duplicate", "tool");
    duplicate.messages.insert(
        1,
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::ToolCall { call: call.clone() },
                ContentBlock::ToolCall { call },
            ],
        },
    );
    cases.push(duplicate);
    let mut no_user = original.clone();
    no_user.messages[0].role = Role::Assistant;
    cases.push(no_user);
    for record in cases {
        let mut prefs = NativeContextPreferences::default();
        assert_eq!(
            prefs.force_compact(&record),
            Err(NativeContextError::InvalidHistory)
        );
        assert_eq!(prefs, NativeContextPreferences::default());
    }
}

fn deep() -> Value {
    (0..20_000).fold(Value::Null, |value, _| Value::Array(vec![value]))
}

fn drop_deep(mut value: Value) {
    loop {
        match value {
            Value::Array(mut values) if values.len() == 1 => value = values.pop().unwrap(),
            _ => return,
        }
    }
}

#[test]
fn borrowed_deep_values_are_neither_cloned_nor_recursively_dropped() {
    let mut metadata = BTreeMap::from([("unrelated".to_owned(), deep())]);
    assert_eq!(
        NativeContextPreferences::from_metadata(&metadata).unwrap(),
        NativeContextPreferences::default()
    );
    metadata.insert(NATIVE_CONTEXT_PREFERENCES_KEY.to_owned(), deep());
    assert_eq!(
        NativeContextPreferences::from_metadata(&metadata),
        Err(NativeContextError::MalformedPreferences)
    );
    drop_deep(metadata.remove(NATIVE_CONTEXT_PREFERENCES_KEY).unwrap());
    let mut record = record(&[("user", "assistant")]);
    record.metadata = metadata;
    assert_eq!(
        NativeContextPreferences::default()
            .projection(&record)
            .unwrap_err(),
        NativeContextError::RecordLimit
    );
    drop_deep(record.metadata.remove("unrelated").unwrap());
}

#[test]
fn store_envelope_bounds_do_not_narrow_admitted_records_to_core_defaults() {
    let mut record = record(&[]);
    record.messages = (0..5000).map(|_| Message::text(Role::User, "u")).collect();
    record
        .metadata
        .insert("large".to_owned(), json!("x".repeat(300 * 1024)));
    let mut prefs = NativeContextPreferences::default();
    assert!(prefs.force_compact(&record).unwrap());
    assert_eq!(prefs.first_retained_message(), 4999);
    assert!(prefs.projection(&record).unwrap().is_some());
    let mut exact = record_with_two("u", "a");
    exact.metadata.insert("payload".to_owned(), json!(""));
    let overhead = serde_json::to_vec(&exact).unwrap().len();
    exact.metadata.insert(
        "payload".to_owned(),
        json!("x".repeat(MAX_FILE_SESSION_BYTES - overhead)),
    );
    assert!(
        NativeContextPreferences::default()
            .projection(&exact)
            .unwrap()
            .is_none()
    );
    exact.metadata.insert(
        "payload".to_owned(),
        json!("x".repeat(MAX_FILE_SESSION_BYTES - overhead + 1)),
    );
    assert_eq!(
        NativeContextPreferences::default()
            .projection(&exact)
            .unwrap_err(),
        NativeContextError::RecordLimit
    );
}

#[test]
fn aggregate_json_nodes_and_block_preflight_work_are_bounded() {
    let mut record = record_with_two("u", "a");
    record
        .metadata
        .insert("wide".to_owned(), json!(vec![0; 65_536]));
    assert_eq!(
        NativeContextPreferences::default()
            .projection(&record)
            .unwrap_err(),
        NativeContextError::RecordLimit
    );
    record.metadata.clear();
    record.messages[0].content = vec![
        ContentBlock::Text {
            text: String::new()
        };
        MAX_FILE_SESSION_BYTES / 16 + 1
    ];
    assert_eq!(
        NativeContextPreferences::default()
            .projection(&record)
            .unwrap_err(),
        NativeContextError::RecordLimit
    );
}
