//! Canonical transcript reads keep the original journal/worker custody to join.
use super::{SharedManagedRuntimeFactory, preparation};
use crate::reference_host::managed_agents::history::{
    self, NativeManagedHistoryError as Error, NativeManagedHistorySnapshot, Reservation,
};
use crate::session_store::{FileSessionScanControl, FileSessionScanError};
use crate::{NativeObservedManagedAgent, managed::store::ManagedJournal};
use machine_god_core::{BoxFuture, CancellationToken, SessionRecord};
use std::sync::Arc;

impl SharedManagedRuntimeFactory {
    pub(in crate::reference_host) fn read_history(
        &self,
        journal: ManagedJournal,
        observed: NativeObservedManagedAgent,
        resident: Option<Arc<SessionRecord>>,
        reservation: Arc<Reservation>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeManagedHistorySnapshot, Error>> {
        let factory = Arc::downgrade(&self.0);
        Box::pin(async move {
            check(&cancellation)?;
            let factory = factory.upgrade().ok_or(Error::Closed)?;
            let cohort = Arc::new(
                factory
                    .services
                    .control_workers
                    .as_ref()
                    .ok_or(Error::Unavailable)?
                    .begin_run_with_keepalive(Arc::new((
                        journal.owner_lease(),
                        reservation.clone(),
                    )))
                    .map_err(|_| Error::Busy)?,
            );
            let completion = cohort.completion();
            let result = preparation::Attributed::new(
                cohort,
                Box::pin(async move {
                    let workers = factory
                        .services
                        .control_workers
                        .as_ref()
                        .ok_or(Error::Unavailable)?;
                    let original = journal
                        .inspect_in_run(workers, observed.id.clone())
                        .map_err(|_| Error::Unavailable)?
                        .await
                        .map_err(|_| Error::Unavailable)?;
                    history::validate(&observed, &original)?;
                    check(&cancellation)?;
                    let record = if let Some(record) = resident {
                        record
                    } else {
                        let store = factory.services.session_lifecycle.session_store().clone();
                        let id = original.head.transcript.session_id.clone();
                        let control = FileSessionScanControl {
                            cancel: cancellation.clone(),
                            abandoned: CancellationToken::new(),
                            #[cfg(test)]
                            after_read: None,
                        };
                        workers
                            .run(move || {
                                store
                                    .load_controlled(&id, &control)
                                    .map_err(|error| match error {
                                        FileSessionScanError::Busy => Error::Busy,
                                        FileSessionScanError::Cancelled => Error::Cancelled,
                                        FileSessionScanError::Store(_) => Error::Unavailable,
                                    })
                            })
                            .await
                            .map_err(|_| Error::Unavailable)??
                            .map(Arc::new)
                            .ok_or(Error::Unavailable)?
                    };
                    check(&cancellation)?;
                    let current = journal
                        .inspect_in_run(workers, observed.id.clone())
                        .map_err(|_| Error::Unavailable)?
                        .await
                        .map_err(|_| Error::Unavailable)?;
                    history::validate(&observed, &current)?;
                    check(&cancellation)?;
                    history::snapshot(record, observed, &original, reservation)
                }),
            )
            .await;
            // The response never substitutes for actual TLS/collector completion.
            completion.wait().await;
            result
        })
    }
}
fn check(cancellation: &CancellationToken) -> Result<(), Error> {
    if cancellation.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
