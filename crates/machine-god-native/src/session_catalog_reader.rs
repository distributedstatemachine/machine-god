//! Owned cancellable reads of the host's exact authoritative session catalog.

use crate::{
    FileSessionStore, NativeOwnedWorkerScope, NativeSessionCatalogCursor,
    NativeSessionCatalogError, NativeSessionCatalogInvalidRecords, NativeSessionCatalogPage,
    NativeSessionCatalogQuery,
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Picker scope; a workspace association is a filter, not filesystem authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionCatalogScope {
    CurrentWorkspace,
    All,
}

/// Fixed, data-free outcomes. Cancelled or busy scans never publish partial pages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionCatalogReadError {
    Catalog(NativeSessionCatalogError),
    Busy,
    Cancelled,
    Unavailable,
}
impl fmt::Display for NativeSessionCatalogReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog(error) => error.fmt(f),
            Self::Busy => f.write_str("session catalog is busy"),
            Self::Cancelled => f.write_str("session catalog read was cancelled"),
            Self::Unavailable => f.write_str("session catalog reader is unavailable"),
        }
    }
}
impl std::error::Error for NativeSessionCatalogReadError {}

/// Clones share one scan admission. This retains no engine or host-lifetime vote.
#[derive(Clone)]
pub struct NativeSessionCatalogReader {
    inner: Arc<ReaderInner>,
}
struct ReaderInner {
    store: Arc<FileSessionStore>,
    workspace: PathBuf,
    workers: NativeOwnedWorkerScope,
    active: Arc<AtomicBool>,
    #[cfg(test)]
    probe: Option<Arc<tests::Probe>>,
}
impl fmt::Debug for NativeSessionCatalogReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSessionCatalogReader")
            .finish_non_exhaustive()
    }
}
impl NativeSessionCatalogReader {
    #[cfg(any(feature = "ai-gateway-http", test))]
    pub(crate) fn new(
        store: Arc<FileSessionStore>,
        workspace: PathBuf,
        workers: NativeOwnedWorkerScope,
    ) -> Self {
        Self {
            inner: Arc::new(ReaderInner {
                store,
                workspace,
                workers,
                active: Arc::new(AtomicBool::new(false)),
                #[cfg(test)]
                probe: None,
            }),
        }
    }

    /// Inert until first poll. Runs the existing bounded scanner on the actual
    /// host worker scope, with nonblocking record locks and cancellation checks.
    /// Dropping a polled response requests private cancellation, not caller-token
    /// cancellation or host-scope closure. Filesystem calls and bounded decoding
    /// do not have a hard wall-clock deadline. Full worker join is separate.
    #[must_use]
    pub fn list(
        &self,
        scope: NativeSessionCatalogScope,
        limit: usize,
        continuation: Option<NativeSessionCatalogCursor>,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            use NativeSessionCatalogReadError as Error;
            let mut query = NativeSessionCatalogQuery::new(limit).map_err(Error::Catalog)?;
            if scope == NativeSessionCatalogScope::CurrentWorkspace {
                query = query
                    .with_workspace(&inner.workspace)
                    .map_err(Error::Catalog)?;
            }
            if let Some(cursor) = continuation {
                query = query.with_continuation(cursor);
            }
            query = query.with_invalid_records(NativeSessionCatalogInvalidRecords::SkipAndReport);
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            inner
                .active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| Error::Busy)?;
            let admission = Admission(Arc::clone(&inner.active));
            let abandoned = CancellationToken::new();
            let guard = CancelOnDrop(abandoned.clone());
            let control = crate::session_store::FileSessionScanControl {
                cancel,
                abandoned,
                #[cfg(test)]
                after_read: None,
            };
            let workers = inner.workers.clone();
            let result = workers
                .run(move || {
                    let _admission = admission;
                    #[cfg(test)]
                    if let Some(probe) = &inner.probe {
                        probe.enter();
                    }
                    crate::session_catalog::list_from_store_with_control(
                        &inner.store,
                        &query,
                        Some(&control),
                    )
                })
                .await
                .map_err(|_| Error::Unavailable)?;
            drop(guard);
            result
        })
    }
}
struct Admission(Arc<AtomicBool>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(test)]
mod tests;
