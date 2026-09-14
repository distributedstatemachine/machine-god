//! A candidate may have persisted metadata before native composition rejected it.
use super::Current;
use crate::session_store::FileSessionScanControl;
use machine_god_core::{BoxFuture, CancellationToken};

pub(super) fn revalidate(current: &Current) -> BoxFuture<'static, bool> {
    let expected = current.session.runtime().record_snapshot();
    let store = current.host.session_store().clone();
    let workers = current.host.control_workers();
    Box::pin(async move {
        let Some(workers) = workers else {
            return false;
        };
        // Nonblocking controlled locking keeps a foreign writer or unavailable
        // checkpoint distinct from evidence that the old actor remains usable.
        workers
            .run(move || {
                let control = FileSessionScanControl {
                    cancel: CancellationToken::new(),
                    abandoned: CancellationToken::new(),
                    #[cfg(test)]
                    after_read: None,
                };
                matches!(store.load_controlled(&expected.id, &control), Ok(Some(actual))
                if actual.id == expected.id && actual.incarnation_id == expected.incarnation_id
                    && actual.revision == expected.revision)
            })
            .await
            .unwrap_or(false)
    })
}
