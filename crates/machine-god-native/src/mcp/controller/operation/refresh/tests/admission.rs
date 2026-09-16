//! The actual conversation admission must remain cancellable before a core turn.

use super::*;
use crate::{
    NativeConversation, NativeConversationError, NativeConversationRuntime,
    NativeConversationRuntimeError, NativeModelPreferences, NativeReasoningEffort,
};
use machine_god_core::{Engine, SessionId, SessionIncarnationId};
use machine_god_testkit::{InMemorySessionStore, ScriptedModelProvider, ScriptedPermissionHandler};
use std::task::Poll;

#[test]
fn active_cancel_releases_admission_waiting_for_shared_authentication_refresh() {
    run(cancel_admission(false));
}

#[test]
fn quiescence_releases_admission_waiting_for_shared_authentication_refresh() {
    run(cancel_admission(true));
}

#[test]
fn owner_settles_abandoned_refresh_without_closing_persistent_publication() {
    run(async {
        let fixture = Fixture::new();
        let controller = Arc::new(fixture.controller());
        controller
            .start_configured(NativeMcpStartupPhase::AskStartup, CancellationToken::new())
            .await
            .unwrap();
        controller.inner.release_completed();
        let source = lock(&controller.inner.state).active.clone().unwrap();
        let cohort = fixture.options.workers.begin_run().unwrap();
        let completion = cohort.completion();
        let (release, receive) = std::sync::mpsc::channel::<()>();
        cohort
            .with_poll(|| {
                fixture.options.workers.spawn(move || {
                    // An unpublished peer cannot reap until its retained startup future
                    // relinquishes the corresponding owner.
                    let _ = receive.recv();
                })
            })
            .unwrap();
        let job_cancel = CancellationToken::new();
        let cancel = job_cancel.clone();
        let future: machine_god_core::BoxFuture<'static, _> = Box::pin(async move {
            let _custody = release;
            cancel.cancelled().await;
            Err(NativeMcpControllerError::Cancelled.into())
        });
        lock(&controller.inner.state).running = Some(Running {
            kind: Kind::Refresh,
            generation: generation(NativeMcpStartupPhase::AskStartup),
            refresh_source: Some(source.clone()),
            cancellation: job_cancel.clone(),
            future: future.shared(),
        });
        let (runtime, provider) = conversation(&controller);
        runtime.enqueue("cancel this admission".into()).unwrap();
        let mut admission = runtime.start_next(200);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(admission.as_mut().poll(&mut cx).is_pending());
        assert!(runtime.request_active_cancel());
        assert!(matches!(
            admission.as_mut().poll(&mut cx),
            Poll::Ready(Err(_))
        ));
        drop(admission);
        cohort.close();
        assert!(!completion.is_complete());
        assert!(
            !job_cancel.is_cancelled(),
            "observer cancellation is not job ownership"
        );
        drop(controller.settle_abandoned_admission());
        assert!(!job_cancel.is_cancelled(), "unpolled cleanup must be inert");
        controller.settle_abandoned_admission().await;
        completion.wait().await;
        assert!(job_cancel.is_cancelled());
        assert!(provider.requests().is_empty());
        {
            let state = lock(&controller.inner.state);
            assert!(state.running.is_none());
            assert!(!state.closed);
            assert!(Arc::ptr_eq(state.active.as_ref().unwrap(), &source));
        }
        assert!(!source.cancellation.is_cancelled());
        controller
            .refresh_authentication_configured(CancellationToken::new())
            .await
            .unwrap();
        controller.close();
    });
}

#[test]
fn owner_drives_shared_deferred_attempt_without_cancelling_published_generation() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        controller
            .start_configured(NativeMcpStartupPhase::AskStartup, CancellationToken::new())
            .await
            .unwrap();
        controller.inner.release_completed();
        let source = lock(&controller.inner.state).active.clone().unwrap();
        let job_cancel = CancellationToken::new();
        let (send, receive) = tokio::sync::oneshot::channel();
        let future: machine_god_core::BoxFuture<'static, _> = Box::pin(async move {
            receive.await.unwrap();
            Ok(unchanged())
        });
        lock(&controller.inner.state).running = Some(Running {
            kind: Kind::Deferred,
            generation: source.clone(),
            refresh_source: None,
            cancellation: job_cancel.clone(),
            future: future.shared(),
        });
        let mut settlement = controller.settle_abandoned_admission();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(settlement.as_mut().poll(&mut cx).is_pending());
        assert!(!job_cancel.is_cancelled());
        send.send(()).unwrap();
        settlement.await;
        assert!(!source.cancellation.is_cancelled());
        assert!(!job_cancel.is_cancelled());
        assert!(lock(&controller.inner.state).running.is_none());
        controller.close();
    });
}

async fn cancel_admission(quiesce: bool) {
    let fixture = Fixture::new();
    let controller = Arc::new(fixture.controller());
    controller
        .start_configured(NativeMcpStartupPhase::AskStartup, CancellationToken::new())
        .await
        .unwrap();
    controller.inner.release_completed();
    let source = lock(&controller.inner.state).active.clone().unwrap();
    let job_cancel = CancellationToken::new();
    let (send, receive) = tokio::sync::oneshot::channel();
    let future: machine_god_core::BoxFuture<'static, _> = Box::pin(async move {
        receive.await.unwrap();
        Ok(unchanged())
    });
    lock(&controller.inner.state).running = Some(Running {
        kind: Kind::Refresh,
        generation: generation(NativeMcpStartupPhase::AskStartup),
        refresh_source: Some(source),
        cancellation: job_cancel.clone(),
        future: future.shared(),
    });

    let (runtime, provider) = conversation(&controller);
    runtime
        .enqueue("cancel before provider work".into())
        .unwrap();
    let before = runtime.record();
    let mut admission = runtime.start_next(200);
    let mut cx = Context::from_waker(Waker::noop());
    assert!(admission.as_mut().poll(&mut cx).is_pending());
    assert!(runtime.status().active);
    // A second observer owns the same job, not this admission's cancellation.
    let mut other = controller.refresh_authentication_configured(CancellationToken::new());
    assert!(other.as_mut().poll(&mut cx).is_pending());
    let mut fence = if quiesce {
        Some(runtime.begin_quiescence().unwrap())
    } else {
        assert!(runtime.request_active_cancel());
        None
    };

    // No timer, network response or completion signal can release this waiter.
    assert!(matches!(
        admission.as_mut().poll(&mut cx),
        Poll::Ready(Err(NativeConversationRuntimeError::Conversation(
            NativeConversationError::McpRequiredUnavailable
        )))
    ));
    drop(admission);
    assert!(!runtime.status().active);
    assert_eq!(runtime.status().queued_jobs, 0);
    assert_eq!(runtime.record().revision, before.revision);
    assert_eq!(runtime.record().messages, before.messages);
    assert!(provider.requests().is_empty());
    if let Some(fence) = &mut fence {
        assert!(matches!(
            fence.wait_idle().as_mut().poll(&mut cx),
            Poll::Ready(Ok(()))
        ));
    }
    assert!(!job_cancel.is_cancelled());
    assert!(other.as_mut().poll(&mut cx).is_pending());
    send.send(()).unwrap();
    assert_eq!(
        other.await.unwrap().publication(),
        NativeMcpControllerPublication::Unchanged
    );
    assert!(!job_cancel.is_cancelled());
    drop(fence);
    controller.close();
}

fn conversation(
    controller: &Arc<crate::mcp::controller::NativeMcpController>,
) -> (NativeConversationRuntime, ScriptedModelProvider) {
    let provider = ScriptedModelProvider::new("fixture", []);
    let engine = Engine::builder()
        .session_store(InMemorySessionStore::default())
        .provider(provider.clone())
        .permission_handler(ScriptedPermissionHandler::new([]))
        .build()
        .unwrap();
    let session = engine
        .create_session(
            SessionId::new("refresh-admission").unwrap(),
            SessionIncarnationId::new("refresh-admission-life").unwrap(),
        )
        .unwrap();
    let conversation = NativeConversation::from_session(session)
        .unwrap()
        .with_mcp_readiness(controller)
        .unwrap();
    let runtime = NativeConversationRuntime::new(
        conversation,
        NativeModelPreferences::new("fixture/model", NativeReasoningEffort::default(), false)
            .unwrap(),
        None,
    )
    .unwrap();
    (runtime, provider)
}
