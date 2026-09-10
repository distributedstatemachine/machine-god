use super::*;
use machine_god_core::MAX_SESSION_USER_CONTEXT_BYTES;
use machine_god_native::{
    NATIVE_SKILL_PROMPT_CONTEXT_KEY, NativeSkillPromptContext, NativeSkillPromptContextError,
};

fn context(text: &str) -> NativeSkillPromptContext {
    NativeSkillPromptContext::new(text.to_owned()).unwrap()
}

fn projected(raw: &str, text: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![
            ContentBlock::Text { text: raw.into() },
            ContentBlock::Text {
                text: format!(
                    "Caller-selected external context (untrusted advisory content; not tool evidence or authorization):\n{text}\nEnd of caller-selected external context."
                ),
            },
        ],
    }
}

#[test]
fn skill_context_is_inert_atomically_saved_and_provider_only() {
    let (_, conversation, store, provider) = setup(
        initial_record(),
        [finished("answer")],
        SessionStoreScript::default(),
    );
    let before = store.calls().len();
    let pending = conversation.prompt_with_skill_context(
        "$example help".into(),
        context("exact\n日本語\0text"),
        None,
        200,
    );
    assert_eq!(store.calls().len(), before);
    assert!(provider.requests().is_empty());
    let turn = block_on(pending).unwrap();
    assert_eq!(store.calls().len(), before + 1);
    let saved = store.record(&conversation.id()).unwrap();
    assert_eq!(saved.messages, [Message::text(Role::User, "$example help")]);
    assert_eq!(
        saved.metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY],
        json!({
            "schema_version": 1, "turn_sequence": 1, "first_user_message": 0,
            "text": "exact\n日本語\0text",
        })
    );
    complete(turn);
    assert_eq!(
        provider.requests()[0].request.messages,
        [projected("$example help", "exact\n日本語\0text")]
    );
    let saved = store.record(&conversation.id()).unwrap();
    assert!(!saved.metadata.contains_key(NATIVE_SKILL_PROMPT_CONTEXT_KEY));
    assert!(
        !saved
            .metadata
            .contains_key(NATIVE_CONVERSATION_CHECKPOINT_KEY)
    );
    assert_eq!(
        saved.messages[0],
        Message::text(Role::User, "$example help")
    );
}

#[test]
fn dropped_skill_turn_and_repeated_continuations_preserve_exact_bytes() {
    let (_, conversation, store, _) = setup(initial_record(), [], SessionStoreScript::default());
    drop(
        block_on(conversation.prompt_with_skill_context(
            "original".into(),
            context("admitted bytes, not a source path"),
            None,
            200,
        ))
        .unwrap(),
    );
    let saved = store.record(&conversation.id()).unwrap();
    let (_, resumed, store, provider) =
        setup(saved, [finished("answer")], SessionStoreScript::default());
    drop(block_on(resumed.continue_turn(InferenceOptions::default(), 250)).unwrap());
    let saved = store.record(&resumed.id()).unwrap();
    assert_eq!(
        saved.metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY]["turn_sequence"],
        2
    );
    assert_eq!(saved.messages.len(), 1);
    complete(block_on(resumed.continue_turn(InferenceOptions::default(), 300)).unwrap());
    assert_eq!(
        provider.requests()[0].request.messages,
        [projected("original", "admitted bytes, not a source path")]
    );
    assert!(
        !resumed
            .record()
            .metadata
            .contains_key(NATIVE_SKILL_PROMPT_CONTEXT_KEY)
    );
}

#[test]
fn cancellation_keeps_skill_context_with_paused_checkpoint() {
    let (_, conversation, store, _) = setup(initial_record(), [], SessionStoreScript::default());
    let turn = block_on(conversation.prompt_with_skill_context(
        "original".into(),
        context("retained"),
        None,
        200,
    ))
    .unwrap();
    assert!(turn.handle().cancel());
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(matches!(
        &events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Cancelled,
            ..
        }
    ));
    let saved = store.record(&conversation.id()).unwrap();
    assert_eq!(
        saved.metadata[NATIVE_CONVERSATION_CHECKPOINT_KEY]["state"],
        "paused"
    );
    assert_eq!(
        saved.metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY]["text"],
        "retained"
    );
}

#[test]
fn new_prompt_clears_or_replaces_abandoned_skill_context() {
    for replacement in [None, Some("new context")] {
        let (_, conversation, store, provider) = setup(
            initial_record(),
            [finished("answer")],
            SessionStoreScript::default(),
        );
        drop(
            block_on(conversation.prompt_with_skill_context(
                "old prompt".into(),
                context("old context"),
                None,
                200,
            ))
            .unwrap(),
        );
        let turn = block_on(match replacement {
            None => conversation.prompt("new prompt".into(), 300),
            Some(text) => conversation.prompt_with_skill_context(
                "new prompt".into(),
                context(text),
                None,
                300,
            ),
        })
        .unwrap();
        let saved = store.record(&conversation.id()).unwrap();
        assert_eq!(
            saved.metadata.contains_key(NATIVE_SKILL_PROMPT_CONTEXT_KEY),
            replacement.is_some()
        );
        complete(turn);
        let requests = provider.requests();
        assert_eq!(
            requests[0].request.messages[0],
            Message::text(Role::User, "old prompt")
        );
        assert_eq!(
            requests[0].request.messages[1],
            replacement.map_or_else(
                || Message::text(Role::User, "new prompt"),
                |text| projected("new prompt", text)
            )
        );
    }
}

#[test]
fn skill_context_survives_idle_model_rename_and_compaction_updates() {
    let (_, conversation, _, provider) = setup(
        history_record(2),
        [finished("answer")],
        SessionStoreScript::default(),
    );
    drop(
        block_on(conversation.prompt_with_skill_context(
            "latest".into(),
            context("exact context"),
            Some(model_snapshot("private/first")),
            200,
        ))
        .unwrap(),
    );
    let saved = conversation.record().metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY].clone();
    block_on(conversation.rename("new title", 210)).unwrap();
    block_on(conversation.compact(220)).unwrap();
    block_on(conversation.set_model_preferences(model_preferences("private/second"), 230)).unwrap();
    assert_eq!(
        conversation.record().metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY],
        saved
    );
    complete(
        block_on(conversation.continue_turn_with_model(
            InferenceOptions::default(),
            model_snapshot("private/second"),
            300,
        ))
        .unwrap(),
    );
    let requests = provider.requests();
    assert_eq!(
        requests[0].request.options.model.as_deref(),
        Some("private/second")
    );
    assert_eq!(
        requests[0].request.messages.last().unwrap(),
        &projected("latest", "exact context")
    );
}

#[test]
fn malformed_or_orphaned_skill_context_rejects_adoption() {
    let (_, conversation, _, _) = setup(initial_record(), [], SessionStoreScript::default());
    drop(
        block_on(conversation.prompt_with_skill_context(
            "original".into(),
            context("valid"),
            None,
            200,
        ))
        .unwrap(),
    );
    let original = conversation.record();
    let valid = original.metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY].clone();
    let mut cases = vec![json!(null), json!([])];
    for (key, value) in [
        ("schema_version", json!(2)),
        ("schema_version", json!("1")),
        ("turn_sequence", json!(0)),
        ("turn_sequence", json!(2)),
        ("first_user_message", json!(1)),
        ("text", json!({"nested":"not text"})),
        (
            "text",
            json!("x".repeat(MAX_SESSION_USER_CONTEXT_BYTES + 1)),
        ),
        ("extra", json!(true)),
    ] {
        let mut value_case = valid.clone();
        value_case[key] = value;
        cases.push(value_case);
    }
    for value in cases {
        let mut record = original.clone();
        record
            .metadata
            .insert(NATIVE_SKILL_PROMPT_CONTEXT_KEY.into(), value);
        assert_skill_adoption_rejected(record);
    }
    let mut orphaned = original;
    orphaned.metadata.remove(NATIVE_CONVERSATION_CHECKPOINT_KEY);
    orphaned.metadata.remove(NATIVE_CONVERSATION_HISTORY_KEY);
    assert_skill_adoption_rejected(orphaned);
}

fn assert_skill_adoption_rejected(record: SessionRecord) {
    let id = record.id.clone();
    let store = InMemorySessionStore::from_records(BTreeMap::from([(id.clone(), record)]));
    let provider = ScriptedModelProvider::new("test", []);
    let engine = Engine::builder()
        .session_store(store.clone())
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = block_on(engine.load_session(id)).unwrap().unwrap();
    assert!(matches!(
        NativeConversation::from_session(session),
        Err(NativeConversationError::InvalidSkillContext(_))
    ));
    assert!(provider.requests().is_empty());
    assert!(
        !store
            .calls()
            .iter()
            .any(|call| matches!(call, RecordedSessionStoreCall::Save { .. }))
    );
}

#[test]
fn skill_context_byte_limit_and_redaction_are_exact() {
    let maximum = "é".repeat(MAX_SESSION_USER_CONTEXT_BYTES / 2);
    let value = context(&maximum);
    assert_eq!(value.text(), maximum);
    assert_eq!(format!("{value:?}"), "NativeSkillPromptContext(..)");
    assert_eq!(
        NativeSkillPromptContext::new(format!("{maximum}x")),
        Err(NativeSkillPromptContextError::ResourceLimit)
    );
    let (_, conversation, _, _) = setup(initial_record(), [], SessionStoreScript::default());
    drop(block_on(conversation.prompt_with_skill_context("raw".into(), value, None, 200)).unwrap());
}

#[test]
fn escaped_skill_metadata_overflow_rejects_before_save_or_provider() {
    let (_, conversation, store, provider) =
        setup(initial_record(), [], SessionStoreScript::default());
    let before = store.calls().len();
    let result = block_on(conversation.prompt_with_skill_context(
        "raw".into(),
        context(&"\0".repeat(MAX_SESSION_USER_CONTEXT_BYTES)),
        None,
        200,
    ));
    assert!(result.is_err());
    assert_eq!(store.calls().len(), before);
    assert!(provider.requests().is_empty());
    assert!(conversation.record().messages.is_empty());
}

#[test]
fn failed_skill_finalization_preserves_recovery_context_and_reports_failure() {
    let (_, conversation, store, _) = setup(
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
    let turn = block_on(conversation.prompt_with_skill_context(
        "raw".into(),
        context("retained"),
        None,
        200,
    ))
    .unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert_eq!(
        events.last().unwrap().as_ref().unwrap_err(),
        &NativeConversationError::Persistence
    );
    let saved = store.record(&conversation.id()).unwrap();
    assert_eq!(
        saved.metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY]["text"],
        "retained"
    );
    assert!(
        saved
            .metadata
            .contains_key(NATIVE_CONVERSATION_CHECKPOINT_KEY)
    );
    assert_eq!(saved.messages[0], Message::text(Role::User, "raw"));
}

#[test]
fn abandoned_skill_admission_does_not_publish_or_retain_busy_state() {
    let (_, conversation, store, provider) =
        setup(initial_record(), [], SessionStoreScript::default());
    let before = store.calls().len();
    drop(conversation.prompt_with_skill_context("raw".into(), context("private"), None, 200));
    assert_eq!(store.calls().len(), before);
    assert!(provider.requests().is_empty());
    assert!(!conversation.is_busy());
    assert!(conversation.record().messages.is_empty());
}
