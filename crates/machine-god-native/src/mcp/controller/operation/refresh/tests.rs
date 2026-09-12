use super::*;
use crate::mcp::{
    controller::{
        NativeMcpControllerPublication,
        operation::{Signals, publish, unchanged},
        state::Running,
        tests::{Fixture, run},
    },
    startup::NativeMcpStartupPhase,
    store::NativeMcpConfigStoreError,
};
use futures_util::{FutureExt, future::join};
use machine_god_core::CancellationToken;
use std::{
    sync::Mutex,
    task::{Context, Waker},
};

fn generation(phase: NativeMcpStartupPhase) -> Arc<Generation> {
    Arc::new(Generation {
        phase,
        cancellation: CancellationToken::new(),
        loaded: Mutex::default(),
        deferred: Mutex::default(),
        workers: std::sync::atomic::AtomicUsize::new(0),
    })
}

#[test]
fn refresh_without_leases_is_inert_then_does_not_load_saved_changes() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        drop(controller.refresh_authentication_configured(CancellationToken::new()));
        assert!(lock(&controller.inner.state).generations.is_empty());
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            controller
                .refresh_authentication_configured(cancel)
                .await
                .unwrap_err()
                .kind(),
            NativeMcpControllerError::Cancelled
        );
        controller
            .start_configured(NativeMcpStartupPhase::AskStartup, CancellationToken::new())
            .await
            .unwrap();
        let source = lock(&controller.inner.state).active.clone().unwrap();
        fixture.seed("not JSON");
        let receipt = controller
            .refresh_authentication_configured(CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            receipt.publication(),
            NativeMcpControllerPublication::Unchanged
        );
        assert!(Arc::ptr_eq(
            &source,
            lock(&controller.inner.state).active.as_ref().unwrap()
        ));
        assert_eq!(lock(&controller.inner.state).generations.len(), 1);
        controller.close();
    });
}

#[test]
fn refresh_phase_preserves_lazy_startup_until_successful_deferred_admission() {
    let source = generation(NativeMcpStartupPhase::AskStartup);
    assert_eq!(phase(&source), NativeMcpStartupPhase::AskStartup);
    *lock(&source.deferred) = Some(Err(NativeMcpControllerError::Unavailable.into()));
    assert_eq!(phase(&source), NativeMcpStartupPhase::AskStartup);
    *lock(&source.deferred) = Some(Ok(unchanged()));
    assert_eq!(phase(&source), NativeMcpStartupPhase::All);
}

#[test]
fn refresh_replacement_rejects_saved_changes_before_candidate_startup() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        controller
            .start_configured(NativeMcpStartupPhase::AskStartup, CancellationToken::new())
            .await
            .unwrap();
        let source = lock(&controller.inner.state).active.clone().unwrap();
        let next = generation(NativeMcpStartupPhase::AskStartup);
        fixture.seed(r#"{"mcp":{"new":{"command":["must-not-execute"]}}}"#);
        let signals = Signals {
            job: CancellationToken::new(),
            caller: None,
        };
        let result = publish::replace(
            &Arc::downgrade(&controller.inner),
            &controller.inner.options,
            &next,
            Some(&source),
            &signals,
            None,
        )
        .await;
        assert_eq!(
            result.err().unwrap().kind,
            NativeMcpControllerError::Store(NativeMcpConfigStoreError::Conflict)
        );
        assert!(lock(&next.loaded).is_none());
        assert!(!source.cancellation.is_cancelled());
        assert!(Arc::ptr_eq(
            &source,
            lock(&controller.inner.state).active.as_ref().unwrap()
        ));
        controller.close();
    });
}

#[test]
fn refresh_observers_join_original_job_and_dropping_one_does_not_cancel_it() {
    run(async {
        let fixture = Fixture::new();
        let controller = fixture.controller();
        controller
            .start_configured(NativeMcpStartupPhase::AskStartup, CancellationToken::new())
            .await
            .unwrap();
        controller.inner.release_completed();
        let source = lock(&controller.inner.state).active.clone().unwrap();
        let next = generation(NativeMcpStartupPhase::AskStartup);
        let job_cancel = CancellationToken::new();
        let (send, receive) = tokio::sync::oneshot::channel();
        let future: machine_god_core::BoxFuture<'static, _> = Box::pin(async move {
            receive.await.unwrap();
            Ok(unchanged())
        });
        lock(&controller.inner.state).running = Some(Running {
            kind: Kind::Refresh,
            generation: next,
            refresh_source: Some(source),
            cancellation: job_cancel.clone(),
            future: future.shared(),
        });
        let mut first = controller.refresh_authentication_configured(CancellationToken::new());
        assert!(
            first
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(first);
        assert!(!job_cancel.is_cancelled());
        let (receipt, ()) = join(
            controller.refresh_authentication_configured(CancellationToken::new()),
            async move {
                send.send(()).unwrap();
            },
        )
        .await;
        assert_eq!(
            receipt.unwrap().publication(),
            NativeMcpControllerPublication::Unchanged
        );
        assert!(!job_cancel.is_cancelled());
        controller.close();
    });
}
