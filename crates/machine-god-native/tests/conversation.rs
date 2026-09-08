#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeMap;
use std::task::{Context, Poll};

use futures_core::Stream;
use futures_executor::block_on;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{
    ContentBlock, Engine, InferenceOptions, Message, ModelEvent, Prompt, Role, SessionId,
    SessionIncarnationId, SessionRecord, SessionRevision, SessionStoreError, SessionStoreErrorKind,
    StopReason, ToolCall, ToolCallId, ToolName, ToolOutput, TurnEvent,
};
use machine_god_native::{
    AI_GATEWAY_INFERENCE_OPTIONS_KEY, NATIVE_CONTEXT_PREFERENCES_KEY,
    NATIVE_CONVERSATION_CHECKPOINT_KEY, NATIVE_CONVERSATION_HISTORY_KEY,
    NATIVE_MODEL_PREFERENCES_KEY, NATIVE_SESSION_METADATA_KEY, NativeContextError,
    NativeContextPreferences, NativeConversation, NativeConversationError,
    NativeConversationHistory, NativeConversationTurn, NativeHistoryBackground,
    NativeHistoryFileAction, NativeHistoryFileEvidence, NativeHistoryFileSource,
    NativeHistoryFileStatus, NativeHistoryState, NativeModelCapabilities, NativeModelPreferences,
    NativeModelSnapshot, NativeReasoningEffort, NativeSessionMetadata, NativeSessionOrigin,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, RecordedSessionStoreCall, ScriptedModelProvider,
    ScriptedPermissionHandler, SessionStoreScript, SessionStoreStep,
};
use serde_json::{Value, json};

fn initial_record() -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("conversation").unwrap(),
        SessionIncarnationId::new("conversation-life").unwrap(),
    );
    record.revision = SessionRevision(1);
    record.metadata.insert(
        NATIVE_SESSION_METADATA_KEY.to_owned(),
        NativeSessionMetadata::new(
            std::path::Path::new("/workspace"),
            100,
            NativeSessionOrigin::Cli,
        )
        .unwrap()
        .to_value(),
    );
    record
        .metadata
        .insert("unrelated".to_owned(), json!({"preserve": true}));
    record
}

fn finished(text: &str) -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: text.to_owned(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}

fn setup(
    record: SessionRecord,
    steps: impl IntoIterator<Item = ModelProviderStep>,
    script: SessionStoreScript,
) -> (
    Engine,
    NativeConversation,
    InMemorySessionStore,
    ScriptedModelProvider,
) {
    let id = record.id.clone();
    let store =
        InMemorySessionStore::configured(BTreeMap::from([(id.clone(), record)]), script, 100);
    let provider = ScriptedModelProvider::new("conversation", steps);
    let engine = Engine::builder()
        .session_store(store.clone())
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    let conversation = NativeConversation::from_session(session).unwrap();
    (engine, conversation, store, provider)
}

fn complete(turn: NativeConversationTurn) {
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    assert!(matches!(
        &events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
}

fn history_record(groups: usize) -> SessionRecord {
    let mut record = initial_record();
    record
        .messages
        .push(Message::text(Role::System, "host system"));
    for index in 0..groups {
        record
            .messages
            .push(Message::text(Role::User, format!("question {index}")));
        record
            .messages
            .push(Message::text(Role::Assistant, format!("answer {index}")));
    }
    record.next_turn_sequence = u64::try_from(groups).unwrap() + 1;
    record
}

fn text_of(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_context_busy(conversation: &NativeConversation) {
    assert_eq!(
        conversation.history().unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        conversation.model_preferences().unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        block_on(conversation.set_model_preferences(NativeModelPreferences::default(), 300))
            .unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        conversation.context_preferences().unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        block_on(conversation.compact(300)).unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        block_on(conversation.set_max_history_turns(1, 300)).unwrap_err(),
        NativeConversationError::Busy
    );
}

#[test]
fn native_history_is_reserved_with_input_and_finalized_without_inferred_drop_reason() {
    let (_, conversation, store, provider) = setup(
        initial_record(),
        [finished("answer")],
        SessionStoreScript::default(),
    );
    assert!(conversation.history().unwrap().groups().is_empty());
    let before = store.calls().len();
    let pending = conversation.prompt("first".into(), 200);
    assert_eq!(store.calls().len(), before);
    let turn = block_on(pending).unwrap();
    assert_eq!(store.calls().len(), before + 1);
    let saved = store.record(&conversation.id()).unwrap();
    let history = NativeConversationHistory::from_record(&saved).unwrap();
    assert_eq!(
        history.group(0).unwrap().state(),
        NativeHistoryState::Running
    );
    assert_eq!(history.group(0).unwrap().turn_sequence(), 1);
    assert!(provider.requests().is_empty());
    drop(turn);
    assert_eq!(conversation.history().unwrap(), history);
    assert_eq!(store.record(&conversation.id()).unwrap(), saved);

    let turn = block_on(conversation.prompt("second".into(), 300)).unwrap();
    let reserved = NativeConversationHistory::from_record(&conversation.record()).unwrap();
    assert_eq!(
        reserved.group(0).unwrap().state(),
        NativeHistoryState::Interrupted
    );
    assert_eq!(
        reserved.group(1).unwrap().state(),
        NativeHistoryState::Running
    );
    complete(turn);
    let history = conversation.history().unwrap();
    assert_eq!(
        history.group(0).unwrap().state(),
        NativeHistoryState::Interrupted
    );
    assert_eq!(
        history.group(1).unwrap().state(),
        NativeHistoryState::Completed
    );
    assert_eq!(provider.requests().len(), 1);
}

#[test]
fn cancelled_failed_and_continued_history_use_actual_native_outcomes() {
    let (_, conversation, _, provider) = setup(
        initial_record(),
        [finished("continued")],
        SessionStoreScript::default(),
    );
    let turn = block_on(conversation.prompt("cancel me".into(), 200)).unwrap();
    let _ = turn.handle().cancel();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(
        conversation.history().unwrap().group(0).unwrap().state(),
        NativeHistoryState::Cancelled
    );
    assert!(provider.requests().is_empty());
    complete(block_on(conversation.continue_turn(InferenceOptions::default(), 300)).unwrap());
    let history = conversation.history().unwrap();
    assert_eq!(history.groups().len(), 1);
    assert_eq!(
        history.group(0).unwrap().state(),
        NativeHistoryState::Completed
    );
    assert_eq!(history.group(0).unwrap().turn_sequence(), 2);
    assert_eq!(
        conversation
            .record()
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .count(),
        1
    );

    let (_, failed, _, _) = setup(initial_record(), [], SessionStoreScript::default());
    let events = block_on(
        block_on(failed.prompt("no provider result".into(), 200))
            .unwrap()
            .collect::<Vec<_>>(),
    );
    assert!(events.iter().any(
        |event| matches!(event, Ok(event) if matches!(event.payload, TurnEvent::Failed { .. }))
    ));
    assert_eq!(
        failed.history().unwrap().group(0).unwrap().state(),
        NativeHistoryState::Failed
    );
}

#[test]
fn failed_history_finalization_does_not_publish_a_completed_fact() {
    let (_, conversation, store, _) = setup(
        initial_record(),
        [finished("saved answer")],
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Error(store_error()),
            ]),
            ..SessionStoreScript::default()
        },
    );
    let events = block_on(
        block_on(conversation.prompt("question".into(), 200))
            .unwrap()
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        events.last().unwrap().as_ref().unwrap_err(),
        &NativeConversationError::Persistence
    );
    let saved = store.record(&conversation.id()).unwrap();
    assert_eq!(saved.messages.len(), 2);
    assert_eq!(
        NativeConversationHistory::from_record(&saved)
            .unwrap()
            .group(0)
            .unwrap()
            .state(),
        NativeHistoryState::Running
    );
    assert_eq!(
        saved.metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY]["state"],
        "running"
    );
}

#[test]
fn explicit_history_observations_are_exact_attempt_saves_not_effects() {
    let (_, conversation, store, provider) = setup(
        initial_record(),
        [finished("one"), finished("two")],
        SessionStoreScript::default(),
    );
    complete(block_on(conversation.prompt("first".into(), 200)).unwrap());
    complete(block_on(conversation.prompt("second".into(), 300)).unwrap());
    let before = conversation.record();
    let file =
        NativeHistoryFileEvidence::new("src/file.rs", NativeHistoryFileAction::Edit, true).unwrap();
    let background =
        NativeHistoryBackground::new("logs/run.log", Some("http://localhost:1234"), true).unwrap();
    let calls = store.calls().len();
    drop(conversation.record_history_file(0, 1, file.clone(), 400));
    drop(conversation.set_history_background(0, 1, Some(background.clone()), 400));
    assert_eq!(store.calls().len(), calls);
    block_on(conversation.record_history_file(0, 1, file.clone(), 400)).unwrap();
    block_on(conversation.set_history_background(0, 1, Some(background.clone()), 500)).unwrap();
    let saved = conversation.record();
    assert_eq!(saved.messages, before.messages);
    assert_eq!(saved.incarnation_id, before.incarnation_id);
    assert_eq!(saved.next_turn_sequence, before.next_turn_sequence);
    assert_eq!(saved.metadata["unrelated"], before.metadata["unrelated"]);
    let history = conversation.history().unwrap();
    assert_eq!(
        history.group(0).unwrap().files(),
        std::slice::from_ref(&file)
    );
    assert_eq!(history.group(0).unwrap().background(), Some(&background));
    assert_eq!(provider.requests().len(), 2);
    let calls = store.calls().len();
    assert!(block_on(conversation.record_history_file(0, 2, file, 600)).is_err());
    assert_eq!(store.calls().len(), calls);
    assert_eq!(conversation.record(), saved);
}

#[test]
fn per_call_file_observations_survive_native_saves_and_reject_dangling_sources() {
    let mut record = initial_record();
    record
        .messages
        .push(Message::text(Role::User, "read, edit, read again"));
    for name in ["read_file", "write_file", "read_file"] {
        record.messages.extend([
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("reused").unwrap(),
                        name: ToolName::new(name).unwrap(),
                        arguments: json!({"path":"file.rs"}),
                    },
                }],
            },
            Message {
                role: Role::Tool,
                content: vec![ContentBlock::ToolResult {
                    call_id: ToolCallId::new("reused").unwrap(),
                    output: ToolOutput::success(json!("observed")),
                }],
            },
        ]);
    }
    record.next_turn_sequence = 2;
    let mut facts = NativeConversationHistory::default();
    facts.begin(0, 1).unwrap();
    facts.finish(0, 1, NativeHistoryState::Completed).unwrap();
    record
        .metadata
        .insert(NATIVE_CONVERSATION_HISTORY_KEY.to_owned(), facts.to_value());
    let (_, conversation, store, provider) = setup(record, [], SessionStoreScript::default());
    let original = conversation.record();
    for (index, name, action) in [
        (1, "read_file", NativeHistoryFileAction::Read),
        (3, "write_file", NativeHistoryFileAction::Write),
        (5, "read_file", NativeHistoryFileAction::Read),
    ] {
        let evidence = file_call_observation(index, name, action);
        block_on(conversation.record_history_file(0, 1, evidence, 200)).unwrap();
    }
    let history = conversation.history().unwrap();
    let files = history.group(0).unwrap().files();
    assert_eq!(files.len(), 3);
    assert!(files[0].stale());
    assert!(!files[2].stale());
    assert_eq!(files[0].source().unwrap().assistant_message(), 1);
    assert_eq!(files[2].source().unwrap().assistant_message(), 5);
    let saved = conversation.record();
    assert_eq!(saved.messages, original.messages);
    assert_eq!(store.record(&conversation.id()).unwrap(), saved);
    let calls = store.calls().len();
    for (index, name) in [(99, "read_file"), (2, "read_file"), (1, "wrong_tool")] {
        let invalid = file_call_observation(index, name, NativeHistoryFileAction::Read);
        assert!(matches!(
            block_on(conversation.record_history_file(0, 1, invalid, 300)),
            Err(NativeConversationError::InvalidHistory(_))
        ));
        assert_eq!(store.calls().len(), calls);
        assert_eq!(conversation.record(), saved);
    }
    assert!(provider.requests().is_empty());
}

fn file_call_observation(
    assistant_message: usize,
    name: &str,
    action: NativeHistoryFileAction,
) -> NativeHistoryFileEvidence {
    NativeHistoryFileEvidence::new("file.rs", action, false)
        .unwrap()
        .with_execution(
            NativeHistoryFileSource::new(
                assistant_message,
                0,
                ToolCallId::new("reused").unwrap(),
                ToolName::new(name).unwrap(),
            )
            .unwrap(),
            NativeHistoryFileStatus::Success,
            None,
            action == NativeHistoryFileAction::Read,
        )
        .unwrap()
}

#[test]
fn contradictory_history_checkpoint_state_is_rejected_on_adoption() {
    let mut original = history_record(1);
    let mut history = NativeConversationHistory::default();
    history.begin(1, 1).unwrap();
    original.metadata.insert(
        NATIVE_CONVERSATION_HISTORY_KEY.to_owned(),
        history.to_value(),
    );
    let store = InMemorySessionStore::from_records(BTreeMap::from([(
        original.id.clone(),
        original.clone(),
    )]));
    let engine = Engine::builder()
        .session_store(store)
        .provider(ScriptedModelProvider::new("test", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(original.id)).unwrap().unwrap();
    assert_eq!(
        NativeConversation::from_session(session).unwrap_err(),
        NativeConversationError::InvalidCheckpoint
    );
}

fn model_preferences(model: &str) -> NativeModelPreferences {
    NativeModelPreferences::new(model, NativeReasoningEffort::parse("high").unwrap(), true).unwrap()
}

fn model_snapshot(model: &str) -> NativeModelSnapshot {
    NativeModelSnapshot::new(
        &model_preferences(model),
        &NativeModelCapabilities::default(),
    )
}

#[test]
fn model_preferences_save_is_inert_and_preserves_paused_history_and_context() {
    let (_engine, conversation, store, provider) =
        setup(history_record(2), [], SessionStoreScript::default());
    assert_eq!(conversation.model_preferences().unwrap(), None);
    drop(block_on(conversation.prompt("paused".into(), 150)).unwrap());
    block_on(conversation.set_max_history_turns(1, 175)).unwrap();
    let before = conversation.record();
    let preferences = model_preferences("private/model");
    let calls = store.calls().len();
    let save = conversation.set_model_preferences(preferences.clone(), 200);
    assert_eq!(store.calls().len(), calls);
    assert!(!conversation.is_busy());
    assert_eq!(conversation.model_preferences().unwrap(), None);
    drop(save);
    assert_eq!(store.calls().len(), calls);
    let revision = block_on(conversation.set_model_preferences(preferences.clone(), 200)).unwrap();
    assert_eq!(revision.0, before.revision.0 + 1);
    let saved = store.record(&conversation.id()).unwrap();
    assert_eq!(saved, conversation.record());
    assert_eq!(saved.messages, before.messages);
    assert_eq!(saved.next_turn_sequence, before.next_turn_sequence);
    assert_eq!(saved.incarnation_id, before.incarnation_id);
    assert_eq!(
        saved.metadata[NATIVE_MODEL_PREFERENCES_KEY],
        preferences.to_value()
    );
    for key in [
        "unrelated",
        NATIVE_CONVERSATION_CHECKPOINT_KEY,
        NATIVE_CONTEXT_PREFERENCES_KEY,
    ] {
        assert_eq!(saved.metadata[key], before.metadata[key]);
    }
    assert_eq!(
        saved.metadata[NATIVE_SESSION_METADATA_KEY]["updated_at_ms"],
        200
    );
    assert_eq!(conversation.model_preferences().unwrap(), Some(preferences));
    assert!(provider.requests().is_empty());
}

#[test]
fn model_snapshot_and_checkpoint_are_one_publication_and_continue_uses_current_selection() {
    let (_engine, conversation, store, provider) = setup(
        initial_record(),
        [finished("new model answer")],
        SessionStoreScript::default(),
    );
    let calls = store.calls().len();
    let start =
        conversation.prompt_with_model("original input".into(), model_snapshot("private/old"), 200);
    assert_eq!(store.calls().len(), calls);
    let turn = block_on(start).unwrap();
    assert_eq!(store.calls().len(), calls + 1);
    assert!(provider.requests().is_empty());
    let saved = store.record(&conversation.id()).unwrap();
    assert_eq!(
        saved.metadata[NATIVE_MODEL_PREFERENCES_KEY],
        model_preferences("private/old").to_value()
    );
    assert!(
        saved
            .metadata
            .contains_key(NATIVE_CONVERSATION_CHECKPOINT_KEY)
    );
    assert_eq!(
        saved.messages,
        [Message::text(Role::User, "original input")]
    );
    assert_context_busy(&conversation);
    drop(turn);
    let options = InferenceOptions {
        max_output_tokens: Some(17),
        ..InferenceOptions::default()
    };
    complete(
        block_on(conversation.continue_turn_with_model(
            options,
            model_snapshot("private/current"),
            300,
        ))
        .unwrap(),
    );
    assert_eq!(
        conversation.model_preferences().unwrap(),
        Some(model_preferences("private/current"))
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].request.options.model.as_deref(),
        Some("private/current")
    );
    assert_eq!(requests[0].request.options.max_output_tokens, Some(17));
    assert_eq!(
        requests[0].request.options.metadata[AI_GATEWAY_INFERENCE_OPTIONS_KEY],
        json!({
            "schema_version": 1, "reasoning_effort": "auto", "fast": false,
        })
    );
    assert_eq!(
        conversation.record().messages,
        [
            Message::text(Role::User, "original input"),
            Message::text(Role::Assistant, "new model answer"),
        ]
    );
}

#[test]
fn model_preference_save_failure_or_drop_does_not_claim_success() {
    let original = initial_record();
    let (_engine, conversation, store, provider) = setup(
        original.clone(),
        [],
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Error(store_error()),
                SessionStoreStep::Pending,
                SessionStoreStep::Pass,
            ]),
            ..SessionStoreScript::default()
        },
    );
    assert_eq!(
        block_on(conversation.set_model_preferences(model_preferences("private/model"), 200))
            .unwrap_err(),
        NativeConversationError::Persistence
    );
    assert_eq!(store.record(&conversation.id()).unwrap(), original);
    assert_eq!(conversation.model_preferences().unwrap(), None);
    let mut save = conversation.set_model_preferences(model_preferences("private/model"), 200);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(save.as_mut().poll(&mut cx).is_pending());
    assert_context_busy(&conversation);
    assert_eq!(
        block_on(conversation.prompt("busy".into(), 200)).unwrap_err(),
        NativeConversationError::Busy
    );
    drop(save);
    assert!(!conversation.is_busy());
    assert_eq!(store.record(&conversation.id()).unwrap(), original);
    block_on(conversation.set_model_preferences(model_preferences("private/retry"), 200)).unwrap();
    assert_eq!(
        conversation.model_preferences().unwrap(),
        Some(model_preferences("private/retry"))
    );
    assert!(provider.requests().is_empty());
}

#[test]
fn model_snapshot_failed_admission_publishes_neither_preferences_nor_input() {
    let original = initial_record();
    let (_engine, conversation, store, provider) = setup(
        original.clone(),
        [],
        SessionStoreScript {
            saves: Some(vec![SessionStoreStep::Error(store_error())]),
            ..SessionStoreScript::default()
        },
    );
    assert_eq!(
        block_on(conversation.prompt_with_model(
            "not saved".into(),
            model_snapshot("private/model"),
            200
        ))
        .unwrap_err(),
        NativeConversationError::Persistence
    );
    assert_eq!(store.record(&conversation.id()).unwrap(), original);
    assert_eq!(conversation.model_preferences().unwrap(), None);
    assert!(provider.requests().is_empty());
}

#[test]
fn model_snapshot_stays_fixed_across_tool_rounds_while_future_selection_changes() {
    use machine_god_core::ToolSpec;
    use machine_god_testkit::{ScriptedPreparedTool, ToolPrepareStep, ToolStep};
    let record = initial_record();
    let store =
        InMemorySessionStore::from_records(BTreeMap::from([(record.id.clone(), record.clone())]));
    let tool = ScriptedPreparedTool::new(
        ToolSpec {
            name: ToolName::new("pure").unwrap(),
            description: "test".to_owned(),
            input_schema: json!({"type":"object"}),
        },
        [ToolPrepareStep::NoAuthority {
            arguments: json!({}),
        }],
        [ToolStep::Output(ToolOutput::success(json!({"answer":42})))],
    );
    let provider = ScriptedModelProvider::new(
        "test",
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new("call").unwrap(),
                        name: ToolName::new("pure").unwrap(),
                        arguments: json!({}),
                    },
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            finished("first answer"),
            finished("next answer"),
        ],
    );
    let engine = Engine::builder()
        .session_store(store)
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .tool(tool.clone())
        .build()
        .unwrap();
    let session = block_on(engine.load_session(record.id)).unwrap().unwrap();
    let conversation = NativeConversation::from_session(session).unwrap();
    let mut preferences = model_preferences("private/original");
    let capabilities =
        NativeModelCapabilities::new(&[NativeReasoningEffort::parse("high").unwrap()], true)
            .unwrap();
    let mut turn = block_on(conversation.prompt_with_model(
        "question".into(),
        NativeModelSnapshot::new(&preferences, &capabilities),
        200,
    ))
    .unwrap();
    loop {
        let event = block_on(turn.next()).expect("tool must complete").unwrap();
        if matches!(event.payload, TurnEvent::ToolFinished { .. }) {
            break;
        }
        assert!(!matches!(
            event.payload,
            TurnEvent::Completed { .. } | TurnEvent::Failed { .. }
        ));
    }
    assert_eq!(tool.invocations().len(), 1);
    preferences.set_model("private/next").unwrap();
    preferences.set_effort(NativeReasoningEffort::default());
    preferences.toggle_fast(&capabilities);
    assert_context_busy(&conversation);
    complete(turn);
    complete(
        block_on(conversation.prompt_with_model(
            "next question".into(),
            NativeModelSnapshot::new(&preferences, &capabilities),
            300,
        ))
        .unwrap(),
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    for request in &requests[..2] {
        assert_eq!(
            request.request.options.model.as_deref(),
            Some("private/original")
        );
        assert_eq!(
            request.request.options.metadata[AI_GATEWAY_INFERENCE_OPTIONS_KEY],
            json!({"schema_version":1,"reasoning_effort":"high","fast":true})
        );
    }
    assert_eq!(
        requests[2].request.options.model.as_deref(),
        Some("private/next")
    );
    assert_eq!(
        requests[2].request.options.metadata[AI_GATEWAY_INFERENCE_OPTIONS_KEY],
        json!({"schema_version":1,"reasoning_effort":"auto","fast":false})
    );
}

#[test]
fn model_preferences_reject_time_regression_and_stale_cross_engine_writes() {
    let (_engine, conversation, store, provider) =
        setup(initial_record(), [], SessionStoreScript::default());
    let calls = store.calls().len();
    assert!(matches!(
        block_on(conversation.set_model_preferences(model_preferences("private/invalid-time"), 99))
            .unwrap_err(),
        NativeConversationError::InvalidMetadata(_)
    ));
    assert_eq!(store.calls().len(), calls);
    let other_engine = Engine::builder()
        .session_store(store.clone())
        .provider(ScriptedModelProvider::new("other", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let other = NativeConversation::from_session(
        block_on(other_engine.load_session(conversation.id()))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    block_on(other.set_model_preferences(model_preferences("private/winner"), 200)).unwrap();
    let winner = store.record(&conversation.id()).unwrap();
    assert_eq!(
        block_on(conversation.set_model_preferences(model_preferences("private/stale"), 300))
            .unwrap_err(),
        NativeConversationError::Conflict
    );
    assert_eq!(store.record(&conversation.id()).unwrap(), winner);
    assert!(provider.requests().is_empty());
}

#[test]
fn malformed_saved_model_preferences_are_not_overwritten_by_admission_or_setter() {
    for value in [
        Value::Null,
        json!({"schema_version":2,"model":"private/model","effort":"auto","fast_mode":false}),
        json!({"schema_version":1,"model":"private/model","effort":"auto","fast_mode":"yes"}),
    ] {
        let (engine, conversation, store, provider) =
            setup(initial_record(), [], SessionStoreScript::default());
        let session = block_on(engine.load_session(conversation.id()))
            .unwrap()
            .unwrap();
        let mut record = session.record();
        record
            .metadata
            .insert(NATIVE_MODEL_PREFERENCES_KEY.to_owned(), value);
        block_on(session.update_metadata(record.revision, record.metadata)).unwrap();
        let original = store.record(&conversation.id()).unwrap();
        let calls = store.calls().len();
        for error in [
            conversation.model_preferences().unwrap_err(),
            block_on(
                conversation.set_model_preferences(model_preferences("private/replacement"), 200),
            )
            .unwrap_err(),
            block_on(conversation.prompt("plain".into(), 200)).unwrap_err(),
            block_on(conversation.prompt_with_model(
                "snapshot".into(),
                model_snapshot("private/replacement"),
                200,
            ))
            .unwrap_err(),
            NativeConversation::from_session(session).unwrap_err(),
        ] {
            assert!(matches!(
                error,
                NativeConversationError::InvalidModelPreferences(_)
            ));
            assert!(!format!("{error:?}: {error}").contains("private"));
        }
        assert_eq!(store.calls().len(), calls);
        assert_eq!(store.record(&conversation.id()).unwrap(), original);
        assert!(provider.requests().is_empty());
    }
}

#[test]
fn manual_context_compaction_persists_selection_without_deleting_history() {
    let original = history_record(3);
    let (_engine, conversation, store, provider) = setup(
        original.clone(),
        [finished("new answer")],
        SessionStoreScript::default(),
    );
    let before_calls = store.calls().len();
    drop(conversation.compact(200));
    drop(conversation.set_max_history_turns(2, 200));
    assert_eq!(store.calls().len(), before_calls);
    assert!(!conversation.is_busy());
    assert_eq!(
        conversation.context_preferences().unwrap(),
        NativeContextPreferences::default()
    );

    assert!(block_on(conversation.compact(200)).unwrap());
    let compacted = store.record(&conversation.id()).unwrap();
    assert_eq!(compacted.messages, original.messages);
    assert_eq!(compacted.next_turn_sequence, original.next_turn_sequence);
    assert_eq!(compacted.revision, SessionRevision(2));
    assert_eq!(
        compacted.metadata["unrelated"],
        original.metadata["unrelated"]
    );
    assert_eq!(
        compacted.metadata[NATIVE_SESSION_METADATA_KEY]["updated_at_ms"],
        200
    );
    assert_eq!(
        conversation
            .context_preferences()
            .unwrap()
            .first_retained_message(),
        5
    );
    assert!(provider.requests().is_empty());
    let calls = store.calls().len();
    assert!(!block_on(conversation.compact(300)).unwrap());
    assert_eq!(store.calls().len(), calls);
    assert_eq!(store.record(&conversation.id()).unwrap(), compacted);

    block_on(conversation.set_max_history_turns(2, 250)).unwrap();
    assert_eq!(
        conversation
            .context_preferences()
            .unwrap()
            .first_retained_message(),
        5
    );

    complete(block_on(conversation.prompt("new question".into(), 300)).unwrap());
    let request = provider.requests().remove(0).request;
    assert_eq!(request.messages[0], original.messages[0]);
    assert_eq!(request.messages[1].role, Role::Assistant);
    let summary = text_of(&request.messages[1]);
    assert!(summary.contains("Conversation summary:"));
    assert!(summary.contains("question 0"));
    assert!(summary.contains("question 1"));
    assert_eq!(request.messages[2..4], original.messages[5..]);
    assert_eq!(
        request.messages[4],
        Message::text(Role::User, "new question")
    );
    let stored = store.record(&conversation.id()).unwrap();
    assert_eq!(
        stored.messages[..original.messages.len()],
        original.messages
    );
    assert_eq!(stored.messages.len(), original.messages.len() + 2);
    assert_eq!(
        conversation
            .context_preferences()
            .unwrap()
            .first_retained_message(),
        5
    );
}

#[test]
fn compact_zero_or_one_group_is_an_effect_free_noop() {
    for groups in [0, 1] {
        let original = history_record(groups);
        let (_engine, conversation, store, provider) =
            setup(original.clone(), [], SessionStoreScript::default());
        let calls = store.calls().len();
        assert!(!block_on(conversation.compact(200)).unwrap());
        assert_eq!(store.calls().len(), calls);
        assert_eq!(store.record(&conversation.id()).unwrap(), original);
        assert!(provider.requests().is_empty());
    }
}

#[test]
fn automatic_context_limits_apply_to_provider_requests_not_canonical_messages() {
    for maximum in [0, 1, 2] {
        let original = history_record(4);
        let (_engine, conversation, store, provider) = setup(
            original.clone(),
            [finished("new answer")],
            SessionStoreScript::default(),
        );
        assert_eq!(
            block_on(conversation.set_max_history_turns(maximum, 200)).unwrap(),
            SessionRevision(2)
        );
        complete(block_on(conversation.prompt("new question".into(), 300)).unwrap());
        let request = provider.requests().remove(0).request;
        assert_eq!(request.messages[0], original.messages[0]);
        if maximum == 0 {
            assert_eq!(
                request.messages[..original.messages.len()],
                original.messages
            );
        } else {
            let retained = if maximum == 1 { 1 } else { 2 };
            if maximum == 2 {
                assert!(text_of(&request.messages[1]).contains("Earlier turns compacted: 3"));
            }
            assert_eq!(
                request.messages[retained..retained + 2],
                original.messages[7..]
            );
            assert_eq!(request.messages.len(), retained + 3);
        }
        let record = store.record(&conversation.id()).unwrap();
        assert_eq!(
            record.messages[..original.messages.len()],
            original.messages
        );
        assert_eq!(
            record.metadata[NATIVE_CONTEXT_PREFERENCES_KEY]["first_retained_message"],
            0
        );
    }
}

#[test]
fn compaction_retains_paused_user_and_all_confirmed_unknown_tool_rounds() {
    let mut original = history_record(2);
    let start = original.messages.len();
    original
        .messages
        .push(Message::text(Role::User, "unfinished user request"));
    for (index, is_error) in [(0, false), (1, true)] {
        let call_id = ToolCallId::new(format!("historical-{index}")).unwrap();
        original.messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                call: ToolCall {
                    id: call_id.clone(),
                    name: ToolName::new("not_registered").unwrap(),
                    arguments: json!({}),
                },
            }],
        });
        original.messages.push(Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id,
                output: ToolOutput {
                    content: if is_error {
                        json!({"code":"tool_result_unknown"})
                    } else {
                        json!({"result":"confirmed receipt"})
                    },
                    is_error,
                },
            }],
        });
        original.messages.push(Message::text(
            Role::Assistant,
            format!("continued round {index}"),
        ));
    }
    original.next_turn_sequence = 4;
    let checkpoint =
        json!({"schema_version":1,"turn_sequence":3,"first_user_message":start,"state":"paused"});
    original.metadata.insert(
        NATIVE_CONVERSATION_CHECKPOINT_KEY.to_owned(),
        checkpoint.clone(),
    );
    let (_engine, conversation, store, provider) = setup(
        original.clone(),
        [finished("finished continuation")],
        SessionStoreScript::default(),
    );
    assert!(block_on(conversation.compact(200)).unwrap());
    assert_eq!(
        store.record(&conversation.id()).unwrap().metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY],
        checkpoint
    );
    assert!(
        conversation
            .paused_turn()
            .unwrap()
            .unwrap()
            .has_uncertain_tool_results
    );
    complete(block_on(conversation.continue_turn(InferenceOptions::default(), 300)).unwrap());
    let request = provider.requests().remove(0).request;
    assert_eq!(request.messages[2..], original.messages[start..]);
    let stored = store.record(&conversation.id()).unwrap();
    assert_eq!(
        stored.messages[..original.messages.len()],
        original.messages
    );
    assert_eq!(stored.next_turn_sequence, 5);
    assert!(conversation.paused_turn().unwrap().is_none());
}

#[test]
fn context_mutation_failure_and_pending_drop_preserve_prior_preferences() {
    let original = history_record(2);
    let (_engine, conversation, store, _) = setup(
        original.clone(),
        [],
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Error(store_error()),
                SessionStoreStep::Pending,
                SessionStoreStep::Pass,
            ]),
            ..SessionStoreScript::default()
        },
    );
    assert_eq!(
        block_on(conversation.compact(200)).unwrap_err(),
        NativeConversationError::Persistence
    );
    assert_eq!(store.record(&conversation.id()).unwrap(), original);
    assert_eq!(
        conversation.context_preferences().unwrap(),
        NativeContextPreferences::default()
    );
    let mut pending = conversation.set_max_history_turns(2, 200);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(pending.as_mut().poll(&mut cx).is_pending());
    assert_eq!(
        conversation.context_preferences().unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        block_on(conversation.compact(200)).unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        block_on(conversation.prompt("busy".into(), 200)).unwrap_err(),
        NativeConversationError::Busy
    );
    drop(pending);
    assert!(!conversation.is_busy());
    assert_eq!(store.record(&conversation.id()).unwrap(), original);
    assert!(block_on(conversation.compact(200)).unwrap());
}

#[test]
fn context_preferences_reject_invalid_inputs_without_persistence_or_provider_work() {
    let (_engine, conversation, store, provider) =
        setup(history_record(2), [], SessionStoreScript::default());
    let original = conversation.record();
    let calls = store.calls().len();
    assert!(matches!(
        block_on(conversation.compact(99)).unwrap_err(),
        NativeConversationError::InvalidMetadata(_)
    ));
    assert!(matches!(
        block_on(conversation.set_max_history_turns(2, 99)).unwrap_err(),
        NativeConversationError::InvalidMetadata(_)
    ));
    assert_eq!(
        block_on(conversation.set_max_history_turns(usize::MAX, 200)).unwrap_err(),
        NativeConversationError::InvalidContext(NativeContextError::PreferenceLimit)
    );
    assert_eq!(store.calls().len(), calls);
    assert_eq!(store.record(&conversation.id()).unwrap(), original);
    assert!(provider.requests().is_empty());
}

#[test]
fn malformed_or_stale_context_never_silently_falls_back_to_full_history() {
    for context in [
        json!(null),
        json!({"schema_version":2,"first_retained_message":0,"max_history_turns":0}),
        json!({"schema_version":1,"first_retained_message":2,"max_history_turns":0}),
        json!({"schema_version":1,"first_retained_message":99,"max_history_turns":0}),
    ] {
        let (engine, conversation, store, provider) =
            setup(history_record(2), [], SessionStoreScript::default());
        let session = block_on(engine.load_session(conversation.id()))
            .unwrap()
            .unwrap();
        let mut record = session.record();
        record
            .metadata
            .insert(NATIVE_CONTEXT_PREFERENCES_KEY.to_owned(), context);
        block_on(session.update_metadata(record.revision, record.metadata)).unwrap();
        let stored = store.record(&conversation.id()).unwrap();
        let calls = store.calls().len();
        assert!(matches!(
            conversation.context_preferences().unwrap_err(),
            NativeConversationError::InvalidContext(_)
        ));
        assert!(matches!(
            block_on(conversation.prompt("must not start".into(), 200)).unwrap_err(),
            NativeConversationError::InvalidContext(_)
        ));
        assert!(matches!(
            NativeConversation::from_session(session).unwrap_err(),
            NativeConversationError::InvalidContext(_)
        ));
        assert_eq!(store.calls().len(), calls);
        assert_eq!(store.record(&conversation.id()).unwrap(), stored);
        assert!(provider.requests().is_empty());
    }
}

#[test]
fn disabled_context_does_not_narrow_existing_full_history_admission() {
    let mut original = initial_record();
    original.messages.push(Message::text(
        Role::Assistant,
        "legacy assistant-led history",
    ));
    let (_engine, conversation, _, provider) = setup(
        original.clone(),
        [finished("new answer")],
        SessionStoreScript::default(),
    );
    block_on(conversation.set_max_history_turns(0, 200)).unwrap();
    complete(block_on(conversation.prompt("new question".into(), 300)).unwrap());
    assert_eq!(
        provider.requests()[0].request.messages[0],
        original.messages[0]
    );
}

fn store_error() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "fixture",
        "private store diagnostics",
        false,
    )
}

#[test]
fn conversation_admission_publishes_checkpoint_input_and_allocator_together() {
    let (_engine, conversation, store, provider) = setup(
        initial_record(),
        [finished("answer")],
        SessionStoreScript::default(),
    );
    let turn = block_on(conversation.prompt("question".into(), 200)).unwrap();
    assert!(provider.requests().is_empty());
    let record = store.record(&conversation.id()).unwrap();
    assert_eq!(record.revision, SessionRevision(2));
    assert_eq!(record.next_turn_sequence, 2);
    assert_eq!(record.messages, [Message::text(Role::User, "question")]);
    assert_eq!(
        record.metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY],
        json!({
            "schema_version": 1, "turn_sequence": 1, "first_user_message": 0, "state": "running",
        })
    );
    assert_eq!(record.metadata["unrelated"], json!({"preserve": true}));
    assert_eq!(
        record.metadata[NATIVE_SESSION_METADATA_KEY]["updated_at_ms"],
        200
    );
    assert_eq!(
        store
            .calls()
            .iter()
            .filter(|call| matches!(call, RecordedSessionStoreCall::Save { .. }))
            .count(),
        1
    );
    assert!(conversation.is_busy());
    complete(turn);
    assert!(!conversation.is_busy());
    assert_eq!(conversation.paused_turn().unwrap(), None);
    assert!(
        !store
            .record(&conversation.id())
            .unwrap()
            .metadata
            .contains_key(NATIVE_CONVERSATION_CHECKPOINT_KEY)
    );
}

#[test]
fn conversation_multiple_turns_preserve_canonical_history() {
    let (_engine, conversation, store, provider) = setup(
        initial_record(),
        [finished("first answer"), finished("second answer")],
        SessionStoreScript::default(),
    );
    complete(block_on(conversation.prompt("first question".into(), 200)).unwrap());
    complete(block_on(conversation.prompt("second question".into(), 300)).unwrap());
    let record = store.record(&conversation.id()).unwrap();
    assert_eq!(
        record.messages,
        [
            Message::text(Role::User, "first question"),
            Message::text(Role::Assistant, "first answer"),
            Message::text(Role::User, "second question"),
            Message::text(Role::Assistant, "second answer"),
        ]
    );
    assert_eq!(record.next_turn_sequence, 3);
    assert_eq!(
        provider.requests()[1].request.messages,
        record.messages[..3]
    );
}

#[test]
fn conversation_drop_preserves_checkpoint_and_continue_never_duplicates_user_input() {
    let (_engine, conversation, store, provider) = setup(
        initial_record(),
        [finished("continued")],
        SessionStoreScript::default(),
    );
    let first = block_on(conversation.prompt("original".into(), 200)).unwrap();
    let id = first.handle().id().clone();
    drop(first);
    assert!(provider.requests().is_empty());
    let paused = conversation.paused_turn().unwrap().unwrap();
    assert_eq!(paused.turn_sequence, 1);
    assert!(!paused.has_uncertain_tool_results);
    let continuation =
        block_on(conversation.continue_turn(InferenceOptions::default(), 300)).unwrap();
    assert_ne!(continuation.handle().id(), &id);
    let record = store.record(&conversation.id()).unwrap();
    assert_eq!(record.next_turn_sequence, 3);
    assert_eq!(record.messages, [Message::text(Role::User, "original")]);
    assert_eq!(
        record.metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY]["turn_sequence"],
        2
    );
    complete(continuation);
    assert_eq!(provider.requests().len(), 1);
    assert_eq!(
        provider.requests()[0].request.messages,
        [Message::text(Role::User, "original")]
    );
}

#[test]
fn conversation_cancel_produces_paused_state_before_forwarding_terminal_event() {
    let (_engine, conversation, store, _provider) = setup(
        initial_record(),
        [ModelProviderStep::pending()],
        SessionStoreScript::default(),
    );
    let mut turn = block_on(conversation.prompt("cancel me".into(), 200)).unwrap();
    assert!(matches!(
        block_on(turn.next()).unwrap().unwrap().payload,
        TurnEvent::Started
    ));
    assert!(turn.handle().cancel());
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    assert_eq!(
        store.record(&conversation.id()).unwrap().metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY]["state"],
        "paused"
    );
    assert!(conversation.paused_turn().unwrap().is_some());
}

#[test]
fn conversation_missing_checkpoint_and_busy_admission_have_no_extra_effects() {
    let (_engine, conversation, store, provider) =
        setup(initial_record(), [], SessionStoreScript::default());
    let calls = store.calls().len();
    assert_eq!(
        block_on(conversation.continue_turn(InferenceOptions::default(), 200)).unwrap_err(),
        NativeConversationError::NoCheckpoint
    );
    assert_eq!(store.calls().len(), calls);
    let first = block_on(conversation.prompt("first".into(), 200)).unwrap();
    let calls = store.calls().len();
    assert_context_busy(&conversation);
    assert_eq!(
        conversation.paused_turn().unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        block_on(conversation.prompt("second".into(), 300)).unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(store.calls().len(), calls);
    assert!(provider.requests().is_empty());
    drop(first);
}

#[test]
fn conversation_failed_reservation_does_not_create_checkpoint_or_invoke_provider() {
    let (_engine, conversation, store, provider) = setup(
        initial_record(),
        [],
        SessionStoreScript {
            saves: Some(vec![SessionStoreStep::Error(store_error())]),
            ..SessionStoreScript::default()
        },
    );
    assert_eq!(
        block_on(conversation.prompt("question".into(), 200)).unwrap_err(),
        NativeConversationError::Persistence
    );
    assert_eq!(store.record(&conversation.id()).unwrap(), initial_record());
    assert!(!conversation.is_busy());
    assert!(provider.requests().is_empty());
}

#[test]
fn conversation_pending_finalization_retains_native_admission_until_settled_or_dropped() {
    let (_engine, conversation, store, _) = setup(
        initial_record(),
        [finished("answer")],
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Pending,
            ]),
            ..SessionStoreScript::default()
        },
    );
    let mut turn = Box::pin(block_on(conversation.prompt("question".into(), 200)).unwrap());
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    loop {
        match turn.as_mut().poll_next(&mut cx) {
            Poll::Ready(Some(Ok(event))) => {
                assert!(!matches!(event.payload, TurnEvent::Completed { .. }));
            }
            Poll::Pending => break,
            other @ Poll::Ready(_) => panic!("unexpected {other:?}"),
        }
    }
    assert!(conversation.is_busy());
    assert_context_busy(&conversation);
    assert_eq!(
        block_on(conversation.prompt("too soon".into(), 300)).unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(
        store.record(&conversation.id()).unwrap().metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY]["state"],
        "running"
    );
    drop(turn);
    assert!(!conversation.is_busy());
    assert!(conversation.paused_turn().unwrap().is_some());
}

#[test]
fn conversation_failed_finalization_is_not_reported_as_completed() {
    let (_engine, conversation, store, _) = setup(
        initial_record(),
        [finished("answer")],
        SessionStoreScript {
            saves: Some(vec![
                SessionStoreStep::Pass,
                SessionStoreStep::Pass,
                SessionStoreStep::Error(store_error()),
            ]),
            ..SessionStoreScript::default()
        },
    );
    let turn = block_on(conversation.prompt("question".into(), 200)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert_eq!(
        events.last().unwrap().as_ref().unwrap_err(),
        &NativeConversationError::Persistence
    );
    assert!(!events.iter().any(|event| matches!(event,
        Ok(event) if matches!(event.payload, TurnEvent::Completed { .. }))));
    assert_eq!(
        store
            .record(&conversation.id())
            .unwrap()
            .messages
            .last()
            .unwrap(),
        &Message::text(Role::Assistant, "answer")
    );
    assert!(conversation.paused_turn().unwrap().is_some());
    assert!(!conversation.is_busy());
}

#[test]
fn conversation_fresh_owner_preserves_confirmed_and_unknown_tool_history() {
    let mut record = initial_record();
    record.next_turn_sequence = 2;
    record.messages = vec![
        Message::text(Role::User, "original"),
        Message {
            role: Role::Assistant,
            content: ["confirmed", "unknown"]
                .into_iter()
                .map(|id| ContentBlock::ToolCall {
                    call: ToolCall {
                        id: ToolCallId::new(id).unwrap(),
                        name: ToolName::new("effect").unwrap(),
                        arguments: json!({}),
                    },
                })
                .collect(),
        },
    ];
    for (id, content, is_error) in [
        ("confirmed", json!({"result": "retained"}), false),
        ("unknown", json!({"code": "tool_result_unknown"}), true),
    ] {
        record.messages.push(Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: ToolCallId::new(id).unwrap(),
                output: ToolOutput { content, is_error },
            }],
        });
    }
    record.metadata.insert(
        NATIVE_CONVERSATION_CHECKPOINT_KEY.to_owned(),
        json!({
            "schema_version": 1, "turn_sequence": 1, "first_user_message": 0, "state": "running",
        }),
    );
    let prefix = record.messages.clone();
    // There is no registered historical tool: a replay would fail this test.
    let (_engine, conversation, store, provider) = setup(
        record,
        [finished("continued")],
        SessionStoreScript::default(),
    );
    assert!(
        conversation
            .paused_turn()
            .unwrap()
            .unwrap()
            .has_uncertain_tool_results
    );
    complete(block_on(conversation.continue_turn(InferenceOptions::default(), 200)).unwrap());
    assert_eq!(provider.requests()[0].request.messages, prefix);
    assert_eq!(
        store.record(&conversation.id()).unwrap().messages[..prefix.len()],
        prefix
    );
}

#[test]
fn conversation_rejects_malformed_future_and_stale_checkpoints_without_mutation() {
    for checkpoint in [
        json!(null),
        json!({"schema_version": 2}),
        json!({"schema_version": 1, "turn_sequence": 2, "first_user_message": 0, "state": "running"}),
        json!({"schema_version": 1, "turn_sequence": 1, "first_user_message": 1, "state": "running"}),
        json!({"schema_version": 1, "turn_sequence": 1, "first_user_message": 0, "state": "other"}),
        json!({"schema_version": 1, "turn_sequence": 1, "first_user_message": 0, "state": "paused", "extra": 1}),
    ] {
        let mut record = initial_record();
        record.next_turn_sequence = 2;
        record
            .messages
            .push(Message::text(Role::User, "private input"));
        record
            .metadata
            .insert(NATIVE_CONVERSATION_CHECKPOINT_KEY.to_owned(), checkpoint);
        let store = InMemorySessionStore::from_records(BTreeMap::from([(
            record.id.clone(),
            record.clone(),
        )]));
        let engine = Engine::builder()
            .session_store(store.clone())
            .provider(ScriptedModelProvider::new("test", []))
            .permission_handler(ScriptedPermissionHandler::new([]))
            .build()
            .unwrap();
        let session = block_on(engine.load_session(record.id.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(
            NativeConversation::from_session(session).unwrap_err(),
            NativeConversationError::InvalidCheckpoint
        );
        assert_eq!(store.record(&record.id).unwrap(), record);
    }
}

fn deep_options() -> InferenceOptions {
    let mut value = Value::Null;
    for _ in 0..10_000 {
        value = Value::Array(vec![value]);
    }
    InferenceOptions {
        metadata: BTreeMap::from([("deep".to_owned(), value)]),
        ..InferenceOptions::default()
    }
}

#[test]
fn conversation_unpolled_and_rejected_deep_inputs_drop_iteratively() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let (_engine, conversation, store, provider) =
                setup(initial_record(), [], SessionStoreScript::default());
            let calls = store.calls().len();
            drop(conversation.prompt(
                Prompt {
                    text: "unpolled".to_owned(),
                    options: deep_options(),
                },
                200,
            ));
            drop(conversation.continue_turn(deep_options(), 200));
            drop(conversation.prompt_with_model(
                Prompt {
                    text: "unpolled model".to_owned(),
                    options: deep_options(),
                },
                model_snapshot("private/model"),
                200,
            ));
            drop(conversation.continue_turn_with_model(
                deep_options(),
                model_snapshot("private/model"),
                200,
            ));
            assert_eq!(
                block_on(conversation.continue_turn_with_model(
                    deep_options(),
                    model_snapshot("private/model"),
                    200
                ))
                .unwrap_err(),
                NativeConversationError::NoCheckpoint
            );
            assert_eq!(
                block_on(conversation.continue_turn(deep_options(), 200)).unwrap_err(),
                NativeConversationError::NoCheckpoint
            );
            assert_eq!(store.calls().len(), calls);
            assert!(provider.requests().is_empty());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn conversation_ordinary_non_cancel_stop_consumes_checkpoint() {
    let (_engine, conversation, _, _) = setup(
        initial_record(),
        [ModelProviderStep::events([ModelEvent::Stop {
            reason: StopReason::MaxOutputTokens,
        }])],
        SessionStoreScript::default(),
    );
    let turn = block_on(conversation.prompt("bounded answer".into(), 200)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(conversation.paused_turn().unwrap(), None);
}

#[test]
fn conversation_provider_failure_pauses_until_explicit_new_attempt() {
    use machine_god_core::{ProviderError, ProviderErrorKind};
    let (_engine, conversation, _, provider) = setup(
        initial_record(),
        [
            ModelProviderStep::StartError(ProviderError::new(
                ProviderErrorKind::Unavailable,
                "unavailable",
                "private provider details",
                true,
            )),
            finished("retry answer"),
        ],
        SessionStoreScript::default(),
    );
    let turn = block_on(conversation.prompt("preserve me".into(), 200)).unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(matches!(
        &events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Failed { .. }
    ));
    assert_eq!(provider.requests().len(), 1);
    assert!(conversation.paused_turn().unwrap().is_some());
    complete(block_on(conversation.continue_turn(InferenceOptions::default(), 300)).unwrap());
    assert_eq!(provider.requests().len(), 2);
    assert_eq!(
        provider.requests()[1].request.messages,
        [Message::text(Role::User, "preserve me")]
    );
    assert_eq!(conversation.paused_turn().unwrap(), None);
}

#[test]
fn conversation_metadata_time_failure_and_debug_are_redacted() {
    let (_engine, conversation, store, provider) =
        setup(initial_record(), [], SessionStoreScript::default());
    let calls = store.calls().len();
    let error = block_on(conversation.prompt("private prompt".into(), 99)).unwrap_err();
    assert!(matches!(error, NativeConversationError::InvalidMetadata(_)));
    assert_eq!(store.calls().len(), calls);
    assert!(provider.requests().is_empty());
    let debug = format!("{conversation:?} {error:?} {error}");
    for private in ["private prompt", "/workspace", "conversation-life"] {
        assert!(!debug.contains(private));
    }
}

#[test]
fn conversation_rename_obeys_admission_and_preserves_paused_checkpoint() {
    let (_engine, conversation, store, _) =
        setup(initial_record(), [], SessionStoreScript::default());
    let turn = block_on(conversation.prompt("original".into(), 200)).unwrap();
    let calls = store.calls().len();
    assert_eq!(
        block_on(conversation.rename("busy title", 300)).unwrap_err(),
        NativeConversationError::Busy
    );
    assert_eq!(store.calls().len(), calls);
    drop(turn);
    let checkpoint = conversation.record().metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY].clone();
    let revision = block_on(conversation.rename("  persisted title  ", 300)).unwrap();
    let record = store.record(&conversation.id()).unwrap();
    assert_eq!(record.revision, revision);
    assert_eq!(
        record.metadata[NATIVE_SESSION_METADATA_KEY]["title"],
        "persisted title"
    );
    assert_eq!(
        record.metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY],
        checkpoint
    );
    assert_eq!(record.messages, [Message::text(Role::User, "original")]);
}

fn file_lifecycle(
    directory: &std::path::Path,
    provider: ScriptedModelProvider,
) -> machine_god_native::NativeSessionLifecycle {
    use machine_god_core::SessionStore;
    use machine_god_native::{FileSessionStore, NativeSessionLifecycle};
    use std::sync::Arc;
    let store = Arc::new(FileSessionStore::open(directory).unwrap());
    let shared: Arc<dyn SessionStore> = store.clone();
    let engine = Engine::builder()
        .provider(provider)
        .shared_session_store(shared)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    NativeSessionLifecycle::new(engine, store).unwrap()
}

#[test]
fn conversation_real_store_create_drop_and_fresh_resume_continues_once() {
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = loop {
        let path = std::env::temp_dir().join(format!(
            "mg-conversation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => break Directory(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("create fixture: {error}"),
        }
    };
    let first_provider = ScriptedModelProvider::new("test", [finished("earlier answer")]);
    let lifecycle = file_lifecycle(&directory.0, first_provider.clone());
    let metadata = NativeSessionMetadata::new(
        &directory.0.canonicalize().unwrap(),
        100,
        NativeSessionOrigin::Cli,
    )
    .unwrap();
    let create = NativeConversation::create(&lifecycle, metadata.clone());
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    let conversation = block_on(create).unwrap();
    let id = conversation.id();
    assert_eq!(conversation.record().revision, SessionRevision(1));
    assert_eq!(
        conversation.record().metadata[NATIVE_SESSION_METADATA_KEY],
        metadata.to_value()
    );
    complete(block_on(conversation.prompt("earlier question".into(), 150)).unwrap());
    drop(block_on(conversation.prompt("persisted original".into(), 200)).unwrap());
    assert!(block_on(conversation.compact(225)).unwrap());
    block_on(conversation.set_max_history_turns(1, 250)).unwrap();
    let saved_model = model_preferences("private/saved");
    block_on(conversation.set_model_preferences(saved_model.clone(), 275)).unwrap();
    let preferences = conversation.context_preferences().unwrap();
    assert_eq!(preferences.first_retained_message(), 2);
    assert_eq!(first_provider.requests().len(), 1);
    drop(conversation);
    drop(lifecycle);
    let provider = ScriptedModelProvider::new("test", [finished("resumed answer")]);
    let lifecycle = file_lifecycle(&directory.0, provider.clone());
    let conversation = block_on(NativeConversation::resume(&lifecycle, id.clone())).unwrap();
    assert_eq!(
        conversation.paused_turn().unwrap().unwrap().turn_sequence,
        2
    );
    assert_eq!(conversation.context_preferences().unwrap(), preferences);
    assert_eq!(
        conversation.model_preferences().unwrap(),
        Some(saved_model.clone())
    );
    complete(
        block_on(conversation.continue_turn_with_model(
            InferenceOptions::default(),
            NativeModelSnapshot::new(&saved_model, &NativeModelCapabilities::default()),
            300,
        ))
        .unwrap(),
    );
    let record = block_on(lifecycle.replay(id)).unwrap();
    assert_eq!(record.next_turn_sequence, 4);
    assert_eq!(
        record.messages,
        [
            Message::text(Role::User, "earlier question"),
            Message::text(Role::Assistant, "earlier answer"),
            Message::text(Role::User, "persisted original"),
            Message::text(Role::Assistant, "resumed answer")
        ]
    );
    assert!(
        !record
            .metadata
            .contains_key(NATIVE_CONVERSATION_CHECKPOINT_KEY)
    );
    assert_eq!(provider.requests().len(), 1);
    assert_eq!(
        provider.requests()[0].request.options.model.as_deref(),
        Some("private/saved")
    );
    assert_eq!(
        provider.requests()[0].request.messages,
        [Message::text(Role::User, "persisted original")]
    );
}
