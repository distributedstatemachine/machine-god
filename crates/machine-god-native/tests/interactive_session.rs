#![cfg(all(
    feature = "ai-gateway-http",
    not(target_family = "wasm"),
    any(target_os = "linux", target_os = "macos")
))]

#[path = "interactive_session/support.rs"]
mod support;

use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use futures_util::future::poll_fn;
use machine_god_core::{
    CancellationToken, EngineEvent, SessionId, ToolCallId, ToolContext, TurnEvent, TurnId,
};
use machine_god_native as native;
use machine_god_native::{
    FileUndoOutcome, NATIVE_MODEL_PREFERENCES_KEY, NativeConversationRuntimePhase,
    NativeInteractiveError, NativeInteractiveInitialSession, NativeInteractiveOutcome,
    NativeInteractiveSession, NativeInteractiveSessionOptions, NativeInteractiveTransition,
    NativeInteractiveTransitionReceipt, NativeModelPreferences, NativeReasoningEffort,
    NativeResumeTarget, NativeSessionMetadata, NativeSessionOrigin, NativeSessionResumeErrorKind,
    PermissionMode,
};
use serde_json::{Value, json};
use support::{Fixture, answer, call};

#[test]
fn public_workspace_control_keeps_additional_authority_across_resume() {
    executor().block_on(async {
        let fixture = Fixture::new_with_workspace();
        let shared = fixture.workspace.parent().unwrap().join("shared-root");
        std::fs::create_dir(&shared).unwrap();
        let store = Arc::new(native::NativeUserConfigStore::new(
            fixture.workspace.parent().unwrap().join("user-settings"),
        ));
        let mut owner = fresh(&fixture).await;
        let original_id = owner.runtime().id();
        owner
            .request_control(
                native::NativeInteractiveControl::Workspace {
                    action: native::NativeWorkspaceAction::Add(shared.clone()),
                    store,
                },
                150,
            )
            .unwrap();
        let control = tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| {
                let _ = owner.poll_progress(cx, 150);
                owner
                    .take_control_outcome()
                    .map_or(Poll::Pending, Poll::Ready)
            }),
        )
        .await
        .unwrap();
        assert!(!control.failed());
        transition(&mut owner, NativeInteractiveTransition::New, 160).await;
        transition(
            &mut owner,
            NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(original_id)),
            170,
        )
        .await;
        let file = shared.join("selected.txt");
        run_tool(
            &fixture,
            &mut owner,
            "write_file",
            &json!({"path":file,"content":"selected root"}),
            180,
        )
        .await;
        assert_eq!(std::fs::read(file).unwrap(), b"selected root");
        shutdown(&mut owner, 190).await;
        drop(owner);
        fixture.finish();
    });
}

fn preferences(model: &str) -> NativeModelPreferences {
    NativeModelPreferences::new(model, NativeReasoningEffort::default(), false).unwrap()
}

fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn options(fixture: &Fixture) -> NativeInteractiveSessionOptions {
    NativeInteractiveSessionOptions::new(
        fixture.workspace.clone(),
        preferences("workspace/default"),
    )
    .unwrap()
}

async fn fresh(fixture: &Fixture) -> NativeInteractiveSession {
    NativeInteractiveSession::open(
        fixture.host.clone(),
        options(fixture),
        NativeInteractiveInitialSession::Fresh,
        100,
    )
    .await
    .unwrap()
}

async fn outcome(
    owner: &mut NativeInteractiveSession,
    now: i64,
) -> (NativeInteractiveOutcome, Vec<EngineEvent>) {
    let mut events = Vec::new();
    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        poll_fn(|cx| {
            let _ = owner.poll_progress(cx, now);
            while let Some(event) = owner.take_presentation() {
                events.push(event);
            }
            owner.take_outcome().map_or(Poll::Pending, Poll::Ready)
        }),
    )
    .await
    .expect("interactive owner made bounded progress");
    (outcome, events)
}

async fn transition(
    owner: &mut NativeInteractiveSession,
    kind: NativeInteractiveTransition,
    now: i64,
) -> NativeInteractiveTransitionReceipt {
    let request = owner.request_transition(kind, now).unwrap();
    let (result, _) = outcome(owner, now).await;
    let NativeInteractiveOutcome::Transition(receipt) = result else {
        panic!("expected transition receipt, got {result:?}");
    };
    assert_eq!(receipt.request, request.id);
    receipt
}

async fn run_tool(
    fixture: &Fixture,
    owner: &mut NativeInteractiveSession,
    name: &str,
    input: &Value,
    now: i64,
) -> Value {
    fixture.transport.push(call(name, input));
    fixture.transport.push(answer());
    owner
        .enqueue("perform the scripted operation".into())
        .unwrap();
    let (result, events) = outcome(owner, now).await;
    assert!(
        matches!(&result, NativeInteractiveOutcome::Turn(Ok(event)) if matches!(event.payload, TurnEvent::Completed { .. })),
        "{result:?}"
    );
    let output = events
        .iter()
        .find_map(|event| match &event.payload {
            TurnEvent::ToolFinished { output, .. } => Some(output),
            _ => None,
        })
        .expect("real native tool result");
    assert!(!output.is_error, "native tool failed: {:?}", output.content);
    output.content.clone()
}

async fn shutdown(owner: &mut NativeInteractiveSession, now: i64) {
    owner.request_shutdown();
    let (result, _) = outcome(owner, now).await;
    assert!(
        matches!(result, NativeInteractiveOutcome::Shutdown),
        "{result:?}"
    );
    assert!(owner.is_closed());
}

fn context(owner: &NativeInteractiveSession) -> ToolContext {
    let record = owner.runtime().record();
    ToolContext {
        session_id: record.id,
        session_incarnation_id: record.incarnation_id,
        turn_id: TurnId::new("route-observation").unwrap(),
        call_id: ToolCallId::new("route-observation").unwrap(),
    }
}

async fn seed(fixture: &Fixture, model: &str, now: i64) -> SessionId {
    let session = fixture
        .host
        .session_lifecycle()
        .create_generated_with_metadata(
            NativeSessionMetadata::new(&fixture.workspace, now, NativeSessionOrigin::Cli).unwrap(),
        )
        .await
        .unwrap();
    let record = session.record();
    let mut metadata = record.metadata;
    metadata.insert(
        NATIVE_MODEL_PREFERENCES_KEY.into(),
        preferences(model).to_value(),
    );
    session
        .update_metadata(record.revision, metadata)
        .await
        .unwrap();
    session.id()
}

#[test]
fn fresh_clear_new_reset_publish_distinct_ids_and_retire_all_old_aliases() {
    let fixture = Fixture::new();
    executor().block_on(async {
        let unpolled = NativeInteractiveSession::open(
            fixture.host.clone(),
            options(&fixture),
            NativeInteractiveInitialSession::Fresh,
            99,
        );
        drop(unpolled);
        assert!(fixture.transport.requests().is_empty());
        assert!(
            fixture
                .host
                .session_lifecycle()
                .list_sessions()
                .await
                .unwrap()
                .session_ids()
                .is_empty()
        );
        let mut owner = fresh(&fixture).await;
        let mut ids = vec![owner.runtime().id()];
        for (index, kind) in [
            NativeInteractiveTransition::Clear,
            NativeInteractiveTransition::New,
            NativeInteractiveTransition::Reset,
        ]
        .into_iter()
        .enumerate()
        {
            let now = 110 + i64::try_from(index).unwrap();
            let old = owner.runtime().clone();
            let old_permissions = old.permissions().unwrap().clone();
            old_permissions.set_mode(PermissionMode::Yolo).unwrap();
            let old_context = context(&owner);
            assert!(fixture.routes.snapshot(&old_context).is_some());
            owner
                .enqueue("discard only after a successful transition".into())
                .unwrap();
            let receipt = transition(&mut owner, kind, now).await;
            assert!(!receipt.unchanged);
            assert_eq!(receipt.source.session_id(), &old.id());
            assert_eq!(receipt.destination.session_id(), &owner.runtime().id());
            assert!(!ids.contains(&owner.runtime().id()));
            ids.push(owner.runtime().id());
            assert_eq!(old.status().phase, NativeConversationRuntimePhase::Retired);
            assert!(old.enqueue("old alias".into()).is_err());
            assert!(old_permissions.snapshot().is_err());
            assert!(old_permissions.set_mode(PermissionMode::Ask).is_err());
            assert!(fixture.routes.snapshot(&old_context).is_none());
            assert_eq!(owner.runtime().status().queued_jobs, 0);
            assert_eq!(
                owner.runtime().model_preferences().model(),
                "workspace/default"
            );
            assert!(fixture.transport.requests().is_empty());
            let durable = fixture
                .host
                .session_lifecycle()
                .replay(owner.runtime().id())
                .await
                .unwrap();
            assert_eq!(durable.id, owner.runtime().id());
            assert_eq!(
                NativeSessionMetadata::from_metadata(&durable.metadata)
                    .unwrap()
                    .workspace(),
                Some(fixture.workspace.as_path())
            );
        }
        shutdown(&mut owner, 120).await;
        drop(owner);
    });
    fixture.finish();
}

#[test]
fn exact_and_latest_resume_restore_saved_selection_but_new_uses_workspace_defaults() {
    let fixture = Fixture::new();
    executor().block_on(async {
        let first = seed(&fixture, "saved/first", 10).await;
        let latest = seed(&fixture, "saved/latest", 20).await;
        let mut owner = NativeInteractiveSession::open(
            fixture.host.clone(),
            options(&fixture),
            NativeInteractiveInitialSession::Resume(NativeResumeTarget::Latest),
            30,
        )
        .await
        .unwrap();
        assert_eq!(owner.runtime().id(), latest);
        assert_eq!(owner.runtime().model_preferences().model(), "saved/latest");
        let old_alias = owner.runtime().clone();
        transition(
            &mut owner,
            NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(first.clone())),
            31,
        )
        .await;
        assert_eq!(owner.runtime().id(), first);
        assert_eq!(owner.runtime().model_preferences().model(), "saved/first");
        assert_eq!(
            old_alias.status().phase,
            NativeConversationRuntimePhase::Retired
        );
        transition(&mut owner, NativeInteractiveTransition::New, 32).await;
        assert_eq!(
            owner.runtime().model_preferences().model(),
            "workspace/default"
        );
        assert_ne!(owner.runtime().id(), first);
        assert_ne!(owner.runtime().id(), latest);
        // The retired alias stays alive while its exact durable identity is registered again.
        transition(
            &mut owner,
            NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(latest.clone())),
            33,
        )
        .await;
        assert_eq!(owner.runtime().id(), latest);
        assert_eq!(owner.runtime().model_preferences().model(), "saved/latest");
        drop(old_alias);
        assert_eq!(
            fixture.routes.snapshot(&context(&owner)).as_deref(),
            Some("saved/latest")
        );
        let newer = seed(&fixture, "saved/newest", 40).await;
        fixture
            .host
            .session_lifecycle()
            .create_generated_with_metadata(
                NativeSessionMetadata::new(
                    &fixture.workspace.join("other-workspace"),
                    500,
                    NativeSessionOrigin::Cli,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        transition(
            &mut owner,
            NativeInteractiveTransition::Resume(NativeResumeTarget::Latest),
            41,
        )
        .await;
        assert_eq!(owner.runtime().id(), newer);
        assert_eq!(owner.runtime().model_preferences().model(), "saved/newest");
        shutdown(&mut owner, 42).await;
        drop(owner);
    });
    fixture.finish();
}

#[test]
fn initial_exact_override_preserves_saved_controls_and_does_not_become_fresh_defaults() {
    let fixture = Fixture::new();
    executor().block_on(async {
        let selected = seed(&fixture, "saved/exact", 10).await;
        let session = fixture
            .host
            .session_lifecycle()
            .resume(selected.clone())
            .await
            .unwrap();
        let record = session.record();
        let mut metadata = record.metadata;
        let saved = NativeModelPreferences::new(
            "saved/exact",
            NativeReasoningEffort::parse("high").unwrap(),
            true,
        )
        .unwrap();
        metadata.insert(NATIVE_MODEL_PREFERENCES_KEY.into(), saved.to_value());
        session
            .update_metadata(record.revision, metadata)
            .await
            .unwrap();
        let mut owner = NativeInteractiveSession::open(
            fixture.host.clone(),
            options(&fixture)
                .with_process_model_override("process/override")
                .unwrap(),
            NativeInteractiveInitialSession::Resume(NativeResumeTarget::Exact(selected)),
            20,
        )
        .await
        .unwrap();
        let restored = owner.runtime().model_preferences();
        assert_eq!(restored.model(), "process/override");
        assert_eq!(restored.effort().label(), "high");
        assert!(restored.requested_fast());
        transition(&mut owner, NativeInteractiveTransition::New, 21).await;
        assert_eq!(
            owner.runtime().model_preferences(),
            preferences("workspace/default")
        );
        assert_eq!(
            fixture
                .host
                .loaded_config()
                .config()
                .model_preferences()
                .model(),
            "fixture/default"
        );
        shutdown(&mut owner, 22).await;
        drop(owner);
    });
    fixture.finish();
}

#[test]
fn same_id_resume_is_noop_and_invalid_target_preserves_policy_queue_and_real_undo() {
    let fixture = Fixture::new();
    executor().block_on(async {
        let mut owner = fresh(&fixture).await;
        run_tool(&fixture, &mut owner, "write_file", &json!({"path":"keep.txt","content":"kept"}), 101).await;
        let old = owner.runtime().clone();
        old.permissions().unwrap().set_mode(PermissionMode::Yolo).unwrap();
        old.set_model_preferences(preferences("selected/unsaved")).unwrap();
        owner.enqueue("remain pending after rejection".into()).unwrap();
        let receipt = transition(&mut owner, NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(old.id())), 102).await;
        assert!(receipt.unchanged);
        assert!(Arc::ptr_eq(owner.runtime(), &old));
        assert_eq!(old.status().queued_jobs, 1);
        let reservation = fixture.undo.reserve_clear().unwrap();
        let request = owner.request_transition(NativeInteractiveTransition::Clear, 103).unwrap();
        let (result, _) = outcome(&mut owner, 103).await;
        assert!(matches!(result, NativeInteractiveOutcome::Rejected { request: id, error: NativeInteractiveError::Undo(machine_god_native::FileUndoError::Busy), .. } if id == request.id), "{result:?}");
        assert!(Arc::ptr_eq(owner.runtime(), &old));
        assert_eq!(old.status().queued_jobs, 1);
        drop(reservation);
        let corrupt = seed(&fixture, "initial/valid", 1).await;
        let session = fixture.host.session_lifecycle().resume(corrupt.clone()).await.unwrap();
        let record = session.record();
        let mut metadata = record.metadata;
        metadata.insert(NATIVE_MODEL_PREFERENCES_KEY.into(), json!({"schema_version":999}));
        session.update_metadata(record.revision, metadata).await.unwrap();
        let busy = seed(&fixture, "busy/target", 2).await;
        let busy_session = fixture.host.session_lifecycle().resume(busy.clone()).await.unwrap();
        let held_turn = busy_session.prompt("hold exact target admission").await.unwrap();
        for (target, expected) in [(corrupt, NativeSessionResumeErrorKind::Corrupt), (SessionId::new("missing-target").unwrap(), NativeSessionResumeErrorKind::NotFound), (busy, NativeSessionResumeErrorKind::Busy)] {
            let request = owner.request_transition(NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(target)), 103).unwrap();
            let (result, _) = outcome(&mut owner, 103).await;
            assert!(matches!(result, NativeInteractiveOutcome::Rejected { request: id, error: NativeInteractiveError::Resume(error), .. } if id == request.id && error.kind() == expected), "{result:?}");
            assert!(Arc::ptr_eq(owner.runtime(), &old));
            assert_eq!(old.status().phase, NativeConversationRuntimePhase::Open);
            assert_eq!(old.status().queued_jobs, 1);
            assert_eq!(old.model_preferences().model(), "selected/unsaved");
            assert_eq!(old.permissions().unwrap().snapshot().unwrap().mode(), PermissionMode::Yolo);
            assert_eq!(std::fs::read(fixture.workspace.join("keep.txt")).unwrap(), b"kept");
        }
        drop(held_turn);
        assert_eq!(fixture.undo.undo_last(&CancellationToken::new()).unwrap(), FileUndoOutcome::Removed("keep.txt".into()));
        assert_eq!(fixture.transport.requests().len(), 2);
        shutdown(&mut owner, 104).await;
        drop(owner);
    });
    fixture.finish();
}

#[test]
fn completed_transition_clears_real_undo_without_reverting_the_written_file() {
    let fixture = Fixture::new();
    executor().block_on(async {
        let mut owner = fresh(&fixture).await;
        run_tool(
            &fixture,
            &mut owner,
            "write_file",
            &json!({"path":"committed.txt","content":"durable effect"}),
            101,
        )
        .await;
        let history = owner.runtime().history().unwrap();
        assert_eq!(history.groups().len(), 1);
        assert_eq!(history.groups()[0].files().len(), 1);
        let fact = &history.groups()[0].files()[0];
        assert_eq!(fact.path(), "committed.txt");
        assert_eq!(
            fact.status(),
            machine_god_native::NativeHistoryFileStatus::Success
        );
        assert!(fact.source().is_some());
        transition(&mut owner, NativeInteractiveTransition::Clear, 102).await;
        assert_eq!(
            fixture.undo.undo_last(&CancellationToken::new()).unwrap(),
            FileUndoOutcome::Empty
        );
        assert_eq!(
            std::fs::read(fixture.workspace.join("committed.txt")).unwrap(),
            b"durable effect"
        );
        assert!(Arc::ptr_eq(
            &fixture.observations,
            &fixture.host.observations().unwrap()
        ));
        shutdown(&mut owner, 103).await;
        drop(owner);
    });
    fixture.finish();
}

#[test]
fn real_terminal_carry_and_stop_forget_follow_all_four_transition_kinds() {
    for resume in [false, true] {
        let fixture = Fixture::new();
        executor().block_on(async {
            let target = seed(&fixture, "saved/terminal-target", 50).await;
            let mut owner = fresh(&fixture).await;
            let started = run_tool(
                &fixture,
                &mut owner,
                "terminal",
                &json!({"action":"start","profile":"clean","command":"exec /bin/sleep 120"}),
                101,
            )
            .await;
            let id = started["session"]["session_id"].clone();
            assert!(id.is_string());
            let carry = if resume {
                NativeInteractiveTransition::New
            } else {
                NativeInteractiveTransition::Clear
            };
            let receipt = transition(&mut owner, carry, 102).await;
            assert_eq!(receipt.handoff.unwrap().transferred(), 1);
            let inspected = run_tool(
                &fixture,
                &mut owner,
                "terminal",
                &json!({"action":"inspect","session_id":id}),
                103,
            )
            .await;
            assert_eq!(inspected["session"]["session_id"], id);
            assert_eq!(inspected["session"]["lifecycle"], "running");
            let forget = if resume {
                NativeInteractiveTransition::Resume(NativeResumeTarget::Exact(target))
            } else {
                NativeInteractiveTransition::Reset
            };
            let receipt = transition(&mut owner, forget, 104).await;
            let reset = receipt.reset.unwrap();
            assert_eq!(reset.entries().len(), 1);
            assert_eq!(reset.entries()[0].id().as_str(), id.as_str().unwrap());
            assert_eq!(
                reset.entries()[0].outcome(),
                machine_god_native::NativeTerminalResetOutcome::StoppedAndForgotten
            );
            let listed = run_tool(
                &fixture,
                &mut owner,
                "terminal",
                &json!({"action":"list"}),
                105,
            )
            .await;
            assert_eq!(listed["sessions"], json!([]));
            shutdown(&mut owner, 106).await;
            drop(owner);
        });
        fixture.finish();
    }
}

#[test]
fn blocked_presentation_does_not_own_transition_or_shutdown_progress() {
    let fixture = Fixture::new();
    executor().block_on(async {
        let mut owner = fresh(&fixture).await;
        fixture.transport.push(answer());
        owner
            .enqueue("produce a presentation event".into())
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| owner.poll_progress(cx, 101)),
        )
        .await
        .unwrap();
        // Leave the single presentation slot unconsumed when requesting control work.
        let original = owner.runtime().id();
        let receipt = transition(&mut owner, NativeInteractiveTransition::Clear, 102).await;
        assert_eq!(receipt.source.session_id(), &original);
        assert_ne!(owner.runtime().id(), original);
        fixture.transport.push(answer());
        owner.enqueue("block presentation again".into()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            poll_fn(|cx| owner.poll_progress(cx, 103)),
        )
        .await
        .unwrap();
        shutdown(&mut owner, 104).await;
        drop(owner);
    });
    fixture.finish();
}

#[test]
fn actual_finalizer_publication_failure_rejects_switch_and_preserves_committed_effect() {
    let fixture = Fixture::new();
    executor().block_on(async {
        let mut owner = fresh(&fixture).await;
        let original = owner.runtime().clone();
        fixture.transport.push(call("write_file", &json!({"path":"before-failed-finalizer","content":"already committed"})));
        fixture.transport.push(answer());
        owner.enqueue("perform the write".into()).unwrap();
        tokio::time::timeout(Duration::from_secs(30), poll_fn(|cx| {
            let _ = owner.poll_progress(cx, 101);
            while let Some(event) = owner.take_presentation() {
                if let TurnEvent::ToolFinished { output, .. } = event.payload {
                    assert!(!output.is_error);
                    return Poll::Ready(());
                }
            }
            Poll::Pending
        })).await.unwrap();
        let durable_before = fixture.host.session_lifecycle().replay(original.id()).await.unwrap();
        let blocked = fixture.block_publication(&original.id());
        owner.enqueue("preserve queued prompt when finalization fails".into()).unwrap();
        let request = owner.request_transition(NativeInteractiveTransition::Clear, 102).unwrap();
        let (result, _) = outcome(&mut owner, 102).await;
        assert!(matches!(result, NativeInteractiveOutcome::Rejected { request: id, .. } if id == request.id), "{result:?}");
        assert!(Arc::ptr_eq(owner.runtime(), &original));
        assert_eq!(owner.runtime().status().phase, NativeConversationRuntimePhase::Open);
        assert_eq!(owner.runtime().status().queued_jobs, 1);
        assert_eq!(std::fs::read(fixture.workspace.join("before-failed-finalizer")).unwrap(), b"already committed");
        drop(blocked);
        assert_eq!(fixture.host.session_lifecycle().replay(original.id()).await.unwrap(), durable_before);
        assert_eq!(fixture.host.session_lifecycle().list_sessions().await.unwrap().session_ids(), &[original.id()]);
        assert_eq!(fixture.undo.undo_last(&CancellationToken::new()).unwrap(), FileUndoOutcome::Removed("before-failed-finalizer".into()));
        shutdown(&mut owner, 103).await;
        drop(owner);
    });
    fixture.finish();
}
