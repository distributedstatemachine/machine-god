use super::{Fixture, close, executor, outcome, owner, support};
use crate::interactive_session::{
    NativeInteractiveControl as Control, NativeInteractiveControlError as ControlError,
    NativeInteractiveControlOutcome, NativeInteractiveControlReceipt as Receipt,
    NativeInteractiveError, NativeInteractiveOutcome, NativeInteractiveSession,
    NativeInteractiveTransition,
};
use crate::{
    NATIVE_SESSION_PERMISSION_RULES_KEY, NativeConversation, NativeConversationRuntime,
    NativeConversationRuntimePhase, NativeModelPreferencePersistence, NativeModelPreferences,
    NativePermissionActionPreparer, NativePermissionController, NativePermissionPolicySnapshot,
    NativePermissionRuleChange, NativePermissionRuleDecision, NativePermissionRuleKey,
    NativePermissionRuleKind, NativePermissionRuleProposal, NativePreparedPermissionAction,
    NativeReasoningEffort, NativeResumeTarget, NativeSessionMetadata, NativeSessionPermissionRules,
    NativeUserConfigStore, PermissionMode, PermissionPromptDecision, PermissionPromptError,
    PermissionPrompter,
};
use futures_util::{future::poll_fn, task::AtomicWaker};
use machine_god_core::{
    BoxFuture, CancellationToken, Engine, InferenceOptions, PermissionError, PermissionInvocation,
    PermissionRequest, SessionId, SessionRecord, SessionRevision, SessionStore, SessionStoreError,
    SessionStoreErrorKind,
};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider};
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

#[path = "allowlist_tests.rs"]
mod allowlist;

#[path = "workspace_tests.rs"]
mod workspace;

#[path = "cancellation_tests.rs"]
mod cancellation;

#[path = "skills_composed_tests.rs"]
mod skills_composed;

fn preferences(model: &str) -> NativeModelPreferences {
    NativeModelPreferences::new(model, NativeReasoningEffort::default(), false).unwrap()
}
struct Count(AtomicUsize);
impl std::task::Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn poll_once(owner: &mut NativeInteractiveSession) -> Poll<()> {
    owner.poll_progress(&mut Context::from_waker(Waker::noop()), 300)
}
async fn control_outcome(owner: &mut NativeInteractiveSession) -> NativeInteractiveControlOutcome {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 300);
            owner
                .take_control_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap()
}
async fn finish_turn(owner: &mut NativeInteractiveSession) {
    let terminal = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 300);
            let _ = owner.take_presentation();
            owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .unwrap();
    assert!(matches!(terminal, NativeInteractiveOutcome::Turn(Ok(_))));
}
fn proposal(runtime: &NativeConversationRuntime) -> NativePermissionRuleProposal {
    runtime
        .permissions()
        .unwrap()
        .propose_rule_change(NativePermissionRuleChange::Set {
            key: NativePermissionRuleKey::new(
                NativePermissionRuleKind::StructuredTool,
                "exact-action",
            )
            .unwrap(),
            display_identity: "private-action-description".to_owned(),
            decision: NativePermissionRuleDecision::Allow,
        })
        .unwrap()
}

#[test]
fn composed_rename_is_inert_and_same_session_resume_sees_its_saved_revision() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let current = session.runtime().clone();
        let before = current.record();
        let id = session
            .request_control(
                Control::Rename {
                    title: "  private renamed title\n".into(),
                },
                200,
            )
            .unwrap();
        assert_eq!(current.record(), before);
        session.enqueue("queued after rename".into()).unwrap();
        session
            .request_transition(
                NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(current.id())),
                210,
            )
            .unwrap();
        let result = control_outcome(&mut session).await;
        assert_eq!(result.id, id);
        let Receipt::Renamed(revision) = result.result.unwrap() else {
            panic!("rename receipt");
        };
        assert!(revision > before.revision);
        let NativeInteractiveOutcome::Transition(receipt) = outcome(&mut session).await else {
            panic!("same-session receipt");
        };
        assert!(receipt.unchanged);
        assert!(Arc::ptr_eq(session.runtime(), &current));
        assert_eq!(session.runtime().status().queued_jobs, 1);
        assert_eq!(
            NativeSessionMetadata::from_metadata(&current.record().metadata)
                .unwrap()
                .title(),
            Some("private renamed title")
        );
        assert!(fixture.transport.requests().is_empty());
        drop(current);
        close(session, fixture).await;
    });
}

#[test]
fn composed_compaction_and_model_saves_preserve_transcript_and_exact_target_receipts() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        for prompt in ["first", "second"] {
            fixture.transport.push(support::answer());
            session.enqueue(prompt.into()).unwrap();
            finish_turn(&mut session).await;
        }
        let messages = session.runtime().record().messages;
        session.request_control(Control::Compact, 400).unwrap();
        assert!(matches!(control_outcome(&mut session).await.result, Ok(Receipt::Compacted(true))));
        assert_eq!(session.runtime().record().messages, messages);
        session.request_control(Control::Compact, 401).unwrap();
        assert!(matches!(control_outcome(&mut session).await.result, Ok(Receipt::Compacted(false))));
        let generation = session.set_model_preferences(preferences("selected/session")).unwrap();
        session.request_control(Control::SaveModelSession, 410).unwrap();
        assert!(matches!(control_outcome(&mut session).await.result, Ok(Receipt::ModelSession(NativeModelPreferencePersistence::Saved { generation: saved, .. })) if saved == generation));
        session.request_control(Control::SaveModelSession, 411).unwrap();
        assert!(matches!(control_outcome(&mut session).await.result, Ok(Receipt::ModelSession(NativeModelPreferencePersistence::Unchanged))));
        let store = Arc::new(NativeUserConfigStore::new(fixture.workspace.join("user-defaults")));
        session.request_control(Control::SaveModelDefaults { store: store.clone() }, 420).unwrap();
        let Receipt::ModelDefaults(commit) = control_outcome(&mut session).await.result.unwrap() else { panic!("composite receipt"); };
        assert_eq!(commit.generation, generation);
        assert_eq!(commit.session.unwrap(), NativeModelPreferencePersistence::Unchanged);
        assert_eq!(commit.user_defaults.unwrap().config().model_preferences(), preferences("selected/session"));
        assert_eq!(store.load().unwrap().loaded().config().model_preferences(), preferences("selected/session"));
        assert_eq!(session.runtime().record().messages, messages);
        close(session, fixture).await;
    });
}

#[test]
fn composed_rule_confirmation_and_foreign_proposal_do_not_infer_consent() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let proposed = proposal(session.runtime());
        let before = session.runtime().record();
        session
            .request_control(Control::ConfirmPermissionRule { proposal: proposed }, 200)
            .unwrap();
        assert_eq!(session.runtime().record(), before);
        assert!(matches!(
            control_outcome(&mut session).await.result,
            Ok(Receipt::PermissionRuleConfirmed(_))
        ));
        assert_ne!(session.runtime().record().metadata, before.metadata);
        let proposal = proposal(session.runtime());
        session
            .request_transition(NativeInteractiveTransition::Clear, 210)
            .unwrap();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Transition(_)
        ));
        let before = session.runtime().record();
        session
            .request_control(Control::ConfirmPermissionRule { proposal }, 220)
            .unwrap();
        assert!(matches!(
            control_outcome(&mut session).await.result,
            Err(ControlError::Permission(_))
        ));
        assert_eq!(session.runtime().record(), before);
        close(session, fixture).await;
    });
}

#[derive(Default)]
struct SaveGate {
    armed: AtomicBool,
    release: AtomicBool,
    fail_after_publish: AtomicBool,
    entered: AtomicUsize,
    dropped: AtomicUsize,
    wake: AtomicWaker,
}
impl SaveGate {
    fn open(&self) {
        self.release.store(true, Ordering::SeqCst);
        self.wake.wake();
    }
}
struct SaveProbe(Arc<SaveGate>);
impl Drop for SaveProbe {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
struct GatedStore {
    inner: InMemorySessionStore,
    gate: Arc<SaveGate>,
}
impl SessionStore for GatedStore {
    fn load(
        &self,
        id: SessionId,
    ) -> BoxFuture<'_, Result<Option<SessionRecord>, SessionStoreError>> {
        self.inner.load(id)
    }
    fn save(
        &self,
        record: SessionRecord,
        revision: Option<SessionRevision>,
    ) -> BoxFuture<'_, Result<SessionRevision, SessionStoreError>> {
        Box::pin(async move {
            let selected = self.gate.armed.swap(false, Ordering::SeqCst);
            let probe = selected.then(|| {
                self.gate.entered.fetch_add(1, Ordering::SeqCst);
                SaveProbe(self.gate.clone())
            });
            if selected {
                poll_fn(|cx| {
                    self.gate.wake.register(cx.waker());
                    if self.gate.release.load(Ordering::SeqCst) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
            let saved = self.inner.save(record, revision).await?;
            drop(probe);
            if selected && self.gate.fail_after_publish.load(Ordering::SeqCst) {
                Err(SessionStoreError::new(
                    SessionStoreErrorKind::Unavailable,
                    "control-fixture",
                    "private uncertain publication",
                    false,
                ))
            } else {
                Ok(saved)
            }
        })
    }
}
struct NoEffects;
impl NativePermissionActionPreparer for NoEffects {
    fn prepare<'a>(
        &'a self,
        _: &'a PermissionRequest,
        _: PermissionInvocation<'a>,
        _: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        panic!("no tool preparation");
    }
}
impl PermissionPrompter for NoEffects {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        panic!("no implicit human confirmation");
    }
}

/// Test-only store substitution at the native owner boundary. Operations below
/// remain real Runtime/Conversation/Core/RuleOperation futures, not fake phases.
async fn gated(session: &mut NativeInteractiveSession) -> Arc<GatedStore> {
    let initial = session.runtime().record();
    let store = Arc::new(GatedStore {
        inner: InMemorySessionStore::from_records(BTreeMap::from([(
            initial.id.clone(),
            initial.clone(),
        )])),
        gate: Arc::new(SaveGate::default()),
    });
    let controller = Arc::new(NativePermissionController::new(
        Arc::new(NoEffects),
        Arc::new(NoEffects),
    ));
    let engine = Engine::builder()
        .shared_session_store(store.clone())
        .provider(ScriptedModelProvider::new("control-fixture", []))
        .shared_permission_handler(controller.clone())
        .build()
        .unwrap();
    let core = engine.load_session(initial.id).await.unwrap().unwrap();
    let conversation = NativeConversation::from_session(core)
        .unwrap()
        .with_permission_controller(
            &controller,
            NativePermissionPolicySnapshot::new(PermissionMode::Ask, Arc::default()),
        )
        .unwrap();
    let replacement =
        NativeConversationRuntime::new(conversation, session.runtime().model_preferences(), None)
            .unwrap();
    let mut old = session.current.begin_quiescence().unwrap();
    old.wait_idle().await.unwrap();
    old.retire().unwrap();
    session.current = Arc::new(replacement);
    store
}

#[test]
fn actual_pending_rename_survives_dropped_polls_blocks_queue_and_settles_before_shutdown() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let store = gated(&mut session).await;
        let current = session.current.clone();
        store.gate.armed.store(true, Ordering::SeqCst);
        session.enqueue("must stay queued".into()).unwrap();
        session
            .request_control(
                Control::Rename {
                    title: "settled before shutdown".into(),
                },
                200,
            )
            .unwrap();
        let mut wrapper = Box::pin(poll_fn(|cx| session.poll_progress(cx, 300)));
        assert!(
            wrapper
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(wrapper);
        assert_eq!(store.gate.entered.load(Ordering::SeqCst), 1);
        assert_eq!(store.gate.dropped.load(Ordering::SeqCst), 0);
        session
            .request_transition(NativeInteractiveTransition::Reset, 210)
            .unwrap();
        assert!(poll_once(&mut session).is_pending());
        assert_eq!(current.status().phase, NativeConversationRuntimePhase::Open);
        assert_eq!(current.status().queued_jobs, 1);
        session.request_shutdown();
        assert!(poll_once(&mut session).is_pending());
        store.gate.open();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Shutdown
        ));
        assert!(session.is_closed());
        assert!(matches!(
            session.take_control_outcome().unwrap().result,
            Ok(Receipt::Renamed(_))
        ));
        assert_eq!(store.gate.dropped.load(Ordering::SeqCst), 1);
        // Confirmed retirement discards never-taken input; no provider ran it.
        assert_eq!(current.status().queued_jobs, 0);
        assert_eq!(
            NativeSessionMetadata::from_metadata(
                &store.inner.record(&current.id()).unwrap().metadata
            )
            .unwrap()
            .title(),
            Some("settled before shutdown")
        );
        drop(session);
        fixture.finish();
    });
}

#[test]
fn actual_active_turn_editor_publication_finishes_before_cancel_or_finalization() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let store = gated(&mut session).await;
        session.enqueue("pending model turn".into()).unwrap();
        let turn = session.current.start_next(200).await.unwrap().unwrap();
        let handle = turn.handle().unwrap();
        session.turn = Some(turn);
        let proposal = proposal(session.runtime());
        let before = session.runtime().record();
        store.gate.armed.store(true, Ordering::SeqCst);
        session
            .request_control(Control::ConfirmPermissionRule { proposal }, 210)
            .unwrap();
        assert!(poll_once(&mut session).is_pending());
        session
            .request_transition(NativeInteractiveTransition::Clear, 220)
            .unwrap();
        assert!(poll_once(&mut session).is_pending());
        assert!(!handle.is_cancelled());
        assert_eq!(session.runtime().record(), before);
        session.request_shutdown();
        assert!(poll_once(&mut session).is_pending());
        assert!(!handle.is_cancelled());
        store.gate.open();
        let result = control_outcome(&mut session).await;
        assert!(matches!(
            result.result,
            Ok(Receipt::PermissionRuleConfirmed(_))
        ));
        assert!(handle.is_cancelled());
        let saved = store.inner.record(&session.current.id()).unwrap();
        assert!(
            NativeSessionPermissionRules::from_value(
                &saved.metadata[NATIVE_SESSION_PERMISSION_RULES_KEY]
            )
            .is_ok()
        );
        while !session.is_closed() {
            let _ = outcome(&mut session).await;
        }
        assert_eq!(store.gate.entered.load(Ordering::SeqCst), 1);
        drop(session);
        fixture.finish();
    });
}

#[test]
fn explicit_cancel_waits_for_confirmed_save_without_replacing_session_or_queue() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let store = gated(&mut session).await;
        let id = session.runtime().id();
        assert!(!session.request_cancel());
        session.enqueue("active request".into()).unwrap();
        let turn = session.current.start_next(200).await.unwrap().unwrap();
        let handle = turn.handle().unwrap();
        session.turn = Some(turn);
        session.enqueue("untaken request".into()).unwrap();
        let proposal = proposal(session.runtime());
        store.gate.armed.store(true, Ordering::SeqCst);
        session
            .request_control(Control::ConfirmPermissionRule { proposal }, 210)
            .unwrap();
        assert!(poll_once(&mut session).is_pending());
        assert!(session.request_cancel());
        assert!(poll_once(&mut session).is_pending());
        assert!(!handle.is_cancelled());
        store.gate.open();
        assert!(matches!(
            control_outcome(&mut session).await.result,
            Ok(Receipt::PermissionRuleConfirmed(_))
        ));
        assert!(handle.is_cancelled());
        finish_turn(&mut session).await;
        assert_eq!(session.runtime().id(), id);
        assert_eq!(session.runtime().status().queued_jobs, 1);
        assert_eq!(
            session.runtime().status().phase,
            NativeConversationRuntimePhase::Open
        );
        close(session, fixture).await;
    });
}

#[test]
fn published_but_failed_control_preserves_error_and_rejects_switch_without_rollback() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let store = gated(&mut session).await;
        let current = session.current.clone();
        let before = current.record();
        store.gate.armed.store(true, Ordering::SeqCst);
        store.gate.fail_after_publish.store(true, Ordering::SeqCst);
        session
            .request_control(
                Control::Rename {
                    title: "uncertain saved title".into(),
                },
                200,
            )
            .unwrap();
        assert!(poll_once(&mut session).is_pending());
        let request = session
            .request_transition(NativeInteractiveTransition::Reset, 210)
            .unwrap();
        store.gate.open();
        let NativeInteractiveOutcome::Rejected {
            request: rejected,
            error: NativeInteractiveError::ControlFailed,
            candidate,
            ..
        } = outcome(&mut session).await
        else {
            panic!("control rejects switching");
        };
        assert_eq!(rejected, request.id);
        assert!(candidate.is_none());
        assert!(Arc::ptr_eq(session.runtime(), &current));
        assert_eq!(current.status().phase, NativeConversationRuntimePhase::Open);
        assert_eq!(current.record().revision, before.revision);
        assert!(store.inner.record(&current.id()).unwrap().revision > before.revision);
        assert!(matches!(
            session.request_transition(NativeInteractiveTransition::New, 220),
            Err(NativeInteractiveError::ControlFailed)
        ));
        let failure = session.take_control_outcome().unwrap();
        assert!(matches!(failure.result, Err(ControlError::Runtime(_))));
        assert!(!format!("{failure:?}").contains("uncertain saved title"));
        drop(current);
        close(session, fixture).await;
    });
}

#[test]
fn continuation_checks_checkpoint_before_queueing_and_receipt_blocks_provider_admission() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        assert!(
            session
                .request_control(
                    Control::Continue {
                        options: InferenceOptions::default()
                    },
                    200
                )
                .is_err()
        );
        session.enqueue("original prompt".into()).unwrap();
        let turn = session.current.start_next(200).await.unwrap().unwrap();
        let _ = turn.handle().unwrap().cancel();
        session.turn = Some(turn);
        finish_turn(&mut session).await;
        let before = session.runtime().record();
        let id = session
            .request_control(
                Control::Continue {
                    options: InferenceOptions::default(),
                },
                310,
            )
            .unwrap();
        assert_eq!(id.get(), 1);
        assert_eq!(session.runtime().record(), before);
        assert_eq!(session.runtime().status().queued_jobs, 1);
        assert!(poll_once(&mut session).is_ready());
        assert!(session.admission.is_none());
        assert!(fixture.transport.requests().is_empty());
        assert!(matches!(
            session.request_control(Control::Compact, 310),
            Err(NativeInteractiveError::Busy)
        ));
        assert!(matches!(
            session.take_control_outcome().unwrap().result,
            Ok(Receipt::Continued(_))
        ));
        // Continue must not move ahead of an existing queued job.
        assert!(
            session
                .request_control(
                    Control::Continue {
                        options: InferenceOptions::default()
                    },
                    310
                )
                .is_err()
        );
        fixture.transport.push(support::answer());
        finish_turn(&mut session).await;
        assert_eq!(
            session
                .runtime()
                .record()
                .messages
                .iter()
                .filter(|message| message.role == machine_god_core::Role::User)
                .count(),
            1
        );
        close(session, fixture).await;
    });
}

#[test]
fn active_model_defaults_save_preserves_deferred_session_and_progresses_with_blocked_presentation()
{
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        fixture.transport.push(support::answer());
        session.enqueue("active".into()).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = session.poll_progress(cx, 200);
                if session.presentation.is_some() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(session.runtime().status().active);
        let generation = session
            .set_model_preferences(preferences("future/selection"))
            .unwrap();
        let store = Arc::new(NativeUserConfigStore::new(
            fixture.workspace.join("active-defaults"),
        ));
        session
            .request_control(
                Control::SaveModelDefaults {
                    store: store.clone(),
                },
                210,
            )
            .unwrap();
        assert!(poll_once(&mut session).is_ready());
        assert!(session.presentation.is_some());
        let result = session.control_outcome.as_ref().unwrap();
        let Ok(Receipt::ModelDefaults(commit)) = &result.result else {
            panic!("composite result");
        };
        assert_eq!(commit.generation, generation);
        assert_eq!(
            commit.session,
            Ok(NativeModelPreferencePersistence::Deferred)
        );
        assert!(commit.user_defaults.is_ok());
        assert_eq!(
            store.load().unwrap().loaded().config().model_preferences(),
            preferences("future/selection")
        );
        session.request_shutdown();
        while !session.is_closed() {
            let _ = outcome(&mut session).await;
        }
        assert!(session.take_control_outcome().is_some());
        drop(session);
        fixture.finish();
    });
}

#[test]
fn partial_model_save_keeps_both_targets_and_rejects_pending_reset() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        session.set_model_preferences(preferences("session/saved")).unwrap();
        let blocked_parent = fixture.workspace.join("blocked-parent");
        std::fs::write(&blocked_parent, b"not a directory").unwrap();
        let store = Arc::new(NativeUserConfigStore::new(blocked_parent.join("defaults")));
        session.request_control(Control::SaveModelDefaults { store }, 200).unwrap();
        let request = session.request_transition(NativeInteractiveTransition::Reset, 210).unwrap();
        assert!(matches!(outcome(&mut session).await, NativeInteractiveOutcome::Rejected { request: rejected, error: NativeInteractiveError::ControlFailed, .. } if rejected == request.id));
        let Receipt::ModelDefaults(commit) = session.take_control_outcome().unwrap().result.unwrap() else { panic!("independent receipts"); };
        assert!(matches!(commit.session, Ok(NativeModelPreferencePersistence::Saved { .. })));
        assert!(commit.user_defaults.is_err());
        assert_eq!(session.current.status().phase, NativeConversationRuntimePhase::Open);
        assert!(!session.is_fenced());
        close(session, fixture).await;
    });
}

#[test]
fn control_bounds_ids_redaction_and_empty_receipt_wakes_are_explicit() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let before = session.runtime().record();
        for title in [String::new(), "x".repeat(241), "bad\0title".into()] {
            assert!(matches!(
                session.request_control(Control::Rename { title }, 200),
                Err(NativeInteractiveError::Configuration)
            ));
        }
        let oversized = InferenceOptions {
            model: Some("x".repeat(crate::MAX_NATIVE_QUEUED_OPTIONS_BYTES + 1)),
            ..InferenceOptions::default()
        };
        assert!(matches!(
            session.request_control(Control::Continue { options: oversized }, 200),
            Err(NativeInteractiveError::Runtime(
                crate::NativeConversationRuntimeError::InputLimit
            ))
        ));
        session.next_control = u64::MAX;
        assert!(matches!(
            session.request_control(Control::Compact, 200),
            Err(NativeInteractiveError::IdentityExhausted)
        ));
        assert_eq!(session.runtime().record(), before);
        session.next_control = 1;
        session
            .request_control(
                Control::Rename {
                    title: "private-visible-title".into(),
                },
                200,
            )
            .unwrap();
        assert!(
            !format!(
                "{:?}",
                Control::Rename {
                    title: "private-visible-title".into()
                }
            )
            .contains("private-visible-title")
        );
        assert!(matches!(
            session.set_model_preferences(preferences("unaccepted/model")),
            Err(NativeInteractiveError::Busy)
        ));
        let mut nested = serde_json::Value::Null;
        for _ in 0..10_000 {
            nested = serde_json::Value::Array(vec![nested]);
        }
        let rejected = InferenceOptions {
            metadata: BTreeMap::from([("nested".into(), nested)]),
            ..InferenceOptions::default()
        };
        assert!(matches!(
            session.request_control(Control::Continue { options: rejected }, 200),
            Err(NativeInteractiveError::Busy)
        ));
        let result = control_outcome(&mut session).await;
        assert!(!format!("{result:?}").contains("private-visible-title"));
        let counter = Arc::new(Count(AtomicUsize::new(0)));
        let wake = Waker::from(counter.clone());
        assert!(
            session
                .poll_progress(&mut Context::from_waker(&wake), 300)
                .is_pending()
        );
        assert!(session.take_control_outcome().is_none());
        assert_eq!(counter.0.load(Ordering::SeqCst), 0);
        close(session, fixture).await;
    });
}

#[test]
fn composed_failed_rename_rejects_reset_and_retains_actual_file_undo() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        fixture.transport.push(support::call(
            "write_file",
            &serde_json::json!({"path":"retained.txt","content":"actual saved bytes"}),
        ));
        fixture.transport.push(support::answer());
        session.enqueue("write the file".into()).unwrap();
        finish_turn(&mut session).await;
        assert_eq!(
            std::fs::read(fixture.workspace.join("retained.txt")).unwrap(),
            b"actual saved bytes"
        );
        let current = session.current.clone();
        let blocker = fixture.block_publication(&current.id());
        session
            .request_control(
                Control::Rename {
                    title: "cannot publish".into(),
                },
                400,
            )
            .unwrap();
        session
            .request_transition(NativeInteractiveTransition::Reset, 410)
            .unwrap();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Rejected {
                error: NativeInteractiveError::ControlFailed,
                candidate: None,
                ..
            }
        ));
        assert!(matches!(
            session.take_control_outcome().unwrap().result,
            Err(ControlError::Runtime(_))
        ));
        assert!(Arc::ptr_eq(session.runtime(), &current));
        assert_eq!(current.status().phase, NativeConversationRuntimePhase::Open);
        assert_eq!(
            fixture.undo.undo_last(&CancellationToken::new()).unwrap(),
            crate::FileUndoOutcome::Removed("retained.txt".into())
        );
        drop(blocker);
        drop(current);
        close(session, fixture).await;
    });
}

#[test]
fn shutdown_finishes_failed_session_save_then_user_default_success_and_retains_both() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        let gated = gated(&mut session).await;
        gated.gate.armed.store(true, Ordering::SeqCst);
        gated.gate.fail_after_publish.store(true, Ordering::SeqCst);
        session
            .set_model_preferences(preferences("partially/published"))
            .unwrap();
        let store = Arc::new(NativeUserConfigStore::new(
            fixture.workspace.join("retained-defaults"),
        ));
        session
            .request_control(
                Control::SaveModelDefaults {
                    store: store.clone(),
                },
                200,
            )
            .unwrap();
        assert!(poll_once(&mut session).is_pending());
        session.request_shutdown();
        assert!(poll_once(&mut session).is_pending());
        gated.gate.open();
        assert!(matches!(
            outcome(&mut session).await,
            NativeInteractiveOutcome::Shutdown
        ));
        assert!(session.is_closed());
        let Receipt::ModelDefaults(commit) =
            session.take_control_outcome().unwrap().result.unwrap()
        else {
            panic!("independent save results");
        };
        assert!(commit.session.is_err());
        assert!(commit.user_defaults.is_ok());
        assert_eq!(
            store.load().unwrap().loaded().config().model_preferences(),
            preferences("partially/published")
        );
        assert_eq!(gated.gate.entered.load(Ordering::SeqCst), 1);
        assert_eq!(gated.gate.dropped.load(Ordering::SeqCst), 1);
        drop(session);
        fixture.finish();
    });
}

#[test]
fn unread_receipt_does_not_first_poll_a_previously_retained_admission() {
    executor().block_on(async {
        let fixture = Fixture::new();
        let mut session = owner(&fixture).await;
        session.enqueue("retained but not taken".into()).unwrap();
        let runtime = session.current.clone();
        // Reproduce the owner's progress-budget boundary with its actual owned
        // admission future, not a fabricated active turn or publication stage.
        session.admission = Some(Box::pin(async move { runtime.start_next(300).await }));
        session
            .request_control(
                Control::Rename {
                    title: "before taking input".into(),
                },
                200,
            )
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = session.poll_progress(cx, 300);
                if session.control_outcome.is_some() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }),
        )
        .await
        .unwrap();
        assert!(session.admission.is_some());
        assert_eq!(session.current.status().queued_jobs, 1);
        assert!(!session.current.status().active);
        assert!(fixture.transport.requests().is_empty());
        assert!(session.take_control_outcome().unwrap().result.is_ok());
        fixture.transport.push(support::answer());
        finish_turn(&mut session).await;
        close(session, fixture).await;
    });
}
