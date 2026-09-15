use super::*;
use crate::NativeOwnedWorkerScope;
use machine_god_core::{ModelEventStream, ModelProvider, ModelRequest, ProviderError};
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
