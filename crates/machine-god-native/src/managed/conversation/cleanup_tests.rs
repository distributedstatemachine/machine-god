use super::*;
use crate::NativeOwnedWorkerScope;
use machine_god_core::{ModelEventStream, ModelProvider, ModelRequest, ProviderError};
use machine_god_core::{
    SessionRecord, SessionRevision, SessionStore, SessionStoreError, SessionStoreErrorKind,
};
use std::{sync::mpsc, time::Duration};

struct DropWorker {
    scope: NativeOwnedWorkerScope,
    release: Mutex<Option<mpsc::Receiver<()>>>,
    admitted: mpsc::Sender<bool>,
}

impl DropWorker {
    fn spawn(&self) {
        let Some(release) = self.release.lock().unwrap().take() else {
            return;
        };
        let result = self.scope.spawn(move || {
            release.recv_timeout(Duration::from_secs(30)).unwrap();
        });
        self.admitted.send(result.is_ok()).unwrap();
    }
}

struct CleanupProvider {
    inner: ScriptedModelProvider,
    cleanup: Arc<DropWorker>,
}

impl ModelProvider for CleanupProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, std::result::Result<ModelEventStream, ProviderError>> {
        Box::pin(async move {
            let inner = self.inner.stream(request, cancellation).await?;
            Ok(Box::pin(CleanupStream {
                inner,
                cleanup: self.cleanup.clone(),
            }) as ModelEventStream)
        })
    }
}

struct CleanupStream {
    inner: ModelEventStream,
    cleanup: Arc<DropWorker>,
}

impl Stream for CleanupStream {
    type Item = std::result::Result<ModelEvent, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl Drop for CleanupStream {
    fn drop(&mut self) {
        self.cleanup.spawn();
    }
}

#[test]
fn actual_stream_destruction_enrolls_before_cohort_close_and_blocks_only_original_run() {
    let scope = NativeOwnedWorkerScope::new();
    let (release, receiver) = mpsc::channel();
    let (admitted, admission) = mpsc::channel();
    let cleanup = Arc::new(DropWorker {
        scope: scope.clone(),
        release: Mutex::new(Some(receiver)),
        admitted,
    });
    let fixture = Fixture::with_provider(
        vec![ModelProviderStep::pending(), completed(), completed()],
        |inner| CleanupProvider { inner, cleanup },
    );
    let (a, a_owner) = fixture.conversation("a");
    let (b, b_owner) = fixture.conversation("b");
    for owner in [&a_owner, &b_owner] {
        owner
            .configure_worker_binding(scope.clone(), Arc::new(()))
            .unwrap();
    }
    let mut turn = start(&a).unwrap();
    let first_run = a_owner.run().unwrap();
    let first_cleanup = a_owner.cleanup_for(&first_run).unwrap();
    assert!(b_owner.cleanup_for(&first_run).is_err());
    poll_until_pending(&mut turn);
    drop(turn);
    assert!(admission.recv_timeout(Duration::from_secs(5)).unwrap());
    assert!(!first_cleanup.completion().is_complete());
    // Even while the original worker is blocked, another actual principal's
    // completed run has no obligation to wait for shared-host completion.
    finish(start(&b).unwrap());
    let (second_run, second_settlement) = b_owner.take_settlement().unwrap();
    assert!(
        b_owner
            .cleanup_for(&second_run)
            .unwrap()
            .completion()
            .is_complete()
    );
    assert!(a_owner.cleanup_for(&second_run).is_err());
    second_settlement.complete().unwrap();
    assert!(start(&a).is_err());
    release.send(()).unwrap();
    first_cleanup.completion().wait_on_worker().unwrap();
    a_owner.take_settlement().unwrap().1.complete().unwrap();
    finish(start(&a).unwrap());
    assert!(a_owner.cleanup_for(&first_run).is_err());
    let (latest, settlement) = a_owner.take_settlement().unwrap();
    assert!(
        a_owner
            .cleanup_for(&latest)
            .unwrap()
            .completion()
            .is_complete()
    );
    settlement.complete().unwrap();
    scope.close();
    scope.completion().wait_on_worker().unwrap();
}

#[test]
fn unpolled_actual_turn_closes_empty_cohort_without_provider_work() {
    let fixture = Fixture::new(vec![completed()]);
    let (conversation, owner) = fixture.conversation("a");
    let scope = NativeOwnedWorkerScope::new();
    owner
        .configure_worker_binding(scope.clone(), Arc::new(()))
        .unwrap();
    let turn = start(&conversation).unwrap();
    let run = owner.run().unwrap();
    let completion = owner.cleanup_for(&run).unwrap().completion();
    assert!(!completion.is_complete());
    drop(turn);
    assert!(completion.is_complete());
    assert!(fixture.provider.requests().is_empty());
    owner.take_settlement().unwrap().1.complete().unwrap();
    scope.close();
}

struct CleanupStore {
    cleanup: Arc<DropWorker>,
    pending: bool,
}

impl SessionStore for CleanupStore {
    fn load(
        &self,
        _: SessionId,
    ) -> BoxFuture<'_, std::result::Result<Option<SessionRecord>, SessionStoreError>> {
        Box::pin(async { Ok(None) })
    }

    fn save(
        &self,
        _: SessionRecord,
        _: Option<SessionRevision>,
    ) -> BoxFuture<'_, std::result::Result<SessionRevision, SessionStoreError>> {
        Box::pin(CleanupSave {
            cleanup: self.cleanup.clone(),
            pending: self.pending,
            polled: false,
        })
    }
}

struct CleanupSave {
    cleanup: Arc<DropWorker>,
    pending: bool,
    polled: bool,
}

impl Future for CleanupSave {
    type Output = std::result::Result<SessionRevision, SessionStoreError>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        self.polled = true;
        if self.pending {
            Poll::Pending
        } else {
            Poll::Ready(Err(SessionStoreError::new(
                SessionStoreErrorKind::Other,
                "fixture",
                "fixture save failed",
                false,
            )))
        }
    }
}

impl Drop for CleanupSave {
    fn drop(&mut self) {
        if self.polled {
            self.cleanup.spawn();
        }
    }
}

#[test]
fn failed_and_dropped_pre_turn_checkpoint_keep_actual_admission_cleanup() {
    for pending in [false, true] {
        let scope = NativeOwnedWorkerScope::new();
        let (release, receiver) = mpsc::channel();
        let (admitted, admission) = mpsc::channel();
        let cleanup = Arc::new(DropWorker {
            scope: scope.clone(),
            release: Mutex::new(Some(receiver)),
            admitted,
        });
        let fixture = Fixture::with_adapters(
            vec![],
            |provider| provider,
            CleanupStore { cleanup, pending },
        );
        let (conversation, owner) = fixture.conversation("a");
        owner
            .configure_worker_binding(scope.clone(), Arc::new(()))
            .unwrap();
        let runtime = crate::NativeConversationRuntime::new(
            conversation,
            NativeModelPreferences::new("selected-model", NativeReasoningEffort::default(), false)
                .unwrap(),
            None,
        )
        .unwrap();
        runtime
            .enqueue(machine_god_core::Prompt {
                text: "work".into(),
                options: machine_god_core::InferenceOptions::default(),
            })
            .unwrap();
        let mut future = runtime.start_next(100);
        assert!(owner.binding().admission_completion().is_none());
        let outcome = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        if pending {
            assert!(outcome.is_pending());
        } else {
            assert!(matches!(outcome, Poll::Ready(Err(_))));
        }
        drop(future);
        assert!(admission.recv_timeout(Duration::from_secs(5)).unwrap());
        assert!(owner.run().is_none(), "no actual core turn was minted");
        assert!(fixture.provider.requests().is_empty());
        let completion = owner.binding().admission_completion().unwrap();
        assert!(!completion.is_complete());
        runtime
            .enqueue(machine_god_core::Prompt {
                text: "later".into(),
                options: machine_god_core::InferenceOptions::default(),
            })
            .unwrap();
        assert!(matches!(
            block_on(runtime.start_next(101)),
            Err(crate::NativeConversationRuntimeError::Conversation(
                NativeConversationError::ManagedAdmission
            ))
        ));
        assert_eq!(runtime.status().queued_jobs, 1);
        release.send(()).unwrap();
        completion.wait_on_worker().unwrap();
        scope.close();
        scope.completion().wait_on_worker().unwrap();
    }
}
