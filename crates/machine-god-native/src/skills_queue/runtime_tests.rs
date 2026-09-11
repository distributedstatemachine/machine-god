//! Exact queue/admission tests use in-memory core ownership and native file reads.
use super::*;
use crate::skills_queue::NativeSkillsQueueError;
use crate::{
    NATIVE_SKILL_PROMPT_CONTEXT_KEY, NativeOwnedWorkerScope, NativeSkillCatalog,
    NativeSkillLinkPolicy, NativeSkillRoot, NativeSkillSelection, NativeSkillSnapshot,
    NativeSkillSource,
};
use futures_executor::block_on;
use futures_util::StreamExt;
use machine_god_core::{Engine, Message, ModelEvent, Role, StopReason};
use machine_god_testkit::{
    InMemorySessionStore, ModelProviderStep, ScriptedModelProvider, ScriptedPermissionHandler,
};
use std::{
    fs,
    future::Future,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    task::Waker,
    time::Duration,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    directory: PathBuf,
    catalog: Arc<NativeSkillCatalog>,
    workers: NativeOwnedWorkerScope,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "mg-skills-queue-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::create_dir(directory.join("skills")).unwrap();
        let root = NativeSkillRoot::from_directory(
            Arc::new(fs::File::open(&directory).unwrap()),
            "skills".into(),
            directory.clone(),
            directory.join("skills"),
            NativeSkillSource::WorkspaceShared,
            NativeSkillLinkPolicy::Reject,
        )
        .unwrap();
        Self {
            directory,
            catalog: Arc::new(NativeSkillCatalog::new(vec![root]).unwrap()),
            workers: NativeOwnedWorkerScope::new(),
        }
    }
    fn skill(&self, folder: &str, name: &str, body: &str) {
        let directory = self.directory.join("skills").join(folder);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\n{body}"),
        )
        .unwrap();
    }
    fn snapshot(&self) -> NativeSkillSnapshot {
        self.catalog.discover(&CancellationToken::new()).unwrap()
    }
    fn enqueue(
        &self,
        runtime: &NativeConversationRuntime,
        prompt: &str,
        snapshot: &NativeSkillSnapshot,
        explicit: &[NativeSkillSelection],
    ) -> crate::skills_queue::NativeQueuedSkillsReceipt {
        runtime
            .enqueue_with_skills(
                prompt.into(),
                self.catalog.clone(),
                snapshot,
                explicit,
                self.workers.clone(),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.workers.close();
        let _ = self.workers.completion().wait_on_worker();
        let result = fs::remove_dir_all(&self.directory);
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}
fn setup(
    steps: impl IntoIterator<Item = ModelProviderStep>,
) -> (
    Arc<NativeConversationRuntime>,
    Arc<InMemorySessionStore>,
    ScriptedModelProvider,
) {
    let provider = ScriptedModelProvider::new("skills-queue", steps);
    let store = Arc::new(InMemorySessionStore::default());
    let runtime = runtime_with_store(store.clone(), provider.clone());
    (runtime, store, provider)
}
fn runtime_with_store(
    store: Arc<dyn machine_god_core::SessionStore>,
    provider: ScriptedModelProvider,
) -> Arc<NativeConversationRuntime> {
    let engine = Engine::builder()
        .shared_session_store(store)
        .provider(provider)
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("skill-queue").unwrap(),
            SessionIncarnationId::new("exact-life").unwrap(),
        )
        .unwrap();
    Arc::new(
        NativeConversationRuntime::new(
            NativeConversation::from_session(session).unwrap(),
            NativeModelPreferences::new(
                "test/model",
                crate::NativeReasoningEffort::default(),
                false,
            )
            .unwrap(),
            None,
        )
        .unwrap(),
    )
}
fn finished() -> ModelProviderStep {
    ModelProviderStep::events([
        ModelEvent::TextDelta {
            text: "answer".into(),
        },
        ModelEvent::Stop {
            reason: StopReason::Completed,
        },
    ])
}
fn complete(turn: NativeConversationRuntimeTurn) {
    for event in block_on(turn.collect::<Vec<_>>()) {
        event.unwrap();
    }
}

#[test]
fn enqueue_and_unpolled_start_are_inert_and_fifo_pins_exact_skills() {
    let fixture = Fixture::new();
    fixture.skill("one", "one", "FIRST");
    fixture.skill("two", "two", "SECOND");
    let snapshot = fixture.snapshot();
    let (runtime, store, provider) = setup([finished(), finished()]);
    let one = fixture.enqueue(&runtime, "$one help", &snapshot, &[]);
    let two = fixture.enqueue(&runtime, "$two help", &snapshot, &[]);
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
    assert!(one.queued_id.get() < two.queued_id.get());
    let future = runtime.start_next(100);
    drop(future);
    assert_eq!(runtime.status().queued_jobs, 2);
    let turn = block_on(runtime.start_next(100)).unwrap().unwrap();
    assert_eq!(turn.queued_id(), one.queued_id);
    assert!(
        runtime.record().metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY]["text"]
            .as_str()
            .unwrap()
            .contains("FIRST")
    );
    assert_eq!(
        runtime.record().messages,
        [Message::text(Role::User, "$one help")]
    );
    complete(turn);
    let turn = block_on(runtime.start_next(200)).unwrap().unwrap();
    assert_eq!(turn.queued_id(), two.queued_id);
    let text = runtime.record().metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.contains("SECOND"));
    assert!(!text.contains("FIRST"));
    complete(turn);
    assert!(provider.requests()[0].request.messages[0].content.len() > 1);
    assert!(
        !runtime
            .record()
            .metadata
            .contains_key(NATIVE_SKILL_PROMPT_CONTEXT_KEY)
    );
}

#[test]
fn source_changed_after_enqueue_fails_without_provider_save_or_requeue() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "original");
    let snapshot = fixture.snapshot();
    let (runtime, store, provider) = setup([]);
    fixture.enqueue(&runtime, "$selected help", &snapshot, &[]);
    fixture.skill("selected", "selected", "changed larger body");
    assert!(matches!(
        block_on(runtime.start_next(100)),
        Err(NativeConversationRuntimeError::Skills(
            NativeSkillsQueueError::Catalog {
                error: crate::NativeSkillCatalogError::StaleSelection,
                ..
            }
        ))
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
    assert_eq!(runtime.status().queued_jobs, 0);
    assert!(!runtime.status().active);
}

#[test]
fn duplicate_names_require_exact_choice_and_foreign_catalog_rejects_at_enqueue() {
    let fixture = Fixture::new();
    fixture.skill("one", "same", "FIRST");
    fixture.skill("two", "same", "SECOND");
    let snapshot = fixture.snapshot();
    let selected = snapshot
        .entries()
        .iter()
        .find(|entry| entry.location().ends_with("two"))
        .unwrap()
        .selection();
    let (runtime, _, _) = setup([]);
    fixture.enqueue(
        &runtime,
        "$same help",
        &snapshot,
        std::slice::from_ref(&selected),
    );
    let turn = block_on(runtime.start_next(100)).unwrap().unwrap();
    let text = runtime.record().metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.contains("SECOND"));
    assert!(!text.contains("FIRST"));
    drop(turn);
    let foreign = Fixture::new();
    assert!(matches!(
        runtime.enqueue_with_skills(
            "text".into(),
            foreign.catalog.clone(),
            &snapshot,
            &[selected],
            fixture.workers.clone()
        ),
        Err(NativeConversationRuntimeError::Skills(
            NativeSkillsQueueError::Catalog {
                error: crate::NativeSkillCatalogError::WrongAuthority,
                ..
            }
        ))
    ));
}

#[test]
fn incomplete_snapshot_is_reported_and_does_not_automatically_invoke() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    fs::create_dir(fixture.directory.join("skills/malformed")).unwrap();
    fs::write(
        fixture.directory.join("skills/malformed/SKILL.md"),
        "---\nname: \n---\n",
    )
    .unwrap();
    let snapshot = fixture.snapshot();
    assert!(!snapshot.complete());
    let (runtime, _, _) = setup([]);
    assert!(
        fixture
            .enqueue(&runtime, "$selected", &snapshot, &[])
            .automatic_matching_incomplete
    );
    drop(block_on(runtime.start_next(100)).unwrap().unwrap());
    assert!(
        !runtime
            .record()
            .metadata
            .contains_key(NATIVE_SKILL_PROMPT_CONTEXT_KEY)
    );
}

#[test]
fn selection_bytes_are_charged_and_queue_overflow_does_not_evict() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    let snapshot = fixture.snapshot();
    let selected = snapshot.entries()[0].selection();
    let (runtime, _, _) = setup([]);
    let raw = "$selected";
    let expected = input_bytes(&PendingInput::new(ConversationInput::Prompt(raw.into()))).unwrap()
        + selected.retained_bytes();
    let receipt = fixture.enqueue(&runtime, raw, &snapshot, &[]);
    assert_eq!(runtime.status().queued_input_bytes, expected);
    assert!(runtime.cancel_queued(receipt.queued_id));
    assert_eq!(runtime.status().queued_input_bytes, 0);
    let prompt = "x".repeat(MAX_NATIVE_QUEUED_PROMPT_BYTES);
    let mut count = 0;
    loop {
        match runtime.enqueue_with_skills(
            prompt.clone().into(),
            fixture.catalog.clone(),
            &snapshot,
            std::slice::from_ref(&selected),
            fixture.workers.clone(),
        ) {
            Ok(_) => count += 1,
            Err(NativeConversationRuntimeError::QueueLimit) => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(count < MAX_NATIVE_QUEUED_JOBS);
    assert_eq!(runtime.status().queued_jobs, count);
    assert_eq!(runtime.clear_queued(), count);
    assert_eq!(runtime.status().queued_input_bytes, 0);
}

#[test]
fn context_limit_counts_advisory_framing_and_preserves_exact_text() {
    let fixture = Fixture::new();
    fixture.skill(
        "selected",
        "selected",
        &"x".repeat(machine_god_core::MAX_SESSION_USER_CONTEXT_BYTES),
    );
    let snapshot = fixture.snapshot();
    let (runtime, store, provider) = setup([]);
    fixture.enqueue(&runtime, "$selected", &snapshot, &[]);
    assert!(matches!(
        block_on(runtime.start_next(100)),
        Err(NativeConversationRuntimeError::Skills(
            NativeSkillsQueueError::ContextLimit
        ))
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
}

#[test]
fn continuation_reuses_admitted_bytes_after_source_removal_and_new_prompt_clears_context() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "saved exact body\0日本語");
    let snapshot = fixture.snapshot();
    let (runtime, _, provider) = setup([finished(), finished()]);
    fixture.enqueue(&runtime, "$selected", &snapshot, &[]);
    drop(block_on(runtime.start_next(100)).unwrap().unwrap());
    let original = runtime.record().metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::remove_dir_all(fixture.directory.join("skills/selected")).unwrap();
    runtime
        .enqueue_continuation(InferenceOptions::default())
        .unwrap();
    let turn = block_on(runtime.start_next(200)).unwrap().unwrap();
    assert_eq!(
        runtime.record().metadata[NATIVE_SKILL_PROMPT_CONTEXT_KEY]["text"],
        original
    );
    assert_eq!(runtime.record().messages.len(), 1);
    complete(turn);
    assert!(format!("{:?}", provider.requests()[0].request.messages).contains("saved exact body"));
    runtime.enqueue("ordinary prompt".into()).unwrap();
    let turn = block_on(runtime.start_next(300)).unwrap().unwrap();
    assert!(
        !runtime
            .record()
            .metadata
            .contains_key(NATIVE_SKILL_PROMPT_CONTEXT_KEY)
    );
    complete(turn);
}

fn hook(
    runtime: &NativeConversationRuntime,
    operation: impl FnOnce(&CancellationToken) + Send + 'static,
) {
    runtime
        .state
        .lock()
        .unwrap()
        .queue
        .back_mut()
        .unwrap()
        .skills
        .as_mut()
        .unwrap()
        .set_test_hook(operation);
}

#[test]
fn materialization_is_outside_runtime_lock_and_keeps_taken_model_selection() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    let (runtime, _, _) = setup([]);
    fixture.enqueue(&runtime, "$selected", &fixture.snapshot(), &[]);
    let selected = runtime.clone();
    hook(&runtime, move |_| {
        assert!(selected.status().active);
        selected
            .set_model_preferences(
                NativeModelPreferences::new(
                    "test/later",
                    crate::NativeReasoningEffort::default(),
                    false,
                )
                .unwrap(),
            )
            .unwrap();
    });
    let turn = block_on(runtime.start_next(100)).unwrap().unwrap();
    assert_eq!(turn.model_snapshot().preferences().model(), "test/model");
    assert_eq!(runtime.model_preferences().model(), "test/later");
    drop(turn);
}

#[test]
fn dropped_materialization_retains_runtime_lease_until_worker_finishes() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    let (runtime, store, provider) = setup([]);
    fixture.enqueue(&runtime, "$selected", &fixture.snapshot(), &[]);
    let (entered, observe) = std::sync::mpsc::sync_channel(1);
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    hook(&runtime, move |token| {
        assert!(NativeOwnedWorkerScope::retain_current_cleanup().is_some());
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(token.is_cancelled());
    });
    runtime.enqueue("still queued".into()).unwrap();
    // Hook belongs to the selected first job rather than the trailing plain job.
    let mut future = runtime.start_next(100);
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    observe.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut fence = runtime.begin_quiescence().unwrap();
    drop(future);
    assert!(runtime.status().active);
    assert!(fence.try_retire().is_err());
    fixture.workers.close();
    assert!(!fixture.workers.completion().is_complete());
    release.send(()).unwrap();
    fixture.workers.completion().wait_on_worker().unwrap();
    assert!(!runtime.status().active);
    assert_eq!(runtime.status().queued_jobs, 1);
    assert!(fence.try_retire().is_ok());
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
}

#[test]
fn explicit_pre_handle_cancel_stops_reads_without_discarding_later_queue() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    let (runtime, store, provider) = setup([finished()]);
    fixture.enqueue(&runtime, "$selected", &fixture.snapshot(), &[]);
    let (entered, observe) = std::sync::mpsc::sync_channel(1);
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    hook(&runtime, move |_| {
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    runtime.enqueue("later".into()).unwrap();
    let mut future = runtime.start_next(100);
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    observe.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(runtime.request_active_cancel());
    release.send(()).unwrap();
    assert!(matches!(
        block_on(future),
        Err(NativeConversationRuntimeError::Skills(
            NativeSkillsQueueError::Cancelled
        ))
    ));
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
    assert_eq!(runtime.status().queued_jobs, 1);
    complete(block_on(runtime.start_next(200)).unwrap().unwrap());
    assert_eq!(provider.requests().len(), 1);
}

#[test]
fn closed_worker_scope_rejects_before_source_reads_or_core_admission() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    let (runtime, store, provider) = setup([]);
    fixture.enqueue(&runtime, "$selected", &fixture.snapshot(), &[]);
    hook(&runtime, |_| panic!("closed scope cannot read"));
    fixture.workers.close();
    assert!(matches!(
        block_on(runtime.start_next(100)),
        Err(NativeConversationRuntimeError::Skills(
            NativeSkillsQueueError::WorkerUnavailable
        ))
    ));
    assert!(!runtime.status().active);
    assert!(store.calls().is_empty());
    assert!(provider.requests().is_empty());
}

#[test]
fn response_failure_cancels_running_worker_without_releasing_its_runtime_lease() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    let (runtime, _, _) = setup([]);
    fixture.enqueue(&runtime, "$selected", &fixture.snapshot(), &[]);
    let (entered, observe) = std::sync::mpsc::sync_channel(1);
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    hook(&runtime, move |token| {
        entered.send(token.clone()).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    let mut future = runtime.start_next(100);
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let token = observe.recv_timeout(Duration::from_secs(10)).unwrap();
    let (waker, _) = machine_god_reentrant_waker_test::new(
        machine_god_reentrant_waker_test::Callback::Clone,
        || panic!("injected waker clone"),
    );
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Ready(Err(NativeConversationRuntimeError::Skills(
            NativeSkillsQueueError::WorkerUnavailable
        )))
    ));
    assert!(token.is_cancelled());
    assert!(runtime.status().active);
    let mut fence = runtime.begin_quiescence().unwrap();
    assert!(fence.try_retire().is_err());
    release.send(()).unwrap();
    fixture.workers.close();
    fixture.workers.completion().wait_on_worker().unwrap();
    assert!(fence.try_retire().is_ok());
}

#[test]
fn pre_handle_cancellation_callbacks_reenter_runtime_outside_state_lock() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    let (runtime, _, _) = setup([]);
    fixture.enqueue(&runtime, "$selected", &fixture.snapshot(), &[]);
    let (entered, observe) = std::sync::mpsc::sync_channel(1);
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    hook(&runtime, move |token| {
        entered.send(token.clone()).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    let mut future = runtime.start_next(100);
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let token = observe.recv_timeout(Duration::from_secs(10)).unwrap();
    let reentered = runtime.clone();
    let (waker, observer) = machine_god_reentrant_waker_test::new(
        machine_god_reentrant_waker_test::Callback::Wake,
        move || {
            assert!(reentered.status().active);
        },
    );
    let mut cancellation = Box::pin(token.cancelled());
    assert!(
        cancellation
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert!(runtime.request_active_cancel());
    assert!(observer.calls() > 0);
    release.send(()).unwrap();
    assert!(matches!(
        block_on(future),
        Err(NativeConversationRuntimeError::Skills(
            NativeSkillsQueueError::Cancelled
        ))
    ));
}

#[derive(Default)]
struct SaveGate {
    entered: std::sync::atomic::AtomicBool,
    released: std::sync::atomic::AtomicBool,
    wake: futures_util::task::AtomicWaker,
}
struct GatedStore {
    inner: InMemorySessionStore,
    gate: Arc<SaveGate>,
}
impl machine_god_core::SessionStore for GatedStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, machine_god_core::SessionStoreError>> {
        self.inner.load(id)
    }
    fn save(
        &self,
        record: SessionRecord,
        expected: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, machine_god_core::SessionStoreError>> {
        Box::pin(async move {
            self.gate.entered.store(true, Ordering::SeqCst);
            futures_util::future::poll_fn(|cx| {
                self.gate.wake.register(cx.waker());
                if self.gate.released.load(Ordering::SeqCst) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await;
            self.inner.save(record, expected).await
        })
    }
}

#[test]
fn cancellation_after_reads_during_reservation_reaches_later_core_handle() {
    let fixture = Fixture::new();
    fixture.skill("selected", "selected", "body");
    let gate = Arc::new(SaveGate::default());
    let store = Arc::new(GatedStore {
        inner: InMemorySessionStore::default(),
        gate: gate.clone(),
    });
    let provider = ScriptedModelProvider::new("skills-queue", []);
    let runtime = runtime_with_store(store, provider.clone());
    fixture.enqueue(&runtime, "$selected", &fixture.snapshot(), &[]);
    let mut future = runtime.start_next(100);
    block_on(futures_util::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        if gate.entered.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }));
    assert!(runtime.request_active_cancel());
    assert!(runtime.state.lock().unwrap().active_handle.is_none());
    gate.released.store(true, Ordering::SeqCst);
    gate.wake.wake();
    let turn = block_on(future).unwrap().unwrap();
    assert!(turn.handle().unwrap().is_cancelled());
    complete(turn);
    assert!(provider.requests().is_empty());
    assert!(
        runtime
            .record()
            .metadata
            .contains_key(NATIVE_SKILL_PROMPT_CONTEXT_KEY)
    );
}
