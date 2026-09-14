use super::*;
use crate::acp::selection::{
    NativeAcpSelectionOutcome, NativeAcpSelectionOwner, tests::fixture::Factory,
};
use crate::acp::session::NativeAcpSessionSelection;
use std::{sync::atomic::Ordering, task::Poll};

fn run(test: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), test)
                .await
                .unwrap();
        });
}
async fn open(factory: Arc<Factory>) -> NativeAcpSelectionOwner {
    let mut selection = NativeAcpSelectionOwner::new(factory.clone());
    selection
        .request(
            NativeAcpSessionSelection::New,
            factory.workspace.clone(),
            crate::mcp::ephemeral::NativeMcpEphemeralConfiguration::decode(None).unwrap(),
            100,
        )
        .unwrap();
    let outcome = futures_util::future::poll_fn(|cx| {
        let _ = selection.poll_progress(cx, 100);
        selection.take_outcome().map_or(Poll::Pending, Poll::Ready)
    })
    .await;
    assert!(matches!(
        outcome,
        NativeAcpSelectionOutcome::Selected { .. }
    ));
    selection
}
async fn receipt(selection: &mut NativeAcpSelectionOwner) -> NativeInteractiveControlOutcome {
    futures_util::future::poll_fn(|cx| {
        let _ = selection.poll_progress(cx, 100);
        selection
            .current_mut()
            .unwrap()
            .take_command_control_outcome()
            .map_or(Poll::Pending, Poll::Ready)
    })
    .await
}
async fn close(selection: &mut NativeAcpSelectionOwner) {
    selection.request_shutdown();
    futures_util::future::poll_fn(|cx| {
        let _ = selection.poll_progress(cx, 100);
        let _ = selection.take_outcome();
        if selection.is_closed() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

#[test]
fn observations_and_advertisements_use_injected_authority_without_provider_work() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut selection = open(factory.clone()).await;
        let session = selection.current_mut().unwrap();
        let original = session.runtime().record_snapshot();
        let catalog = available_commands(session);
        let names: Vec<_> = catalog["availableCommands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"compact"));
        assert!(names.contains(&"mcp"));
        for absent in [
            "new", "clear", "reset", "skills", "fast", "models", "credits", "review",
        ] {
            assert!(!names.contains(&absent));
        }
        let mut owner = NativeAcpCommandOwner::new();
        for input in ["/help", "/status", "/permissions", "/allowlist", "/mcp"] {
            owner
                .begin(session, &session.id(), command(input), 100)
                .unwrap();
            let result = owner.take_result().unwrap();
            assert!(!result.failed());
            assert_eq!(result.principal(), &session.principal());
            assert!(
                serde_json::to_vec(&result.update()).unwrap().len() < MAX_ACP_COMMAND_OUTPUT_BYTES
            );
        }
        assert_eq!(session.runtime().record_snapshot(), original);
        assert!(!factory.provider_started.load(Ordering::Acquire));
        close(&mut selection).await;
    });
}

#[test]
fn real_control_receipts_cancellation_and_foreign_receipts_keep_exact_custody() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut selection = open(factory.clone()).await;
        let mut owner = NativeAcpCommandOwner::new();
        let session = selection.current_mut().unwrap();
        owner
            .begin(session, &session.id(), command("/compact"), 100)
            .unwrap();
        assert!(session.has_pending_command_control());
        assert!(owner.take_result().is_none());
        assert!(session.take_model_save_outcome().is_none());
        assert!(owner.cancel(session).unwrap());
        assert!(session.cancellation_requested());
        let mut received = Some(receipt(&mut selection).await);
        assert!(!selection.current().unwrap().cancellation_requested());
        assert!(!owner.cancel(selection.current_mut().unwrap()).unwrap());
        let old_id = received.as_ref().unwrap().id;
        owner.complete(&mut received).unwrap();
        assert!(received.is_none());
        let result = owner.take_result().unwrap();
        assert!(result.cancelled());
        assert!(
            result.update()["command_result"]["receipt"]
                .get("changed")
                .is_some()
        );
        let session = selection.current_mut().unwrap();
        owner
            .begin(session, &session.id(), command("/undo"), 100)
            .unwrap();
        let foreign = BackgroundOutputOwner::new(
            session.id(),
            machine_god_core::SessionIncarnationId::new("foreign").unwrap(),
        );
        assert_eq!(
            owner.note_cancellation_requested(&foreign),
            Err(Error::WrongSession)
        );
        assert!(!owner.pending.as_ref().unwrap().cancellation_requested);
        assert!(
            owner
                .note_cancellation_requested(&session.principal())
                .unwrap()
        );
        assert!(
            !session.cancellation_requested(),
            "annotation must not perform native cancellation"
        );
        let mut wrong = Some(NativeInteractiveControlOutcome {
            id: old_id,
            source: session.principal(),
            result: Ok(crate::NativeInteractiveControlReceipt::Compacted(false)),
        });
        assert_eq!(owner.complete(&mut wrong), Err(Error::WrongSession));
        assert!(wrong.is_some());
        assert!(owner.has_pending());
        let mut real = Some(receipt(&mut selection).await);
        let source = real.as_ref().unwrap().source.clone();
        real.as_mut().unwrap().source = BackgroundOutputOwner::new(
            source.session_id().clone(),
            machine_god_core::SessionIncarnationId::new("foreign").unwrap(),
        );
        assert_eq!(owner.complete(&mut real), Err(Error::WrongSession));
        assert!(real.is_some());
        real.as_mut().unwrap().source = source;
        owner.complete(&mut real).unwrap();
        let result = owner.take_result().unwrap();
        assert!(result.cancelled());
        assert_eq!(
            result.update()["command_result"]["receipt"]["outcome"],
            "empty"
        );
        assert!(
            !owner
                .note_cancellation_requested(result.principal())
                .unwrap()
        );
        assert!(!factory.provider_started.load(Ordering::Acquire));
        close(&mut selection).await;
    });
}

#[test]
fn model_changes_report_acceptance_and_actual_session_save_without_user_defaults() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut selection = open(factory.clone()).await;
        let mut owner = NativeAcpCommandOwner::new();
        let session = selection.current_mut().unwrap();
        let id = session.id();
        owner
            .begin(session, &id, command("/model fixture/changed"), 100)
            .unwrap();
        assert_eq!(
            session.runtime().model_preferences().model(),
            "fixture/changed"
        );
        assert!(session.request_model_save(&id, 100).is_err());
        assert!(owner.begin(session, &id, command("/status"), 100).is_err());
        let mut received = Some(receipt(&mut selection).await);
        owner.complete(&mut received).unwrap();
        let result = owner.take_result().unwrap();
        assert!(!result.failed());
        assert_eq!(
            result.update()["command_result"]["receipt"]["persistence"],
            "saved"
        );
        assert!(result.update()["command_result"]["receipt"]["acceptedGeneration"].is_u64());
        assert_eq!(selection.current().unwrap().id(), id);
        assert!(!factory.provider_started.load(Ordering::Acquire));
        close(&mut selection).await;
    });
}

#[test]
fn shutdown_and_abandoned_presentation_keep_native_command_receipts_drainable() {
    run(async {
        let factory = Arc::new(Factory::new());
        let mut selection = open(factory.clone()).await;
        let mut owner = NativeAcpCommandOwner::new();
        let session = selection.current_mut().unwrap();
        owner
            .begin(session, &session.id(), command("/compact"), 100)
            .unwrap();
        let expected = owner.pending_control().unwrap();
        drop(owner);
        selection.request_shutdown();
        let outcome = futures_util::future::poll_fn(|cx| {
            let _ = selection.poll_progress(cx, 100);
            assert!(
                !selection.is_closed(),
                "retirement must wait for accepted receipt drain"
            );
            selection
                .take_command_control_outcome()
                .map_or(Poll::Pending, Poll::Ready)
        })
        .await;
        assert_eq!(outcome.id, expected.0);
        assert_eq!(outcome.source, expected.1);
        assert!(matches!(
            outcome.result,
            Ok(crate::NativeInteractiveControlReceipt::Compacted(_))
        ));
        close(&mut selection).await;
        assert!(!factory.provider_started.load(Ordering::Acquire));
    });
}
