use super::{
    Arc, BoxFuture, CancellationToken, JournalOwner, ManagedPreparation, ManagedRuntimeError,
    ManagedRuntimePreparationKind, ManagedRuntimeRequest, NativeSessionMetadata,
    PreparedManagedRuntime, Session, SharedManagedRuntimeFactoryOptions,
};
use crate::managed::manager::factory::ManagedPreparationReceipt;
use crate::session_lifecycle::NativeInitialSession;
use crate::{NativeOwnedWorkerCompletion, owned_worker::NativeOwnedWorkerRun};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

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
        let operation = async move {
            match request.kind {
                ManagedRuntimePreparationKind::Create => {
                    let metadata = NativeSessionMetadata::new(
                        selected.workspace.primary_identity(),
                        request.now_ms,
                        factory.origin,
                    )
                    .map_err(|_| ManagedRuntimeError::Invalid)?;
                    let mut initial = factory
                        .services
                        .session_lifecycle
                        .prepare_initial(
                            request.transcript.session_id.clone(),
                            request.transcript.incarnation.clone(),
                            metadata,
                        )
                        .await
                        .map_err(|_| ManagedRuntimeError::Persistence)?;
                    let result = initial.publish().await;
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
                    let session = factory
                        .services
                        .session_lifecycle
                        .resume(request.transcript.session_id.clone())
                        .await
                        .map_err(|_| ManagedRuntimeError::Missing)?;
                    factory
                        .compose(&request, session, completion)
                        .map(ManagedPreparation::Ready)
                }
            }
        };
        let result = Attributed::new(cohort, Box::pin(operation)).await;
        if matches!(&result, Ok(ManagedPreparation::Ready(_))) {
            ready_completion.wait().await;
        }
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

type Reconciled = (
    NativeInitialSession,
    Result<Option<Session>, ManagedRuntimeError>,
);
struct Receipt {
    factory: Arc<SharedManagedRuntimeFactoryOptions>,
    request: ManagedRuntimeRequest,
    initial: Option<NativeInitialSession>,
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
            let mut initial = self
                .initial
                .take()
                .expect("exact initial candidate retained between attempts");
            self.attempt = Some(Attributed::new(
                cohort,
                Box::pin(async move {
                    let result = initial
                        .reconcile()
                        .await
                        .map_err(|_| ManagedRuntimeError::Ambiguous);
                    (initial, result)
                }),
            ));
        }
        let Poll::Ready((initial, result)) =
            Pin::new(self.attempt.as_mut().expect("reconciliation attempt")).poll(cx)
        else {
            return Poll::Pending;
        };
        self.attempt = None;
        self.initial = Some(initial);
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
