use machine_god_core::{
    ContentBlock, Message, Role, SessionId, SessionIncarnationId, SessionRecord, ToolCall,
    ToolCallId, ToolName,
};
use machine_god_native::{
    NATIVE_CONVERSATION_HISTORY_KEY, NativeConversationHistory as History,
    NativeConversationHistoryError as Error, NativeHistoryBackground as Background,
    NativeHistoryFileAction as Action, NativeHistoryFileEvidence as File,
    NativeHistoryFileSource as Source, NativeHistoryFileStatus as Status,
    NativeHistoryState as State,
};
use serde_json::{Value, json};

fn record() -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("history").unwrap(),
        SessionIncarnationId::new("incarnation").unwrap(),
    );
    record.messages = vec![
        Message::text(Role::System, "system"),
        Message::text(Role::User, "first"),
        Message::text(Role::Assistant, "answer"),
        Message::text(Role::User, "second"),
    ];
    record.next_turn_sequence = 10;
    record
}

fn decode(value: Value) -> Result<History, Error> {
    let mut record = record();
    record
        .metadata
        .insert(NATIVE_CONVERSATION_HISTORY_KEY.into(), value);
    History::from_record(&record)
}

fn group(index: usize, sequence: u64, state: &str) -> Value {
    json!({"first_user_message":index,"turn_sequence":sequence,"state":state,"files":[],"background":null})
}

fn envelope(groups: Vec<Value>) -> Value {
    Value::Object(serde_json::Map::from_iter([
        ("schema_version".into(), json!(1)),
        ("groups".into(), Value::Array(groups)),
    ]))
}

fn unknown_file(path: &str) -> Value {
    serde_json::to_value(File::new(path, Action::Read, false).unwrap()).unwrap()
}

#[test]
fn missing_facts_remain_missing_and_unrelated_metadata_is_not_interpreted() {
    let mut record = record();
    record
        .metadata
        .insert("unrelated".into(), json!({"status":"cancelled"}));
    assert!(History::from_record(&record).unwrap().groups().is_empty());
    assert_eq!(History::default().to_value(), envelope(vec![]));
}

#[test]
fn sparse_lifecycle_round_trip_and_binary_lookup() {
    let value = envelope(vec![group(1, 2, "completed"), group(3, 5, "running")]);
    let history = decode(value.clone()).unwrap();
    assert_eq!(history.to_value(), value);
    assert_eq!(history.group(1).unwrap().turn_sequence(), 2);
    assert_eq!(history.group(3).unwrap().first_user_message(), 3);
    assert_eq!(history.group(3).unwrap().state(), State::Running);
    assert!(history.group(2).is_none());
}

#[test]
fn continuation_preserves_facts_and_rejects_stale_observations() {
    let mut history = History::default();
    history.begin(1, 1).unwrap();
    history
        .upsert_file(
            1,
            1,
            File::new("private-path", Action::Edit, false).unwrap(),
        )
        .unwrap();
    history
        .set_background(
            1,
            1,
            Some(Background::new("private-log", None, true).unwrap()),
        )
        .unwrap();
    history.finish(1, 1, State::Cancelled).unwrap();
    history.begin(1, 3).unwrap();
    let group = history.group(1).unwrap();
    assert_eq!(group.files()[0].path(), "private-path");
    assert!(group.background().unwrap().expect_url());
    assert_eq!(group.state(), State::Running);
    assert_eq!(group.turn_sequence(), 3);
    let before = history.clone();
    assert_eq!(
        history.set_background(1, 1, None),
        Err(Error::InvalidTransition)
    );
    assert_eq!(
        history.finish(1, 1, State::Completed),
        Err(Error::InvalidTransition)
    );
    assert_eq!(history, before);
    assert_eq!(decode(history.to_value()).unwrap(), history);
}

#[test]
fn next_boundary_marks_only_known_running_predecessor_interrupted() {
    let mut history = History::default();
    history.begin(1, 2).unwrap();
    history.begin(3, 5).unwrap();
    assert_eq!(history.group(1).unwrap().state(), State::Interrupted);
    history.finish(3, 5, State::Failed).unwrap();
    history.begin(4, 6).unwrap();
    assert_eq!(history.group(3).unwrap().state(), State::Failed);
    assert_eq!(decode(history.to_value()), Err(Error::InvalidReference));
}

#[test]
fn explicit_outcomes_do_not_delete_independent_facts() {
    for state in [
        State::Completed,
        State::Cancelled,
        State::Failed,
        State::Interrupted,
    ] {
        let mut history = History::default();
        history.begin(1, 1).unwrap();
        history
            .upsert_file(1, 1, File::new("file", Action::Read, true).unwrap())
            .unwrap();
        history.finish(1, 1, state).unwrap();
        assert!(history.group(1).unwrap().files()[0].stale());
        assert_eq!(
            decode(history.to_value())
                .unwrap()
                .group(1)
                .unwrap()
                .state(),
            state
        );
        let before = history.clone();
        assert_eq!(history.finish(1, 1, state), Err(Error::InvalidTransition));
        assert_eq!(history, before);
    }
}

#[test]
fn previous_groups_accept_explicit_async_facts_but_not_lifecycle_settlement() {
    let mut history = History::default();
    history.begin(1, 1).unwrap();
    history.begin(3, 2).unwrap();
    history
        .set_background(
            1,
            1,
            Some(Background::new("log", Some("https://example.test"), true).unwrap()),
        )
        .unwrap();
    history
        .upsert_file(1, 1, File::new("file", Action::Read, true).unwrap())
        .unwrap();
    assert_eq!(
        history.group(1).unwrap().background().unwrap().url(),
        Some("https://example.test")
    );
    assert_eq!(
        history.group(1).unwrap().background().unwrap().log_path(),
        "log"
    );
    assert_eq!(
        history.finish(1, 1, State::Completed),
        Err(Error::InvalidTransition)
    );
    history.set_background(1, 1, None).unwrap();
    assert!(history.group(1).unwrap().background().is_none());
}

#[test]
fn file_upsert_preserves_observation_order_and_can_update_staleness() {
    let mut history = History::default();
    history.begin(1, 1).unwrap();
    for (path, action) in [
        ("z", Action::Read),
        ("a", Action::Write),
        ("a", Action::Read),
    ] {
        history
            .upsert_file(1, 1, File::new(path, action, false).unwrap())
            .unwrap();
    }
    history
        .upsert_file(1, 1, File::new("a", Action::Read, true).unwrap())
        .unwrap();
    let files = history.group(1).unwrap().files();
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].action().as_str(), "read");
    assert!(!files[0].stale());
    assert_eq!(files[1].action(), Action::Write);
    assert_eq!(files[0].path(), "z");
    assert_eq!(files[2].path(), "a");
    assert!(files[2].stale());
    assert_eq!(decode(history.to_value()).unwrap(), history);
}

#[test]
fn every_file_action_round_trips() {
    let mut history = History::default();
    history.begin(1, 1).unwrap();
    for action in [
        Action::Read,
        Action::Write,
        Action::Edit,
        Action::Delete,
        Action::Rename,
        Action::Copy,
        Action::Search,
        Action::List,
        Action::Unknown,
    ] {
        history
            .upsert_file(1, 1, File::new("file", action, false).unwrap())
            .unwrap();
    }
    assert_eq!(decode(history.to_value()).unwrap(), history);
}

#[test]
fn malformed_envelopes_and_required_nullables_rejected() {
    for value in [
        Value::Null,
        json!([]),
        json!({}),
        json!({"schema_version":1,"groups":[],"extra":null}),
        json!({"schema_version":1.0,"groups":[]}),
        json!({"schema_version":1,"groups":null}),
    ] {
        assert_eq!(decode(value), Err(Error::Malformed));
    }
    assert_eq!(
        decode(json!({"schema_version":2,"groups":[]})),
        Err(Error::UnsupportedVersion)
    );
    for field in [
        "first_user_message",
        "turn_sequence",
        "state",
        "files",
        "background",
    ] {
        let mut value = group(1, 1, "completed");
        value.as_object_mut().unwrap().remove(field);
        assert_eq!(decode(envelope(vec![value])), Err(Error::Malformed));
    }
    let mut value = group(1, 1, "completed");
    value["background"] = json!({"log_path":"log","expect_url":false});
    assert_eq!(decode(envelope(vec![value])), Err(Error::Malformed));
}

#[test]
fn wrong_scalar_types_and_unknown_variants_are_rejected() {
    for (field, bad) in [
        ("first_user_message", json!(1.0)),
        ("first_user_message", json!(-1)),
        ("turn_sequence", json!(1.0)),
        ("turn_sequence", json!(-1)),
        ("state", json!("success")),
        ("state", json!({})),
        ("files", json!({})),
        ("background", json!([])),
    ] {
        let mut value = group(1, 1, "completed");
        value[field] = bad;
        assert_eq!(decode(envelope(vec![value])), Err(Error::Malformed));
    }
}

#[test]
fn invalid_references_monotonicity_and_running_order_rejected() {
    for groups in [
        vec![group(0, 1, "completed")],
        vec![group(2, 1, "completed")],
        vec![group(4, 1, "completed")],
        vec![group(1, 0, "completed")],
        vec![group(1, 10, "completed")],
        vec![group(1, 2, "completed"), group(1, 3, "completed")],
        vec![group(3, 2, "completed"), group(1, 3, "completed")],
        vec![group(1, 3, "completed"), group(3, 2, "completed")],
        vec![group(1, 2, "running"), group(3, 3, "completed")],
    ] {
        assert_eq!(decode(envelope(groups)), Err(Error::InvalidReference));
    }
}

#[test]
fn duplicate_file_facts_and_invalid_attachments_rejected() {
    let file = json!({"path":"a","action":"read","stale":false});
    for files in [
        json!([file, file]),
        json!([{"path":"a","action":"read","stale":null}]),
        json!([{"path":"a","action":"invented","stale":false}]),
        json!([{"path":"a","action":"read","stale":false,"extra":1}]),
    ] {
        let mut group = group(1, 1, "completed");
        group["files"] = files;
        assert_eq!(decode(envelope(vec![group])), Err(Error::Malformed));
    }
    for background in [
        json!({"log_path":"log","url":4,"expect_url":true}),
        json!({"log_path":"log","url":null,"expect_url":"yes"}),
        json!({"log_path":"log","url":null,"expect_url":true,"extra":1}),
    ] {
        let mut group = group(1, 1, "completed");
        group["background"] = background;
        assert_eq!(decode(envelope(vec![group])), Err(Error::Malformed));
    }
}

#[test]
fn invalid_transitions_are_failure_atomic() {
    let mut history = History::default();
    assert_eq!(
        history.finish(1, 1, State::Completed),
        Err(Error::InvalidTransition)
    );
    assert_eq!(history.begin(1, 0), Err(Error::InvalidReference));
    history.begin(3, 5).unwrap();
    let before = history.clone();
    for (index, sequence) in [(1, 6), (3, 5), (4, 4)] {
        assert_eq!(
            history.begin(index, sequence),
            Err(Error::InvalidTransition)
        );
        assert_eq!(history, before);
    }
    assert_eq!(
        history.finish(3, 5, State::Running),
        Err(Error::InvalidTransition)
    );
    assert_eq!(
        history.set_background(1, 5, None),
        Err(Error::InvalidTransition)
    );
    assert_eq!(history, before);
}

#[test]
fn string_boundaries_preserve_utf8_and_redact_diagnostics() {
    let path = "é".repeat(2048);
    assert!(File::new(&path, Action::Read, false).is_ok());
    assert_eq!(
        File::new(&(path + "x"), Action::Read, false),
        Err(Error::Limit)
    );
    assert!(Background::new("log", Some(&"é".repeat(1024)), false).is_ok());
    assert_eq!(
        Background::new("log", Some(&"x".repeat(2049)), false),
        Err(Error::Limit)
    );
    for bad in ["", "a\0b"] {
        assert_eq!(File::new(bad, Action::Read, false), Err(Error::Malformed));
        assert_eq!(Background::new(bad, None, false), Err(Error::Malformed));
        assert_eq!(
            Background::new("log", Some(bad), false),
            Err(Error::Malformed)
        );
    }
    let mut history = History::default();
    history.begin(1, 1).unwrap();
    history
        .upsert_file(1, 1, File::new("secret-file", Action::Read, false).unwrap())
        .unwrap();
    history
        .set_background(
            1,
            1,
            Some(Background::new("secret-log", Some("secret-url"), false).unwrap()),
        )
        .unwrap();
    for debug in [
        format!("{history:?}"),
        format!("{:?}", history.groups()),
        format!("{:?}", history.groups()[0].files()),
        format!("{:?}", history.groups()[0].background()),
    ] {
        assert!(!debug.contains("secret"));
    }
}

#[test]
fn total_nodes_and_serialized_bytes_are_bounded_before_text_clone() {
    let mut group = group(1, 1, "completed");
    group["files"] = Value::Array(
        (0..8191)
            .map(|index| unknown_file(&format!("{index:05}")))
            .collect(),
    );
    assert_eq!(decode(envelope(vec![group])), Err(Error::Limit));
    let mut group = self::group(1, 1, "completed");
    group["files"] = Value::Array(
        (0..2200)
            .map(|index| unknown_file(&format!("{index:04}{}", "x".repeat(4092))))
            .collect(),
    );
    assert_eq!(decode(envelope(vec![group])), Err(Error::Limit));
}

#[test]
fn node_limit_mutations_are_failure_atomic() {
    let mut group = group(1, 1, "completed");
    group["files"] = Value::Array(
        (0..8190)
            .map(|index| unknown_file(&format!("{index:05}")))
            .collect(),
    );
    let mut history = decode(envelope(vec![group])).unwrap();
    history
        .set_background(1, 1, Some(Background::new("log", None, false).unwrap()))
        .unwrap();
    let before = history.clone();
    assert_eq!(
        history.upsert_file(1, 1, File::new("last", Action::Read, false).unwrap()),
        Err(Error::Limit)
    );
    assert_eq!(history, before);
    assert_eq!(history.begin(3, 2), Err(Error::Limit));
    assert_eq!(history, before);
    // Replacement does not grow node count.
    history
        .upsert_file(1, 1, File::new("00000", Action::Read, true).unwrap())
        .unwrap();
    assert!(history.groups()[0].files()[0].stale());
}

#[test]
fn deep_hostile_reserved_values_are_rejected_shallowly() {
    let mut deep = Value::Null;
    for _ in 0..10_000 {
        deep = Value::Array(vec![deep]);
    }
    let mut group = group(1, 1, "completed");
    group["background"] = deep;
    let mut record = record();
    record.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.into(),
        envelope(vec![group]),
    );
    assert_eq!(History::from_record(&record), Err(Error::Malformed));
    // The caller owns hostile JSON; release it iteratively rather than invoking
    // serde_json's recursive destructor in this test.
    let mut pending: Vec<Value> = record.metadata.into_values().collect();
    while let Some(value) = pending.pop() {
        match value {
            Value::Array(values) => pending.extend(values),
            Value::Object(values) => pending.extend(values.into_values()),
            _ => {}
        }
    }
}

#[test]
fn no_small_group_cap_is_imposed() {
    let mut record = record();
    record.messages = (0..5000).map(|_| Message::text(Role::User, "u")).collect();
    record.next_turn_sequence = 5001;
    record.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.into(),
        envelope(
            (0..5000)
                .map(|index| group(index, index as u64 + 1, "completed"))
                .collect(),
        ),
    );
    assert_eq!(History::from_record(&record).unwrap().groups().len(), 5000);
}

fn source(message: usize, block: usize, name: &str) -> Source {
    Source::new(
        message,
        block,
        ToolCallId::new("reused").unwrap(),
        ToolName::new(name).unwrap(),
    )
    .unwrap()
}

fn execution(
    path: &str,
    action: Action,
    message: usize,
    name: &str,
    status: Status,
    destination: Option<&str>,
) -> File {
    File::new(path, action, false)
        .unwrap()
        .with_execution(
            source(message, 0, name),
            status,
            destination,
            status == Status::Success && action == Action::Read,
        )
        .unwrap()
}

fn call_message(name: &str) -> Message {
    Message {
        role: Role::Assistant,
        content: vec![ContentBlock::ToolCall {
            call: ToolCall {
                id: ToolCallId::new("reused").unwrap(),
                name: ToolName::new(name).unwrap(),
                arguments: json!({}),
            },
        }],
    }
}

fn execution_record(history: &History) -> SessionRecord {
    let mut record = record();
    record.messages = vec![
        Message::text(Role::User, "user"),
        call_message("read_file"),
        call_message("write_file"),
        call_message("read_file"),
        Message::text(Role::User, "next"),
        call_message("read_file"),
    ];
    record
        .metadata
        .insert(NATIVE_CONVERSATION_HISTORY_KEY.into(), history.to_value());
    record
}

#[test]
fn repeated_call_ids_and_read_write_read_preserve_distinct_observations() {
    let mut history = History::default();
    history.begin(0, 1).unwrap();
    history
        .upsert_file(
            0,
            1,
            execution("a", Action::Read, 1, "read_file", Status::Success, None),
        )
        .unwrap();
    history
        .upsert_file(
            0,
            1,
            execution("a", Action::Write, 2, "write_file", Status::Success, None),
        )
        .unwrap();
    history
        .upsert_file(
            0,
            1,
            execution("a", Action::Read, 3, "read_file", Status::Success, None),
        )
        .unwrap();
    let files = history.groups()[0].files();
    assert_eq!(files.len(), 3);
    assert!(files[0].stale());
    assert!(!files[2].stale());
    assert!(files[2].model_view_covers_full_file());
    assert_eq!(files[0].source().unwrap().call_id().as_str(), "reused");
    assert_eq!(files[0].source().unwrap().tool_name().as_str(), "read_file");
    assert_eq!(files[0].source().unwrap().content_block(), 0);
    assert_eq!(files[2].source().unwrap().assistant_message(), 3);
    assert_eq!(
        History::from_record(&execution_record(&history)).unwrap(),
        history
    );
    history.finish(0, 1, State::Cancelled).unwrap();
    history.begin(0, 2).unwrap();
    history
        .upsert_file(
            0,
            2,
            execution("a", Action::Read, 3, "read_file", Status::Success, None),
        )
        .unwrap();
    assert_eq!(history.groups()[0].files().len(), 3);
    assert_eq!(
        History::from_record(&execution_record(&history)).unwrap(),
        history
    );
}

#[test]
fn successful_mutation_staleness_uses_source_and_destination_but_copy_spares_source() {
    for action in [
        Action::Write,
        Action::Edit,
        Action::Delete,
        Action::Rename,
        Action::Copy,
    ] {
        for status in [Status::Unknown, Status::Failure, Status::Success] {
            let mut history = History::default();
            history.begin(0, 1).unwrap();
            // The prior read need not have succeeded to become stale.
            history
                .upsert_file(
                    0,
                    1,
                    execution(
                        "source",
                        Action::Read,
                        1,
                        "read_file",
                        Status::Failure,
                        None,
                    ),
                )
                .unwrap();
            history
                .upsert_file(
                    0,
                    1,
                    execution(
                        "destination",
                        Action::Read,
                        2,
                        "read_file",
                        Status::Success,
                        None,
                    ),
                )
                .unwrap();
            let destination =
                matches!(action, Action::Copy | Action::Rename).then_some("destination");
            history
                .upsert_file(
                    0,
                    1,
                    execution("source", action, 3, "mutation", status, destination),
                )
                .unwrap();
            let files = history.groups()[0].files();
            assert_eq!(
                files[0].stale(),
                status == Status::Success && action != Action::Copy
            );
            assert_eq!(
                files[1].stale(),
                status == Status::Success && destination.is_some()
            );
            assert_eq!(files[2].new_path(), destination);
            assert_eq!(files[2].status(), status);
        }
    }
}

#[test]
fn exact_source_updates_do_not_move_entries_or_miss_later_mutations() {
    let mut history = History::default();
    history.begin(0, 1).unwrap();
    history
        .upsert_file(
            0,
            1,
            execution("a", Action::Read, 1, "read_file", Status::Unknown, None),
        )
        .unwrap();
    history
        .upsert_file(
            0,
            1,
            execution("a", Action::Write, 2, "write_file", Status::Success, None),
        )
        .unwrap();
    history
        .upsert_file(
            0,
            1,
            execution("a", Action::Read, 1, "read_file", Status::Success, None),
        )
        .unwrap();
    assert_eq!(history.groups()[0].files().len(), 2);
    assert!(history.groups()[0].files()[0].stale());
    assert_eq!(history.groups()[0].files()[0].status(), Status::Success);
}

#[test]
fn sources_reject_wrong_role_group_block_and_identity() {
    for (message, block, id, name) in [
        (0, 0, "reused", "read_file"),
        (1, 1, "reused", "read_file"),
        (4, 0, "reused", "read_file"),
        (5, 0, "reused", "read_file"),
        (99, 0, "reused", "read_file"),
        (1, 0, "different", "read_file"),
        (1, 0, "reused", "write_file"),
    ] {
        let mut history = History::default();
        history.begin(0, 1).unwrap();
        let source = Source::new(
            message,
            block,
            ToolCallId::new(id).unwrap(),
            ToolName::new(name).unwrap(),
        )
        .unwrap();
        history
            .upsert_file(
                0,
                1,
                File::new("a", Action::Read, false)
                    .unwrap()
                    .with_execution(source, Status::Success, None, true)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            History::from_record(&execution_record(&history)),
            Err(Error::InvalidReference)
        );
    }
}

#[test]
fn malformed_source_fields_nullability_and_control_claims_are_rejected() {
    let file = serde_json::to_value(execution(
        "a",
        Action::Read,
        1,
        "read_file",
        Status::Success,
        None,
    ))
    .unwrap();
    for field in [
        "source",
        "new_path",
        "status",
        "model_view_covers_full_file",
    ] {
        let mut invalid = file.clone();
        invalid.as_object_mut().unwrap().remove(field);
        let mut record = execution_record(&History::default());
        let mut group = group(0, 1, "completed");
        group["files"] = json!([invalid]);
        record.metadata.insert(
            NATIVE_CONVERSATION_HISTORY_KEY.into(),
            envelope(vec![group]),
        );
        assert_eq!(History::from_record(&record), Err(Error::Malformed));
    }
    for (field, value) in [
        ("source", Value::Null),
        ("status", json!("invented")),
        ("new_path", json!("destination")),
        ("model_view_covers_full_file", json!(1)),
    ] {
        let mut invalid = file.clone();
        invalid[field] = value;
        let mut record = execution_record(&History::default());
        let mut group = group(0, 1, "completed");
        group["files"] = json!([invalid]);
        record.metadata.insert(
            NATIVE_CONVERSATION_HISTORY_KEY.into(),
            envelope(vec![group]),
        );
        assert!(History::from_record(&record).is_err());
    }
    for (status, action) in [
        (Status::Unknown, Action::Read),
        (Status::Failure, Action::Read),
        (Status::Success, Action::Write),
    ] {
        assert_eq!(
            File::new("a", action, false).unwrap().with_execution(
                source(1, 0, "read_file"),
                status,
                None,
                true
            ),
            Err(Error::Malformed)
        );
    }
    let mut record = execution_record(&History::default());
    let mut group = group(0, 1, "completed");
    group["files"] = json!([file, file]);
    record.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.into(),
        envelope(vec![group]),
    );
    assert_eq!(History::from_record(&record), Err(Error::Malformed));
}

#[test]
fn known_sources_cannot_be_appended_or_decoded_out_of_order() {
    let mut history = History::default();
    history.begin(0, 1).unwrap();
    history
        .upsert_file(
            0,
            1,
            execution("a", Action::Write, 2, "write_file", Status::Success, None),
        )
        .unwrap();
    let before = history.clone();
    assert_eq!(
        history.upsert_file(
            0,
            1,
            execution("a", Action::Read, 1, "read_file", Status::Success, None)
        ),
        Err(Error::InvalidReference)
    );
    assert_eq!(history, before);
    let mut record = execution_record(&history);
    record
        .metadata
        .get_mut(NATIVE_CONVERSATION_HISTORY_KEY)
        .unwrap()["groups"][0]["files"]
        .as_array_mut()
        .unwrap()
        .push(
            serde_json::to_value(execution(
                "a",
                Action::Read,
                1,
                "read_file",
                Status::Success,
                None,
            ))
            .unwrap(),
        );
    assert_eq!(History::from_record(&record), Err(Error::InvalidReference));
}

#[test]
fn known_sources_have_exact_shape_and_node_accounting() {
    let evidence = execution(
        "private",
        Action::Read,
        1,
        "read_file",
        Status::Success,
        None,
    );
    assert!(!format!("{:?}", evidence.source()).contains("reused"));
    for bad in [
        json!({"assistant_message":1,"content_block":0,"call_id":"reused"}),
        json!({"assistant_message":1.0,"content_block":0,"call_id":"reused","tool_name":"read_file"}),
        json!({"assistant_message":1,"content_block":-1,"call_id":"reused","tool_name":"read_file"}),
        json!({"assistant_message":1,"content_block":0,"call_id":"reused","tool_name":"read_file","extra":null}),
    ] {
        let mut file = serde_json::to_value(&evidence).unwrap();
        file["source"] = bad;
        let mut record = execution_record(&History::default());
        let mut group = group(0, 1, "completed");
        group["files"] = json!([file]);
        record.metadata.insert(
            NATIVE_CONVERSATION_HISTORY_KEY.into(),
            envelope(vec![group]),
        );
        assert!(History::from_record(&record).is_err());
    }
    let mut record = record();
    record.messages = std::iter::once(Message::text(Role::User, "user"))
        .chain((0..5460).map(|_| call_message("read_file")))
        .collect();
    let mut group = group(0, 1, "completed");
    group["files"] = Value::Array(
        (1..=5460)
            .map(|message| {
                serde_json::to_value(execution(
                    "a",
                    Action::Read,
                    message,
                    "read_file",
                    Status::Success,
                    None,
                ))
                .unwrap()
            })
            .collect(),
    );
    record.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.into(),
        envelope(vec![group]),
    );
    let mut history = History::from_record(&record).unwrap();
    let before = history.clone();
    assert_eq!(
        history.upsert_file(0, 1, File::new("unknown", Action::Read, false).unwrap()),
        Err(Error::Limit)
    );
    assert_eq!(history, before);
}
