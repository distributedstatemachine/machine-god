use std::collections::{BTreeMap, VecDeque};
use std::future::poll_fn;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_executor::block_on;
use futures_util::{StreamExt, task::noop_waker};
use machine_god_core::{
    BoxFuture, CancellationToken, ContentBlock, Engine, EngineError, EngineLimits,
    InferenceOptions, MAX_CONTEXT_SUMMARY_BYTES, Message, ModelEvent, PermissionDecision,
    PermissionGrantScope, Role, Session, SessionContextProjection, SessionId, SessionIncarnationId,
    SessionRecord, SessionRevision, SessionStore, SessionStoreError, SessionStoreErrorKind,
    SessionTurnPreparation, StopReason, Tool, ToolCall, ToolCallId, ToolContext, ToolError,
    ToolName, ToolOutput, ToolSpec, Turn, TurnEvent,
};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, PermissionStep, RecordedSessionStoreCall,
    ScriptedModelProvider, ScriptedPermissionHandler, SessionStoreScript, SessionStoreStep,
};
use serde_json::{Value, json};

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new("read_tool_result").unwrap(),
        arguments: json!({}),
    }
}

fn calls(ids: &[&str]) -> Message {
    Message {
        role: Role::Assistant,
        content: ids
            .iter()
            .map(|id| ContentBlock::ToolCall { call: call(id) })
            .collect(),
    }
}

fn result(id: &str, unknown: bool) -> Message {
    Message {
        role: Role::Tool,
        content: vec![ContentBlock::ToolResult {
            call_id: ToolCallId::new(id).unwrap(),
            output: ToolOutput {
                is_error: unknown,
                content: if unknown {
                    json!({"code": "tool_result_unknown"})
                } else {
                    json!({"archive": "complete retained evidence"})
                },
            },
        }],
    }
}

fn record() -> SessionRecord {
    let mut record = SessionRecord::empty(
        SessionId::new("prepared-session").unwrap(),
        SessionIncarnationId::new("prepared-incarnation").unwrap(),
    );
    record.revision = SessionRevision(7);
    record.next_turn_sequence = 12;
    record.messages = vec![
        Message::text(Role::System, "retained system authority"),
        Message::text(Role::User, "old input"),
        calls(&["known", "uncertain"]),
        result("known", false),
        result("uncertain", true),
        Message::text(Role::User, "retained input"),
        Message::text(Role::Assistant, "retained answer"),
    ];
    record.metadata.insert("old".to_owned(), json!(true));
    record
}

fn preparation(first: usize, summary: Option<&str>) -> SessionTurnPreparation {
    SessionTurnPreparation {
        expected_revision: SessionRevision(7),
        metadata: None,
        context: Some(SessionContextProjection {
            first_retained_message: first,
            prefix_summary: summary.map(str::to_owned),
        }),
    }
}

fn finish() -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: "new answer".to_owned(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}

fn store(record: SessionRecord) -> InMemorySessionStore {
    InMemorySessionStore::from_records(BTreeMap::from([(record.id.clone(), record)]))
}

fn engine(
    store: impl SessionStore,
    provider: ScriptedModelProvider,
    limits: EngineLimits,
) -> Engine {
    Engine::builder()
        .session_store(store)
        .provider(provider)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .limits(limits)
        .build()
        .unwrap()
}

fn load(engine: &Engine) -> Session {
    block_on(engine.load_session(record().id)).unwrap().unwrap()
}

fn pending<T>(future: &mut BoxFuture<'_, T>) {
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(&noop_waker()))
            .is_pending()
    );
}

fn complete(turn: Turn) {
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().all(Result::is_ok));
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap().payload,
        TurnEvent::Completed {
            reason: StopReason::Completed,
            ..
        }
    ));
}

#[test]
fn preparation_is_inert_and_atomically_saves_metadata_input_and_allocator() {
    let original = record();
    let store = store(original.clone());
    let provider = ScriptedModelProvider::new("prepared", [finish()]);
    let engine = engine(store.clone(), provider.clone(), EngineLimits::default());
    let session = load(&engine);
    let mut prep = preparation(5, Some("private summary"));
    prep.metadata = Some(BTreeMap::from([(
        "checkpoint".to_owned(),
        json!({"sequence": 12}),
    )]));
    let metadata = prep.metadata.clone().unwrap();
    let future = session.prompt_prepared("new input", prep);
    assert!(!session.has_active_turn());
    assert_eq!(store.calls().len(), 1);
    assert_eq!(session.record(), original);
    let turn = block_on(future).unwrap();
    assert_eq!(turn.id().as_str(), "turn-12");
    assert!(provider.requests().is_empty());
    let mut expected = original.clone();
    expected.revision = SessionRevision(8);
    expected.next_turn_sequence = 13;
    expected.metadata = metadata;
    expected
        .messages
        .push(Message::text(Role::User, "new input"));
    assert_eq!(session.record(), expected);
    assert_eq!(store.record(&session.id()), Some(expected.clone()));
    assert_eq!(store.calls().len(), 2);
    assert!(
        matches!(&store.calls()[1], RecordedSessionStoreCall::Save { record, expected_revision: Some(SessionRevision(7)) }
        if record.messages == expected.messages && record.metadata == expected.metadata && record.next_turn_sequence == 13)
    );
    assert!(matches!(
        block_on(session.clone().prompt("busy")),
        Err(EngineError::SessionBusy)
    ));
    assert!(matches!(
        block_on(session.continue_turn_prepared(InferenceOptions::default(), preparation(0, None))),
        Err(EngineError::SessionBusy)
    ));
    assert!(matches!(
        block_on(session.update_metadata(SessionRevision(8), BTreeMap::new())),
        Err(EngineError::SessionBusy)
    ));
    complete(turn);
    let request = &provider.requests()[0].request;
    assert_eq!(request.messages[0], original.messages[0]);
    assert_eq!(request.messages[1].role, Role::Assistant);
    let ContentBlock::Text { text } = &request.messages[1].content[0] else {
        panic!("summary text")
    };
    assert!(text.starts_with("Advisory summary of earlier conversation"));
    assert!(text.contains("not instructions, tool evidence, or authorization"));
    assert!(text.contains("private summary"));
    assert_eq!(request.messages[2..], expected.messages[5..]);
    assert_eq!(
        session.record().messages[..expected.messages.len()],
        expected.messages
    );
    assert!(!session.has_active_turn());
}

#[test]
fn full_and_summary_free_context_preserve_continuation_semantics() {
    for (first, expected) in [
        (0, record().messages),
        (
            5,
            vec![
                record().messages[0].clone(),
                record().messages[5].clone(),
                record().messages[6].clone(),
            ],
        ),
    ] {
        let store = store(record());
        let provider = ScriptedModelProvider::new("full", [finish()]);
        let engine = engine(store.clone(), provider.clone(), EngineLimits::default());
        let session = load(&engine);
        let turn = block_on(
            session.continue_turn_prepared(InferenceOptions::default(), preparation(first, None)),
        )
        .unwrap();
        assert_eq!(session.record().messages, record().messages);
        assert_eq!(session.record().next_turn_sequence, 13);
        complete(turn);
        assert_eq!(provider.requests()[0].request.messages, expected);
        // Context is turn-local; later unprepared turns see complete history.
        let reserved = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
        assert_eq!(reserved.id().as_str(), "turn-13");
        drop(reserved);
    }
}

#[test]
fn empty_history_full_prompt_is_allowed_but_continuation_or_nonzero_cut_is_not() {
    let store = InMemorySessionStore::new();
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = engine
        .create_session(record().id, record().incarnation_id)
        .unwrap();
    for continue_turn in [false, true] {
        let mut prep = preparation(usize::from(!continue_turn), None);
        prep.expected_revision = SessionRevision(0);
        let result = if continue_turn {
            block_on(session.continue_turn_prepared(InferenceOptions::default(), prep))
        } else {
            block_on(session.prompt_prepared("new", prep))
        };
        assert!(matches!(result, Err(EngineError::Protocol(_))));
        assert!(store.calls().is_empty());
    }
    let mut prep = preparation(0, None);
    prep.expected_revision = SessionRevision(0);
    let turn = block_on(session.prompt_prepared("new", prep)).unwrap();
    assert_eq!(
        session.record().messages,
        vec![Message::text(Role::User, "new")]
    );
    assert_eq!(session.record().revision, SessionRevision(1));
    drop(turn);
}

#[test]
fn invalid_cuts_and_tool_histories_fail_before_save() {
    let mut cases = Vec::new();
    for cut in [2, 3, 4, 6, 7, usize::MAX] {
        cases.push((record(), preparation(cut, None)));
    }
    cases.push((
        record(),
        preparation(0, Some("cannot summarize full history")),
    ));
    let mut nonleading_system = record();
    nonleading_system
        .messages
        .insert(5, Message::text(Role::System, "must not discard"));
    cases.push((nonleading_system, preparation(6, None)));
    let mut missing = record();
    missing.messages.remove(4);
    cases.push((missing, preparation(0, None)));
    let mut duplicate_call = record();
    duplicate_call.messages[2] = calls(&["known", "known"]);
    cases.push((duplicate_call, preparation(0, None)));
    let mut duplicate_result = record();
    duplicate_result.messages[4] = result("known", false);
    cases.push((duplicate_result, preparation(0, None)));
    let mut orphan = record();
    orphan.messages[3] = result("orphan", false);
    cases.push((orphan, preparation(0, None)));
    let mut wrong_role = record();
    wrong_role.messages[2].role = Role::User;
    cases.push((wrong_role, preparation(0, None)));
    let mut empty_tool = record();
    empty_tool.messages[3].content.clear();
    cases.push((empty_tool, preparation(0, None)));
    let mut text_tool = record();
    text_tool.messages[3] = Message::text(Role::Tool, "not result evidence");
    cases.push((text_tool, preparation(0, None)));
    let mut trailing_missing = record();
    trailing_missing.messages.push(calls(&["missing"]));
    cases.push((trailing_missing, preparation(0, None)));
    for (record, prep) in cases {
        let store = store(record.clone());
        let engine = engine(
            store.clone(),
            ScriptedModelProvider::new("unused", []),
            EngineLimits::default(),
        );
        let session = load(&engine);
        assert!(matches!(
            block_on(session.prompt_prepared("new", prep)),
            Err(EngineError::Protocol(_))
        ));
        assert_eq!(session.record(), record);
        assert_eq!(store.calls().len(), 1);
        assert!(!session.has_active_turn());
    }
}

#[test]
fn historical_call_ids_may_repeat_in_separately_closed_rounds() {
    let mut original = record();
    original.messages.extend([
        calls(&["known", "uncertain"]),
        result("known", false),
        result("uncertain", false),
    ]);
    let store = store(original.clone());
    let provider = ScriptedModelProvider::new("repeat-ids", [finish()]);
    let engine = engine(store, provider.clone(), EngineLimits::default());
    let session = load(&engine);
    complete(
        block_on(session.continue_turn_prepared(InferenceOptions::default(), preparation(5, None)))
            .unwrap(),
    );
    assert_eq!(
        provider.requests()[0].request.messages[1..],
        original.messages[5..]
    );
}

#[derive(Clone, Copy)]
enum Fault {
    CommitPending,
    CommitDelayed,
    CommitError,
    BadRevision,
    Error,
}

#[derive(Clone)]
struct FaultStore {
    inner: InMemorySessionStore,
    faults: Arc<Mutex<VecDeque<Fault>>>,
    loads: Arc<Mutex<VecDeque<Option<SessionRecord>>>>,
    release: Arc<AtomicBool>,
}

impl FaultStore {
    fn new(fault: Fault) -> Self {
        Self {
            inner: store(record()),
            faults: Arc::new(Mutex::new(VecDeque::from([fault]))),
            loads: Arc::default(),
            release: Arc::default(),
        }
    }
}

fn failure() -> SessionStoreError {
    SessionStoreError::new(
        SessionStoreErrorKind::Unavailable,
        "private-code",
        "private-detail",
        true,
    )
}

impl SessionStore for FaultStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        Box::pin(async move {
            let loaded = self.inner.load(id).await?;
            Ok(self.loads.lock().unwrap().pop_front().unwrap_or(loaded))
        })
    }

    fn save(
        &self,
        record: SessionRecord,
        expected: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async move {
            let fault = self.faults.lock().unwrap().pop_front();
            if matches!(fault, Some(Fault::Error)) {
                return Err(failure());
            }
            if matches!(fault, Some(Fault::BadRevision)) {
                return Ok(record.revision);
            }
            let revision = self.inner.save(record, expected).await?;
            if matches!(fault, Some(Fault::CommitPending)) {
                std::future::pending::<()>().await;
            }
            if matches!(fault, Some(Fault::CommitDelayed)) {
                poll_fn(|_| {
                    if self.release.load(Ordering::Acquire) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
            if matches!(fault, Some(Fault::CommitError)) {
                return Err(failure());
            }
            Ok(revision)
        })
    }
}

#[test]
fn uncertain_metadata_free_reservation_reloads_before_rejecting_stale_preparation() {
    let store = FaultStore::new(Fault::CommitPending);
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let mut future = session.prompt_prepared("committed input", preparation(5, None));
    pending(&mut future);
    assert!(session.has_active_turn());
    assert!(matches!(
        block_on(session.prompt("busy")),
        Err(EngineError::SessionBusy)
    ));
    assert_eq!(session.record(), record());
    drop(future);
    assert!(!session.has_active_turn());
    let error =
        block_on(session.prompt_prepared("must not duplicate", preparation(5, None))).unwrap_err();
    assert!(
        matches!(error, EngineError::Store(error) if error.kind == SessionStoreErrorKind::Conflict)
    );
    assert_eq!(store.inner.calls().len(), 3);
    assert!(matches!(
        store.inner.calls()[2],
        RecordedSessionStoreCall::Load { .. }
    ));
    assert_eq!(session.record().next_turn_sequence, 13);
    assert_eq!(
        session.record().messages.last(),
        Some(&Message::text(Role::User, "committed input"))
    );
    let mut prep = preparation(5, None);
    prep.expected_revision = SessionRevision(8);
    let turn = block_on(session.continue_turn_prepared(InferenceOptions::default(), prep)).unwrap();
    assert_eq!(turn.id().as_str(), "turn-13");
    drop(turn);
}

#[test]
fn failed_prepared_saves_force_reload_before_every_kind_of_later_mutation() {
    for fault in [Fault::CommitError, Fault::BadRevision, Fault::Error] {
        for next in 0..3 {
            let store = FaultStore::new(fault);
            let engine = engine(
                store.clone(),
                ScriptedModelProvider::new("unused", []),
                EngineLimits::default(),
            );
            let session = load(&engine);
            let mut prep = preparation(5, None);
            prep.metadata = Some(BTreeMap::from([("new".to_owned(), json!(true))]));
            let error = block_on(session.prompt_prepared("saved maybe", prep)).unwrap_err();
            assert!(!format!("{error:?}").contains("private"));
            let calls_before = store.inner.calls().len();
            match next {
                0 => {
                    drop(block_on(session.prompt("next")).unwrap());
                }
                1 => {
                    drop(block_on(session.continue_turn(InferenceOptions::default())).unwrap());
                }
                _ => {
                    let revision = if matches!(fault, Fault::CommitError) {
                        8
                    } else {
                        7
                    };
                    block_on(session.update_metadata(SessionRevision(revision), BTreeMap::new()))
                        .unwrap();
                }
            }
            assert!(matches!(
                store.inner.calls()[calls_before],
                RecordedSessionStoreCall::Load { .. }
            ));
            if matches!(fault, Fault::CommitError) {
                assert_eq!(
                    session.record().messages[7],
                    Message::text(Role::User, "saved maybe")
                );
                assert!(session.record().next_turn_sequence >= 13);
            }
        }
    }
}

#[test]
fn prepared_conflict_attempts_one_save_and_preserves_external_state() {
    let store = store(record());
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let mut external = record();
    external.metadata.insert("external".to_owned(), json!(true));
    external.next_turn_sequence = 20;
    block_on(store.save(external, Some(SessionRevision(7)))).unwrap();
    let error = block_on(session.prompt_prepared("stale", preparation(5, None))).unwrap_err();
    assert!(
        matches!(error, EngineError::Store(error) if error.kind == SessionStoreErrorKind::Conflict)
    );
    assert_eq!(store.calls().len(), 3); // initial load, external save, single conflicting save
    assert_eq!(session.record(), record());
    let error = block_on(session.prompt_prepared("still stale", preparation(5, None))).unwrap_err();
    assert!(
        matches!(error, EngineError::Store(error) if error.kind == SessionStoreErrorKind::Conflict)
    );
    assert_eq!(store.calls().len(), 4); // mandatory reload; no second prepared save
    assert_eq!(session.record().next_turn_sequence, 20);
    assert_eq!(session.record().metadata["external"], json!(true));
}

#[test]
fn delayed_prepared_success_cannot_rewind_a_newer_canonical_load() {
    let store = FaultStore::new(Fault::CommitDelayed);
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let mut prep = preparation(5, Some("pinned summary"));
    prep.metadata = Some(BTreeMap::from([("prepared".to_owned(), json!(true))]));
    let mut future = session.prompt_prepared("prepared input", prep);
    pending(&mut future);
    let mut external = store.inner.record(&session.id()).unwrap();
    external.metadata.insert("external".to_owned(), json!(true));
    block_on(store.inner.save(external, Some(SessionRevision(8)))).unwrap();
    let newer = load(&engine);
    let expected = newer.record();
    store.release.store(true, Ordering::Release);
    let turn = block_on(future).unwrap();
    assert_eq!(turn.id().as_str(), "turn-12");
    assert_eq!(session.record(), expected);
    assert_eq!(expected.next_turn_sequence, 13);
    assert_eq!(expected.revision, SessionRevision(9));
    drop(turn);
    let call_count = store.inner.calls().len();
    block_on(session.update_metadata(SessionRevision(9), expected.metadata)).unwrap();
    assert!(matches!(
        store.inner.calls()[call_count],
        RecordedSessionStoreCall::Save { .. }
    ));
}

#[test]
fn equal_revision_divergent_prepared_success_keeps_reconciliation_armed() {
    let store = FaultStore::new(Fault::CommitDelayed);
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let mut future =
        session.continue_turn_prepared(InferenceOptions::default(), preparation(5, None));
    pending(&mut future);
    let mut divergent = store.inner.record(&session.id()).unwrap();
    divergent
        .metadata
        .insert("divergent".to_owned(), json!(true));
    store
        .loads
        .lock()
        .unwrap()
        .push_back(Some(divergent.clone()));
    drop(load(&engine));
    store.release.store(true, Ordering::Release);
    assert!(matches!(block_on(future), Err(EngineError::Protocol(_))));
    assert_eq!(session.record(), divergent);
    let call_count = store.inner.calls().len();
    assert!(matches!(
        block_on(session.prompt("must not save")),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.inner.calls().len(), call_count + 1);
    assert!(matches!(
        store.inner.calls()[call_count],
        RecordedSessionStoreCall::Load { .. }
    ));
}

#[test]
fn corrupt_missing_or_nonmonotonic_uncertainty_reload_keeps_writes_blocked() {
    let mut bad_records = vec![None];
    for field in 0..5 {
        let mut bad = record();
        match field {
            0 => bad.revision = SessionRevision(0),
            1 => bad.next_turn_sequence = 0,
            2 => bad.incarnation_id = SessionIncarnationId::new("other").unwrap(),
            3 => bad.revision = SessionRevision(6),
            _ => {
                bad.revision = SessionRevision(8);
                bad.next_turn_sequence = 11;
            }
        }
        bad_records.push(Some(bad));
    }
    for bad in bad_records {
        let store = FaultStore::new(Fault::CommitPending);
        let engine = engine(
            store.clone(),
            ScriptedModelProvider::new("unused", []),
            EngineLimits::default(),
        );
        let session = load(&engine);
        let mut future =
            session.continue_turn_prepared(InferenceOptions::default(), preparation(5, None));
        pending(&mut future);
        drop(future);
        store.loads.lock().unwrap().extend([bad.clone(), bad]);
        for _ in 0..2 {
            assert!(block_on(session.prompt("blocked")).is_err());
            assert_eq!(session.record(), record());
            assert!(!session.has_active_turn());
        }
        assert_eq!(store.inner.calls().len(), 4);
        drop(block_on(session.continue_turn(InferenceOptions::default())).unwrap());
        assert_eq!(session.record().next_turn_sequence, 14);
    }
}

#[test]
fn summary_and_metadata_debug_are_redacted() {
    let mut prep = preparation(5, Some("secret summary"));
    prep.metadata = Some(BTreeMap::from([(
        "secret key".to_owned(),
        json!("secret value"),
    )]));
    let debug = format!("{prep:?}");
    assert!(!debug.contains("secret"));
    assert!(debug.contains("[redacted]"));
}

#[test]
fn public_summary_and_projected_byte_limits_are_checked_before_save() {
    for (summary, byte_limit) in [
        ("x".repeat(MAX_CONTEXT_SUMMARY_BYTES + 1), 100_000),
        ("x".repeat(2_000), 1_500),
    ] {
        let store = store(record());
        let limits = EngineLimits {
            max_transcript_bytes: NonZeroUsize::new(byte_limit).unwrap(),
            ..EngineLimits::default()
        };
        let engine = engine(
            store.clone(),
            ScriptedModelProvider::new("unused", []),
            limits,
        );
        let session = load(&engine);
        assert!(matches!(
            block_on(session.continue_turn_prepared(
                InferenceOptions::default(),
                preparation(5, Some(&summary))
            )),
            Err(EngineError::Protocol(_))
        ));
        assert_eq!(store.calls().len(), 1);
    }
    let store = store(record());
    let engine = engine(
        store,
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = load(&engine);
    drop(
        block_on(session.continue_turn_prepared(
            InferenceOptions::default(),
            preparation(5, Some(&"x".repeat(MAX_CONTEXT_SUMMARY_BYTES))),
        ))
        .unwrap(),
    );
}

#[test]
fn projection_does_not_bypass_canonical_message_limits() {
    let store = store(record());
    let limits = EngineLimits {
        max_transcript_messages: NonZeroUsize::new(7).unwrap(),
        ..EngineLimits::default()
    };
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        limits,
    );
    let session = load(&engine);
    assert!(matches!(
        block_on(session.prompt_prepared("one too many", preparation(5, None))),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.calls().len(), 1);
    drop(
        block_on(session.continue_turn_prepared(InferenceOptions::default(), preparation(5, None)))
            .unwrap(),
    );
}

#[test]
fn added_summary_message_counts_against_the_projected_message_limit() {
    let store = store(record());
    let limits = EngineLimits {
        max_transcript_messages: NonZeroUsize::new(7).unwrap(),
        ..EngineLimits::default()
    };
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        limits,
    );
    let session = load(&engine);
    // Index one preserves all non-system messages, so a summary adds one slot.
    assert!(matches!(
        block_on(
            session.continue_turn_prepared(
                InferenceOptions::default(),
                preparation(1, Some("summary"))
            )
        ),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(store.calls().len(), 1);
    drop(
        block_on(session.continue_turn_prepared(InferenceOptions::default(), preparation(1, None)))
            .unwrap(),
    );
}

#[test]
fn metadata_bounds_are_independent_of_the_smaller_provider_projection() {
    for (metadata, limits) in [
        (
            BTreeMap::from([("large".to_owned(), json!("x".repeat(500)))]),
            EngineLimits {
                max_session_metadata_bytes: NonZeroUsize::new(100).unwrap(),
                ..EngineLimits::default()
            },
        ),
        (
            BTreeMap::from([("nodes".to_owned(), json!(vec![0; 100]))]),
            EngineLimits {
                max_json_nodes: NonZeroUsize::new(40).unwrap(),
                ..EngineLimits::default()
            },
        ),
    ] {
        let store = store(record());
        let engine = engine(
            store.clone(),
            ScriptedModelProvider::new("unused", []),
            limits,
        );
        let session = load(&engine);
        let mut prep = preparation(5, None);
        prep.metadata = Some(metadata);
        assert!(matches!(
            block_on(session.continue_turn_prepared(InferenceOptions::default(), prep)),
            Err(EngineError::Protocol(_))
        ));
        assert_eq!(store.calls().len(), 1);
        assert_eq!(session.record(), record());
    }
}

#[test]
fn prepared_admission_checks_revision_after_uncertain_metadata_edit_reload() {
    let store = FaultStore::new(Fault::CommitPending);
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let metadata = BTreeMap::from([("edited".to_owned(), json!(true))]);
    let mut edit = session.update_metadata(SessionRevision(7), metadata.clone());
    pending(&mut edit);
    drop(edit);
    assert!(matches!(
        block_on(session.prompt_prepared("stale", preparation(5, None))),
        Err(EngineError::Store(_))
    ));
    assert_eq!(session.record().metadata, metadata);
    assert_eq!(session.record().messages, record().messages);
    assert_eq!(session.record().next_turn_sequence, 12);
    assert_eq!(store.inner.calls().len(), 3);
    let mut prep = preparation(5, None);
    prep.expected_revision = SessionRevision(8);
    prep.metadata = Some(BTreeMap::new());
    drop(block_on(session.continue_turn_prepared(InferenceOptions::default(), prep)).unwrap());
    assert!(session.record().metadata.is_empty());
    assert_eq!(session.record().next_turn_sequence, 13);
}

#[test]
fn prepared_allocator_exhaustion_fails_without_publishing_metadata_or_input() {
    let mut original = record();
    original.next_turn_sequence = u64::MAX;
    let store = store(original.clone());
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let mut prep = preparation(5, None);
    prep.metadata = Some(BTreeMap::new());
    assert!(matches!(
        block_on(session.prompt_prepared("not saved", prep)),
        Err(EngineError::Protocol(_))
    ));
    assert_eq!(session.record(), original);
    assert_eq!(store.calls().len(), 1);
}

fn deep_metadata() -> BTreeMap<String, Value> {
    let value = (0..20_000).fold(Value::Null, |value, _| Value::Array(vec![value]));
    BTreeMap::from([("deep".to_owned(), value)])
}

#[test]
fn unpolled_busy_stale_and_rejected_preparation_drain_untrusted_json_iteratively() {
    let store = store(record());
    let engine = engine(
        store.clone(),
        ScriptedModelProvider::new("unused", []),
        EngineLimits::default(),
    );
    let session = load(&engine);
    let hostile = |revision| SessionTurnPreparation {
        expected_revision: SessionRevision(revision),
        metadata: Some(deep_metadata()),
        context: None,
    };
    drop(session.prompt_prepared("inert", hostile(7)));
    assert!(matches!(
        block_on(session.prompt_prepared("invalid", hostile(7))),
        Err(EngineError::Protocol(_))
    ));
    assert!(matches!(
        block_on(session.continue_turn_prepared(InferenceOptions::default(), hostile(6))),
        Err(EngineError::Store(_))
    ));
    let turn = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
    assert!(matches!(
        block_on(session.prompt_prepared("busy", hostile(8))),
        Err(EngineError::SessionBusy)
    ));
    drop(turn);
    assert_eq!(store.calls().len(), 2);
}

#[test]
fn dropping_a_pending_save_releases_the_lease_without_starting_provider_work() {
    let store = InMemorySessionStore::configured(
        BTreeMap::from([(record().id, record())]),
        SessionStoreScript {
            saves: Some(vec![SessionStoreStep::Pending, SessionStoreStep::Pass]),
            ..SessionStoreScript::default()
        },
        100,
    );
    let provider = ScriptedModelProvider::new("unused", []);
    let engine = engine(store.clone(), provider.clone(), EngineLimits::default());
    let session = load(&engine);
    let mut future = session.prompt_prepared("pending", preparation(5, None));
    pending(&mut future);
    assert!(session.has_active_turn());
    drop(future);
    assert!(!session.has_active_turn());
    assert!(provider.requests().is_empty());
    let turn =
        block_on(session.continue_turn_prepared(InferenceOptions::default(), preparation(5, None)))
            .unwrap();
    assert_eq!(turn.id().as_str(), "turn-12");
    assert_eq!(store.calls().len(), 4);
    drop(turn);
}

#[test]
fn prepared_future_does_not_retain_host_authority() {
    struct Resource(Arc<AtomicBool>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    for continuation in [false, true] {
        let dropped = Arc::new(AtomicBool::new(false));
        let store = store(record());
        let engine = Engine::builder()
            .session_store(store.clone())
            .provider(ScriptedModelProvider::new("unused", []))
            .permission_handler(ScriptedPermissionHandler::new([]))
            .host_resource(Resource(Arc::clone(&dropped)))
            .build()
            .unwrap();
        let session = load(&engine);
        let mut prep = preparation(5, None);
        prep.metadata = Some(deep_metadata());
        let future = if continuation {
            session.continue_turn_prepared(InferenceOptions::default(), prep)
        } else {
            session.prompt_prepared("new", prep)
        };
        drop(session);
        drop(engine);
        assert!(dropped.load(Ordering::Acquire));
        assert!(matches!(block_on(future), Err(EngineError::HostClosed)));
        assert_eq!(store.calls().len(), 1);
    }
}

struct ArchiveReader(InMemorySessionStore);

impl Tool for ArchiveReader {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("read_tool_result").unwrap(),
            description: "read canonical archive fixture".to_owned(),
            input_schema: json!({"type": "object"}),
        }
    }

    fn execute(
        &self,
        context: ToolContext,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            let canonical = self.0.record(&context.session_id).unwrap();
            assert_eq!(canonical.messages[..7], record().messages);
            let ContentBlock::ToolResult { output, .. } = &canonical.messages[3].content[0] else {
                panic!("full archive remains available")
            };
            Ok(output.clone())
        })
    }
}

#[test]
fn multiple_rounds_use_pinned_projection_while_archive_reader_sees_full_canonical_evidence() {
    let store = store(record());
    let provider = ScriptedModelProvider::new(
        "archive",
        [
            ModelProviderStep::events([
                ModelEvent::ToolCall {
                    call: call("fresh"),
                },
                ModelEvent::Stop {
                    reason: StopReason::ToolCalls,
                },
            ]),
            finish(),
        ],
    );
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Turn,
        })]);
    let engine = Engine::builder()
        .session_store(store.clone())
        .provider(provider.clone())
        .permission_handler(policy.clone())
        .tool(ArchiveReader(store.clone()))
        .build()
        .unwrap();
    let session = load(&engine);
    complete(
        block_on(session.continue_turn_prepared(
            InferenceOptions::default(),
            preparation(5, Some("not evidence")),
        ))
        .unwrap(),
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request.messages.len(), 4);
    assert_eq!(
        requests[1].request.messages[..4],
        requests[0].request.messages
    );
    assert_eq!(requests[1].request.messages[4], calls(&["fresh"]));
    assert_eq!(requests[1].request.messages[5], result("fresh", false));
    assert_eq!(policy.requests().len(), 1); // old known/unknown calls were not replayed
    assert_eq!(session.record().messages[..7], record().messages);
    assert_eq!(
        session.record().messages[9],
        Message::text(Role::Assistant, "new answer")
    );
}

#[test]
fn live_prepared_turn_cancellation_and_drop_keep_reservation_and_release_lease() {
    for drop_turn in [false, true] {
        let store = store(record());
        let provider = ScriptedModelProvider::new("pending", [ModelProviderStep::pending()]);
        let engine = engine(store, provider.clone(), EngineLimits::default());
        let session = load(&engine);
        let mut turn = block_on(session.continue_turn_prepared(
            InferenceOptions::default(),
            preparation(5, Some("advisory")),
        ))
        .unwrap();
        assert!(matches!(
            block_on(turn.next()).unwrap().unwrap().payload,
            TurnEvent::Started
        ));
        let mut next: BoxFuture<'_, _> = Box::pin(turn.next());
        pending(&mut next);
        drop(next);
        let handle = turn.handle();
        if drop_turn {
            drop(turn);
        } else {
            assert!(handle.cancel());
            let events = block_on(turn.collect::<Vec<_>>());
            assert!(matches!(
                events.last().unwrap().as_ref().unwrap().payload,
                TurnEvent::Completed {
                    reason: StopReason::Cancelled,
                    ..
                }
            ));
        }
        assert!(!handle.cancel());
        assert!(provider.requests()[0].cancellation.is_cancelled());
        assert!(!session.has_active_turn());
        assert_eq!(session.record().messages, record().messages);
        assert_eq!(session.record().next_turn_sequence, 13);
        let next = block_on(session.continue_turn(InferenceOptions::default())).unwrap();
        assert_eq!(next.id().as_str(), "turn-13");
        drop(next);
    }
}

#[test]
fn projected_byte_limit_is_rechecked_before_the_next_model_round() {
    let summary = "x".repeat(2_000);
    let probe_store = store(record());
    let probe_provider = ScriptedModelProvider::new("measure", [finish()]);
    let probe = engine(probe_store, probe_provider.clone(), EngineLimits::default());
    complete(
        block_on(
            load(&probe).continue_turn_prepared(
                InferenceOptions::default(),
                preparation(5, Some(&summary)),
            ),
        )
        .unwrap(),
    );
    let initial_bytes = serde_json::to_vec(&probe_provider.requests()[0].request.messages)
        .unwrap()
        .len();
    let store = store(record());
    let provider = ScriptedModelProvider::new(
        "bounded",
        [ModelProviderStep::events([
            ModelEvent::ToolCall {
                call: call("fresh"),
            },
            ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            },
        ])],
    );
    let policy =
        ScriptedPermissionHandler::new([PermissionStep::Decision(PermissionDecision::Allow {
            scope: PermissionGrantScope::Turn,
        })]);
    let limits = EngineLimits {
        max_transcript_bytes: NonZeroUsize::new(initial_bytes).unwrap(),
        ..EngineLimits::default()
    };
    let engine = Engine::builder()
        .session_store(store.clone())
        .provider(provider.clone())
        .permission_handler(policy)
        .tool(ArchiveReader(store))
        .limits(limits)
        .build()
        .unwrap();
    let session = load(&engine);
    let turn = block_on(
        session.continue_turn_prepared(InferenceOptions::default(), preparation(5, Some(&summary))),
    )
    .unwrap();
    let events = block_on(turn.collect::<Vec<_>>());
    assert!(events.iter().any(|event| matches!(&event.as_ref().unwrap().payload, TurnEvent::Failed { code, .. } if code == "context_projection_failed")));
    assert_eq!(provider.requests().len(), 1);
    // The new call and confirmed result committed canonically before the next
    // projection overflow; neither is dropped or converted to unknown evidence.
    assert_eq!(session.record().messages[7], calls(&["fresh"]));
    assert_eq!(session.record().messages[8], result("fresh", false));
}
