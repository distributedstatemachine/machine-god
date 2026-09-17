use super::{
    Arc, BoxFuture, CancellationToken, JournalOwner, ManagedPreparation, ManagedRuntimeError,
    ManagedRuntimePreparationKind, ManagedRuntimeRequest, NativeSessionMetadata,
    PreparedManagedRuntime, Session, SharedManagedRuntimeFactoryOptions,
};
use crate::managed::manager::factory::ManagedPreparationReceipt;
use crate::session_lifecycle::NativeInitialSession;
use crate::session_store::FileSessionScanControl;
use crate::{NativeOwnedWorkerCompletion, owned_worker::NativeOwnedWorkerRun};
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
};

#[allow(clippy::too_many_lines)] // One attempt retains publication and worker settlement custody.
pub(super) fn prepare(
    factory: std::sync::Weak<SharedManagedRuntimeFactoryOptions>,
    request: ManagedRuntimeRequest,
    cancellation: CancellationToken,
) -> BoxFuture<'static, Result<ManagedPreparation, ManagedRuntimeError>> {
    Box::pin(async move {
        let factory = factory.upgrade().ok_or(ManagedRuntimeError::Unavailable)?;
        if cancellation.is_cancelled() {
            return Err(ManagedRuntimeError::Unavailable);
        }
        let selected = factory.selection(&request)?;
        let cohort = begin(&factory, &request.journal_owner)?;
        let completion = cohort.completion();
        let ready_completion = completion.clone();
        let control = control(cancellation);
        let _abandon = Abandon(control.abandoned.clone());
        #[cfg(test)]
        let worker_hook = take_worker_hook();
        let operation = async move {
            let workers = factory
                .services
                .control_workers
                .as_ref()
                .ok_or(ManagedRuntimeError::Invalid)?
                .clone();
            match request.kind {
                ManagedRuntimePreparationKind::Create => {
                    let metadata = NativeSessionMetadata::new(
                        selected.workspace.primary_identity(),
                        request.now_ms,
                        factory.origin,
                    )
                    .map_err(|_| ManagedRuntimeError::Invalid)?;
                    let lifecycle = factory.services.session_lifecycle.clone();
                    let transcript = request.transcript.clone();
                    let (initial, result) = workers
                        .run(move || {
                            #[cfg(test)]
                            if let Some(hook) = worker_hook {
                                hook();
                            }
                            let mut initial = lifecycle
                                .prepare_initial_controlled(
                                    transcript.session_id,
                                    transcript.incarnation,
                                    metadata,
                                    &control,
                                )
                                .map_err(|_| ManagedRuntimeError::Persistence)?;
                            let result =
                                futures_executor::block_on(initial.publish_controlled(control));
                            Ok::<_, ManagedRuntimeError>((Arc::new(Mutex::new(initial)), result))
                        })
                        .await
                        .map_err(|_| ManagedRuntimeError::Unavailable)??;
                    if let Ok(session) = result {
                        #[cfg(test)]
                        let rejected = super::tests::reject_publication_observation();
                        #[cfg(not(test))]
                        let rejected = false;
                        if !rejected
                            && let Ok(runtime) =
                                factory.compose(&request, session, completion.clone())
                        {
                            return Ok(ManagedPreparation::Ready(runtime));
                        }
                    }
                    Ok(ManagedPreparation::Ambiguous(Box::new(Receipt {
                        factory,
                        request,
                        initial: Some(initial),
                        previous: completion,
                        waiting: None,
                        attempt: None,
                        handoff: None,
                        finished: false,
                    })))
                }
                ManagedRuntimePreparationKind::Restore => {
                    // Exact load only. Missing or incompatible records never create a replacement.
                    let lifecycle = factory.services.session_lifecycle.clone();
                    let id = request.transcript.session_id.clone();
                    let original_control = control.clone();
                    let session = workers
                        .run(move || {
                            #[cfg(test)]
                            if let Some(hook) = worker_hook {
                                hook();
                            }
                            futures_executor::block_on(lifecycle.resume_controlled(id, control))
                        })
                        .await
                        .map_err(|_| ManagedRuntimeError::Unavailable)?
                        .map_err(|_| ManagedRuntimeError::Missing)?;
                    original_control
                        .check()
                        .map_err(|_| ManagedRuntimeError::Unavailable)?;
                    factory
                        .compose(&request, session, completion)
                        .map(ManagedPreparation::Ready)
                }
            }
        };
        let result = Attributed::new(cohort, Box::pin(operation)).await;
        // Errors and ambiguous receipts also wait for actual worker/TLS settlement.
        // A response is not a collector join, and no failed attempt releases its
        // journal-owner keepalive before the original worker actually exits.
        ready_completion.wait().await;
        result
    })
}

pub(super) fn begin(
    factory: &SharedManagedRuntimeFactoryOptions,
    journal_owner: &JournalOwner,
) -> Result<Arc<NativeOwnedWorkerRun>, ManagedRuntimeError> {
    factory
        .services
        .control_workers
        .as_ref()
        .ok_or(ManagedRuntimeError::Invalid)?
        .begin_run_with_keepalive(Arc::new(journal_owner.clone()))
        .map(Arc::new)
        .map_err(|_| ManagedRuntimeError::Capacity)
}

/// Each poll and actual future destruction retains original attribution. Closing
/// admission never substitutes for the independently retained completion receipt.
pub(super) struct Attributed<T> {
    cohort: Arc<NativeOwnedWorkerRun>,
    future: Option<BoxFuture<'static, T>>,
}
impl<T> Attributed<T> {
    pub(super) fn new(cohort: Arc<NativeOwnedWorkerRun>, future: BoxFuture<'static, T>) -> Self {
        Self {
            cohort,
            future: Some(future),
        }
    }
}
impl<T> Future for Attributed<T> {
    type Output = T;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let cohort = self.cohort.clone();
        let result = cohort.with_poll(|| {
            self.future
                .as_mut()
                .expect("attributed future polled after completion")
                .as_mut()
                .poll(cx)
        });
        if result.is_ready() {
            cohort.with_poll(|| drop(self.future.take()));
            cohort.close();
        }
        result
    }
}
impl<T> Drop for Attributed<T> {
    fn drop(&mut self) {
        self.cohort.with_poll(|| drop(self.future.take()));
        self.cohort.close();
    }
}

type Reconciled = Result<Option<Session>, ManagedRuntimeError>;
struct Receipt {
    factory: Arc<SharedManagedRuntimeFactoryOptions>,
    request: ManagedRuntimeRequest,
    initial: Option<Arc<Mutex<NativeInitialSession>>>,
    previous: NativeOwnedWorkerCompletion,
    waiting: Option<BoxFuture<'static, ()>>,
    attempt: Option<Attributed<Reconciled>>,
    handoff: Option<BoxFuture<'static, Option<PreparedManagedRuntime>>>,
    finished: bool,
}
impl ManagedPreparationReceipt for Receipt {
    fn poll_reconcile(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<PreparedManagedRuntime>, ManagedRuntimeError>> {
        if self.finished {
            return Poll::Ready(Err(ManagedRuntimeError::Invalid));
        }
        if self.handoff.is_some() {
            return self.poll_handoff(cx);
        }
        if self.attempt.is_none() {
            if self.waiting.is_none() {
                let previous = self.previous.clone();
                self.waiting = Some(Box::pin(async move {
                    previous.wait().await;
                }));
            }
            if self
                .waiting
                .as_mut()
                .expect("completion waiter")
                .as_mut()
                .poll(cx)
                .is_pending()
            {
                return Poll::Pending;
            }
            self.waiting = None;
            let cohort = match begin(&self.factory, &self.request.journal_owner) {
                Ok(cohort) => cohort,
                Err(error) => return Poll::Ready(Err(error)),
            };
            self.previous = cohort.completion();
            let initial = self
                .initial
                .as_ref()
                .expect("exact initial candidate retained between attempts")
                .clone();
            let workers = self
                .factory
                .services
                .control_workers
                .as_ref()
                .expect("validated factory worker scope")
                .clone();
            #[cfg(test)]
            let worker_hook = take_worker_hook();
            self.attempt = Some(Attributed::new(
                cohort,
                Box::pin(async move {
                    // The receipt retains its original candidate even if worker
                    // admission fails. This is confirmation, never create replay.
                    let control = control(CancellationToken::new());
                    let _abandon = Abandon(control.abandoned.clone());
                    workers
                        .run(move || {
                            #[cfg(test)]
                            if let Some(hook) = worker_hook {
                                hook();
                            }
                            let mut initial =
                                initial.lock().map_err(|_| ManagedRuntimeError::Ambiguous)?;
                            futures_executor::block_on(initial.reconcile_controlled(control))
                                .map_err(|_| ManagedRuntimeError::Ambiguous)
                        })
                        .await
                        .map_err(|_| ManagedRuntimeError::Ambiguous)?
                }),
            ));
        }
        let Poll::Ready(result) =
            Pin::new(self.attempt.as_mut().expect("reconciliation attempt")).poll(cx)
        else {
            return Poll::Pending;
        };
        self.attempt = None;
        match result {
            Ok(Some(session)) => {
                match self
                    .factory
                    .compose(&self.request, session, self.previous.clone())
                {
                    Ok(runtime) => {
                        self.initial = None;
                        self.begin_handoff(Some(runtime));
                        self.poll_handoff(cx)
                    }
                    Err(error) => Poll::Ready(Err(error)),
                }
            }
            Ok(None) => {
                self.initial = None;
                self.begin_handoff(None);
                self.poll_handoff(cx)
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

fn control(cancellation: CancellationToken) -> Arc<FileSessionScanControl> {
    Arc::new(FileSessionScanControl {
        cancel: cancellation,
        abandoned: CancellationToken::new(),
        #[cfg(test)]
        after_read: None,
    })
}
struct Abandon(CancellationToken);
impl Drop for Abandon {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(test)]
type WorkerHook = Arc<dyn Fn() + Send + Sync>;
#[cfg(test)]
thread_local! { static WORKER_HOOK: std::cell::RefCell<Option<WorkerHook>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn set_worker_hook(hook: WorkerHook) {
    WORKER_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(hook).is_none());
    });
}
#[cfg(test)]
fn take_worker_hook() -> Option<WorkerHook> {
    WORKER_HOOK.with(|slot| slot.borrow_mut().take())
}

impl Receipt {
    fn begin_handoff(&mut self, runtime: Option<PreparedManagedRuntime>) {
        let completion = self.previous.clone();
        self.handoff = Some(Box::pin(async move {
            completion.wait().await;
            runtime
        }));
    }

    fn poll_handoff(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<PreparedManagedRuntime>, ManagedRuntimeError>> {
        let result = self
            .handoff
            .as_mut()
            .expect("prepared handoff")
            .as_mut()
            .poll(cx);
        if result.is_ready() {
            self.handoff = None;
            self.finished = true;
        }
        result.map(Ok)
    }
}
