//! Owned-worker tests use a deterministic in-memory conversation, no helper.
use super::*;
use crate::{NativeConversation, NativeModelPreferences, NativeReasoningEffort};
use futures_executor::block_on;
use machine_god_core::{Engine, SessionId, SessionIncarnationId};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use std::{
    task::{Context, Waker},
    time::Duration,
};

fn runtime() -> Arc<NativeConversationRuntime> {
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(ScriptedModelProvider::new("skills-control", []))
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("skills-control").unwrap(),
            SessionIncarnationId::new("exact-life").unwrap(),
        )
        .unwrap();
    Arc::new(
        NativeConversationRuntime::new(
            NativeConversation::from_session(session).unwrap(),
            NativeModelPreferences::new("test/model", NativeReasoningEffort::default(), false)
                .unwrap(),
            None,
        )
        .unwrap(),
    )
}

#[test]
fn unpolled_control_is_inert_and_dropping_it_only_cancels_private_token() {
    let runtime = runtime();
    let workers = NativeOwnedWorkerScope::new();
    let token = CancellationToken::new();
    let future = run(runtime.clone(), workers.clone(), token.clone(), |_| {
        panic!("unpolled")
    });
    let mut fence = runtime.begin_quiescence().unwrap();
    assert!(fence.try_retire().is_ok());
    drop(future);
    assert!(token.is_cancelled());
    workers.close();
    assert!(workers.completion().is_complete());
}

#[test]
fn dropped_response_keeps_worker_fence_and_cleanup_until_actual_completion() {
    let runtime = runtime();
    let workers = NativeOwnedWorkerScope::new();
    let token = CancellationToken::new();
    let (entered, observed) = std::sync::mpsc::sync_channel(1);
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    let mut future = run(
        runtime.clone(),
        workers.clone(),
        token.clone(),
        move |cancellation| {
            assert!(NativeOwnedWorkerScope::retain_current_cleanup().is_some());
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(cancellation.is_cancelled());
            Ok(NativeSkillsServiceResult::Path("/reported".into()))
        },
    );
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    observed.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut fence = runtime.begin_quiescence().unwrap();
    drop(future);
    assert!(token.is_cancelled());
    workers.close();
    assert!(!workers.completion().is_complete());
    assert!(fence.try_retire().is_err());
    release.send(()).unwrap();
    workers.completion().wait_on_worker().unwrap();
    assert!(fence.try_retire().is_ok());
}

#[test]
fn cancelled_closed_or_retired_controls_do_not_enter_operation() {
    let workers = NativeOwnedWorkerScope::new();
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        block_on(run(runtime(), workers.clone(), token, |_| panic!(
            "cancelled"
        ))),
        Err(Error::Skills(NativeSkillsServiceError::Cancelled))
    ));
    workers.close();
    assert!(matches!(
        block_on(run(
            runtime(),
            workers,
            CancellationToken::new(),
            |_| panic!("closed")
        )),
        Err(Error::Unavailable)
    ));
    let runtime = runtime();
    runtime.begin_quiescence().unwrap().retire().unwrap();
    assert!(matches!(
        block_on(run(
            runtime,
            NativeOwnedWorkerScope::new(),
            CancellationToken::new(),
            |_| panic!("retired")
        )),
        Err(Error::Runtime(_))
    ));
}

#[test]
fn exact_partial_receipts_and_recovery_errors_survive_control_worker() {
    use crate::{
        NativeSkillBatchReceipt, NativeSkillItemOutcome, NativeSkillItemReceipt,
        NativeSkillManagedError, NativeSkillManagedErrorKind,
    };
    let workers = NativeOwnedWorkerScope::new();
    let receipt = block_on(run(
        runtime(),
        workers.clone(),
        CancellationToken::new(),
        |_| {
            Ok(NativeSkillsServiceResult::Managed(
                NativeSkillBatchReceipt {
                    items: vec![NativeSkillItemReceipt {
                        destination: "exact".into(),
                        outcome: NativeSkillItemOutcome::Indeterminate,
                        error: Some(NativeSkillManagedErrorKind::Indeterminate),
                        recovery_id: Some("private-recovery".into()),
                    }],
                },
            ))
        },
    ))
    .unwrap();
    let outcome = crate::NativeInteractiveControlOutcome {
        id: super::super::NativeInteractiveControlId(1),
        source: machine_god_core::BackgroundOutputOwner::new(
            SessionId::new("original").unwrap(),
            SessionIncarnationId::new("original-life").unwrap(),
        ),
        result: Ok(receipt),
    };
    assert!(outcome.failed());
    let Ok(Receipt::Skills(result)) = outcome.result else {
        panic!("skills receipt")
    };
    assert!(result.failed());
    let error = NativeSkillManagedError::with_recovery(
        NativeSkillManagedErrorKind::Indeterminate,
        "retained".into(),
    );
    let expected = error.clone();
    let receipt = block_on(run(
        runtime(),
        workers.clone(),
        CancellationToken::new(),
        move |_| Err(NativeSkillsServiceError::Managed(error)),
    ));
    assert!(
        matches!(receipt, Err(Error::Skills(NativeSkillsServiceError::Managed(error))) if error == expected)
    );
    workers.close();
    workers.completion().wait_on_worker().unwrap();
}

#[test]
fn operation_panic_is_contained_before_lifecycle_fence_release() {
    let runtime = runtime();
    let workers = NativeOwnedWorkerScope::new();
    assert!(matches!(
        block_on(run(
            runtime.clone(),
            workers.clone(),
            CancellationToken::new(),
            |_| panic!("fixed failure")
        )),
        Err(Error::Unavailable)
    ));
    workers.close();
    workers.completion().wait_on_worker().unwrap();
    assert!(runtime.begin_quiescence().unwrap().try_retire().is_ok());
}

#[test]
fn direct_enum_sizes_are_bounded_before_control_retention() {
    assert!(
        validate_size(&NativeSkillsCommand::Install {
            arguments: "x".repeat(crate::MAX_NATIVE_SKILLS_COMMAND_BYTES + 1)
        })
        .is_err()
    );
    assert!(
        validate_size(&NativeSkillsCommand::Show {
            selector: "x".repeat(crate::MAX_NATIVE_SKILLS_SELECTOR_BYTES + 1)
        })
        .is_err()
    );
    assert!(
        validate_size(&NativeSkillsCommand::Create {
            arguments: "bad\0name".into()
        })
        .is_err()
    );
    assert!(validate_size(&NativeSkillsCommand::Path).is_ok());
}
