//! Bounded rich observations of authoritative native session metadata.

mod facade;
mod projection;
mod query;
pub use facade::{
    inspect_native_session_catalog_entry, inspect_process_session_catalog_entry,
    list_native_session_catalog, list_process_current_workspace_session_catalog,
    list_process_session_catalog,
};
pub use projection::{MAX_NATIVE_SESSION_PREVIEW_BYTES, NativeSessionCatalogEntry};
pub use query::{
    MAX_NATIVE_SESSION_CATALOG_QUERY_BYTES, NativeSessionCatalogInvalidRecords,
    NativeSessionCatalogQuery,
};

use crate::{FileSessionStore, MAX_LIST_SESSIONS, NativeSessionCatalogCursor};
use machine_god_core::{BoxFuture, SessionId, SessionStoreError, SessionStoreErrorKind};
use std::{fmt, sync::Arc};

/// Read-only catalog over the explicitly supplied existing store allocation.
/// Construction performs no filesystem, environment or clock operations.
pub struct NativeSessionCatalog {
    store: Arc<FileSessionStore>,
}
impl NativeSessionCatalog {
    #[must_use]
    pub const fn new(store: Arc<FileSessionStore>) -> Self {
        Self { store }
    }

    /// Observes all reached records before selecting the newest matching rows.
    /// Native I/O and advisory locks run synchronously on first poll, with the
    /// store's finite byte/entry bounds but no wall-clock deadline.
    #[must_use]
    pub fn list(
        &self,
        query: NativeSessionCatalogQuery,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogError>> {
        let store = Arc::clone(&self.store);
        Box::pin(async move { list_from_store(&store, &query) })
    }

    /// Exact lookup is independent of directory and presentation truncation.
    /// No record or lock is created when the selected ID is absent.
    #[must_use]
    pub fn exact(
        &self,
        id: SessionId,
    ) -> BoxFuture<'static, Result<Option<NativeSessionCatalogEntry>, NativeSessionCatalogError>>
    {
        let store = Arc::clone(&self.store);
        Box::pin(async move { exact_from_store(&store, &id) })
    }
}
impl fmt::Debug for NativeSessionCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSessionCatalog { .. }")
    }
}

/// Ranked observations, not an index, continuation token, or atomic store snapshot.
#[derive(Debug)]
pub struct NativeSessionCatalogPage {
    entries: Vec<NativeSessionCatalogEntry>,
    scan_complete: bool,
    matched_count: usize,
    unknown_activity_count: usize,
    scanned_records: usize,
    scanned_record_bytes: usize,
    skipped_invalid: usize,
}
impl NativeSessionCatalogPage {
    fn empty() -> Self {
        Self {
            entries: Vec::new(),
            scan_complete: true,
            matched_count: 0,
            unknown_activity_count: 0,
            scanned_records: 0,
            scanned_record_bytes: 0,
            skipped_invalid: 0,
        }
    }
    #[must_use]
    pub fn entries(&self) -> &[NativeSessionCatalogEntry] {
        &self.entries
    }
    #[must_use]
    pub const fn scan_complete(&self) -> bool {
        self.scan_complete
    }
    /// More matching records were observed than the requested display limit.
    /// This differs from failing to observe the entire bounded candidate set.
    #[must_use]
    pub fn results_truncated(&self) -> bool {
        self.matched_count > self.entries.len()
    }
    #[must_use]
    pub const fn matched_count(&self) -> usize {
        self.matched_count
    }
    #[must_use]
    pub const fn unknown_activity_count(&self) -> usize {
        self.unknown_activity_count
    }
    #[must_use]
    pub const fn scanned_records(&self) -> usize {
        self.scanned_records
    }
    #[must_use]
    pub const fn scanned_record_bytes(&self) -> usize {
        self.scanned_record_bytes
    }

    #[must_use]
    pub const fn skipped_invalid(&self) -> usize {
        self.skipped_invalid
    }

    /// No cursor is issued from an incomplete scan. This boundary does not bind
    /// a snapshot: the same predicates must be reapplied to subsequent scans.
    #[must_use]
    pub fn next_cursor(&self) -> Option<NativeSessionCatalogCursor> {
        if !self.scan_complete || !self.results_truncated() {
            return None;
        }
        self.entries.last().map(|entry| {
            NativeSessionCatalogCursor::new(
                entry.native_metadata().updated_at_ms(),
                entry.id().clone(),
            )
        })
    }

    /// Selects the newest eligible observed row only when the complete scan and
    /// authoritative activity times make that ranking knowable. Writers may
    /// still change records after any individual locked observation.
    /// # Errors
    /// Refuses incomplete scanning, skipped invalid candidates or any matching
    /// unknown activity timestamp.
    pub fn latest(
        &self,
    ) -> Result<Option<&NativeSessionCatalogEntry>, NativeSessionSelectionIncomplete> {
        if !self.scan_complete {
            return Err(NativeSessionSelectionIncomplete::ScanIncomplete);
        }
        if self.skipped_invalid != 0 {
            return Err(NativeSessionSelectionIncomplete::SkippedInvalid);
        }
        if self.unknown_activity_count != 0 {
            return Err(NativeSessionSelectionIncomplete::UnknownActivity);
        }
        Ok(self.entries.first())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionSelectionIncomplete {
    ScanIncomplete,
    UnknownActivity,
    SkippedInvalid,
}
impl fmt::Display for NativeSessionSelectionIncomplete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ScanIncomplete => "session selection scan is incomplete",
            Self::UnknownActivity => "session activity ordering is unknown",
            Self::SkippedInvalid => "session selection omitted invalid records",
        })
    }
}
impl std::error::Error for NativeSessionSelectionIncomplete {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionCatalogErrorKind {
    InvalidQuery,
    InvalidEnvironment,
    UnsafeStateRoot,
    Corrupt,
    Unavailable,
}
impl NativeSessionCatalogErrorKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidQuery => "invalid_query",
            Self::InvalidEnvironment => "invalid_environment",
            Self::UnsafeStateRoot => "unsafe_state_root",
            Self::Corrupt => "corrupt",
            Self::Unavailable => "unavailable",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSessionCatalogError {
    kind: NativeSessionCatalogErrorKind,
}
impl NativeSessionCatalogError {
    const fn new(kind: NativeSessionCatalogErrorKind) -> Self {
        Self { kind }
    }
    #[must_use]
    pub const fn kind(self) -> NativeSessionCatalogErrorKind {
        self.kind
    }
}
impl fmt::Display for NativeSessionCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.kind {
            NativeSessionCatalogErrorKind::InvalidQuery => {
                "native session catalog query is invalid"
            }
            NativeSessionCatalogErrorKind::InvalidEnvironment => {
                "native session environment selection is invalid"
            }
            NativeSessionCatalogErrorKind::UnsafeStateRoot => "native session state root is unsafe",
            NativeSessionCatalogErrorKind::Corrupt => "native session record is corrupt",
            NativeSessionCatalogErrorKind::Unavailable => {
                "native session persistence is unavailable"
            }
        })
    }
}
impl std::error::Error for NativeSessionCatalogError {}

fn list_from_store(
    store: &FileSessionStore,
    query: &NativeSessionCatalogQuery,
) -> Result<NativeSessionCatalogPage, NativeSessionCatalogError> {
    list_from_store_with_control(store, query, None).map_err(|error| match error {
        crate::session_catalog_reader::NativeSessionCatalogReadError::Catalog(error) => error,
        _ => unreachable!("ordinary catalog scans have no cancellation policy"),
    })
}

pub(crate) fn list_from_store_with_control(
    store: &FileSessionStore,
    query: &NativeSessionCatalogQuery,
    control: Option<&crate::session_store::FileSessionScanControl>,
) -> Result<NativeSessionCatalogPage, crate::session_catalog_reader::NativeSessionCatalogReadError>
{
    use crate::session_catalog_reader::NativeSessionCatalogReadError as ReadError;
    use crate::session_store::FileSessionScanError;
    let mut page = NativeSessionCatalogPage::empty();
    let mut scratch = Vec::new();
    let visit = |record: &machine_god_core::SessionRecord| {
        let entry = NativeSessionCatalogEntry::project(record)?;
        if query.matches(&entry, &mut scratch) {
            page.matched_count += 1;
            page.unknown_activity_count +=
                usize::from(entry.native_metadata().updated_at_ms().is_none());
            let index = page
                .entries
                .binary_search_by(|current| projection::newest_first(current, &entry))
                .unwrap_or_else(|index| index);
            if index < query.limit() {
                if page.entries.len() == query.limit() {
                    page.entries.pop();
                }
                page.entries.insert(index, entry);
            }
        }
        Ok(())
    };
    let scan = match control {
        Some(control) => {
            store.scan_session_records_controlled(query.skips_invalid(), control, visit)
        }
        None => store
            .scan_session_records(query.skips_invalid(), visit)
            .map_err(FileSessionScanError::Store),
    }
    .map_err(|error| match error {
        FileSessionScanError::Store(error) => ReadError::Catalog(map_store_error(&error)),
        FileSessionScanError::Busy => ReadError::Busy,
        FileSessionScanError::Cancelled => ReadError::Cancelled,
    })?;
    page.scan_complete = scan.complete;
    page.scanned_records = scan.records;
    page.scanned_record_bytes = scan.bytes;
    page.skipped_invalid = scan.skipped_invalid;
    Ok(page)
}
fn exact_from_store(
    store: &FileSessionStore,
    id: &SessionId,
) -> Result<Option<NativeSessionCatalogEntry>, NativeSessionCatalogError> {
    store
        .project_session_record(id, NativeSessionCatalogEntry::project)
        .map_err(|error| map_store_error(&error))
}
fn map_store_error(error: &SessionStoreError) -> NativeSessionCatalogError {
    NativeSessionCatalogError::new(if error.kind == SessionStoreErrorKind::Corrupt {
        NativeSessionCatalogErrorKind::Corrupt
    } else {
        NativeSessionCatalogErrorKind::Unavailable
    })
}
