//! Bounded native navigation observations, serialized with actual journal work.
use super::{Active, ManagedManager};
use crate::managed::store::{JournalCatalogCursor, JournalCatalogPage, JournalError};
use machine_god_core::{ManagedAgentState, ManagedSubagentCommand};
use std::{
    fmt,
    sync::{Arc, Weak},
    task::{Context, Waker},
};

/// Filtering never increases the raw scan bound or loads a child runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeManagedCatalogFilter {
    Current,
    Archived,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeManagedCatalogError {
    Busy,
    Closed,
    InvalidCursor,
    InvalidLimit,
    Unavailable,
}
impl fmt::Display for NativeManagedCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "managed catalog observation pending",
            Self::Closed => "managed catalog closed",
            Self::InvalidCursor => "managed catalog cursor unavailable",
            Self::InvalidLimit => "managed catalog limit invalid",
            Self::Unavailable => "managed catalog unavailable",
        })
    }
}
impl std::error::Error for NativeManagedCatalogError {}

#[derive(Default)]
struct Identity;

/// Correlation only. The weak identity retains no host, manager, or journal.
#[derive(Clone)]
pub struct NativeManagedCatalogRequest {
    identity: Weak<Identity>,
    sequence: u64,
}
impl PartialEq for NativeManagedCatalogRequest {
    fn eq(&self, other: &Self) -> bool {
        self.identity.ptr_eq(&other.identity) && self.sequence == other.sequence
    }
}
impl Eq for NativeManagedCatalogRequest {}
impl fmt::Debug for NativeManagedCatalogRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedCatalogRequest { .. }")
    }
}

/// Opaque traversal position bound to the original manager and filter. Pages
/// observe live heads, not a frozen global snapshot; each row has its own fence.
#[derive(Clone)]
pub struct NativeManagedCatalogCursor {
    identity: Weak<Identity>,
    filter: NativeManagedCatalogFilter,
    inner: JournalCatalogCursor,
}
impl fmt::Debug for NativeManagedCatalogCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedCatalogCursor { .. }")
    }
}

/// An observed exact head, not a human actor or a process-control capability.
#[derive(Clone)]
pub struct NativeObservedManagedAgent {
    identity: Weak<Identity>,
    pub(crate) id: String,
    pub(crate) generation: u64,
    pub(crate) revision: u64,
}
impl fmt::Debug for NativeObservedManagedAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeObservedManagedAgent { .. }")
    }
}
impl NativeObservedManagedAgent {
    pub(crate) fn matches_command(&self, command: &ManagedSubagentCommand) -> bool {
        let target = match command {
            ManagedSubagentCommand::Inspect(value) => &value.id,
            ManagedSubagentCommand::Message(machine_god_core::ManagedMessage::Send(value)) => {
                &value.id
            }
            ManagedSubagentCommand::Relationship(value) => &value.id,
            ManagedSubagentCommand::Configure(value) => &value.id,
            ManagedSubagentCommand::Lifecycle(value) => &value.id,
            _ => return false,
        };
        target == &self.id
    }
    pub(crate) fn matches_snapshot(
        &self,
        snapshot: &crate::managed::store::JournalSnapshot,
    ) -> bool {
        self.id == snapshot.head.id
            && self.generation == snapshot.head.generation
            && self.revision == snapshot.head.revision
    }
}

#[derive(Debug)]
pub struct NativeManagedCatalogEntry {
    pub id: String,
    pub name: String,
    pub generation: u64,
    pub revision: u64,
    pub parent_id: Option<String>,
    pub state: ManagedAgentState,
    /// A stale saved running/approval state does not assert current execution.
    pub recovery_required: bool,
    pub observation: NativeObservedManagedAgent,
}
#[derive(Debug)]
pub struct NativeManagedCatalogPage {
    pub entries: Vec<NativeManagedCatalogEntry>,
    pub scanned: usize,
    pub next: Option<NativeManagedCatalogCursor>,
}
#[derive(Debug)]
pub struct NativeManagedCatalogOutcome {
    pub request: NativeManagedCatalogRequest,
    pub result: Result<NativeManagedCatalogPage, NativeManagedCatalogError>,
}

pub(super) struct Request {
    token: NativeManagedCatalogRequest,
    filter: NativeManagedCatalogFilter,
    cursor: Option<JournalCatalogCursor>,
    limit: usize,
}
#[derive(Default)]
pub(super) struct Catalog {
    identity: Arc<Identity>,
    next: u64,
    pending: Option<Request>,
    in_flight: bool,
    outcome: Option<NativeManagedCatalogOutcome>,
    last_read: bool,
    closed: bool,
    wake: Option<Waker>,
}
impl Catalog {
    pub(super) fn register(&mut self, cx: &Context<'_>) {
        self.wake = Some(cx.waker().clone());
    }
    fn notify(&mut self) {
        if let Some(wake) = self.wake.take() {
            wake.wake();
        }
    }
    pub(super) fn yield_to_work(&mut self) -> bool {
        std::mem::take(&mut self.last_read)
    }
    pub(super) fn close(&mut self) {
        self.closed = true;
        if let Some(request) = self.pending.take() {
            self.finish(request, Err(JournalError::Missing));
        }
        self.notify();
    }
    pub(super) fn finish(
        &mut self,
        request: Request,
        result: Result<JournalCatalogPage, JournalError>,
    ) {
        self.in_flight = false;
        let result = if self.closed {
            Err(NativeManagedCatalogError::Closed)
        } else {
            result
                .map_err(|error| match error {
                    JournalError::Busy => NativeManagedCatalogError::Busy,
                    JournalError::Conflict => NativeManagedCatalogError::InvalidCursor,
                    _ => NativeManagedCatalogError::Unavailable,
                })
                .map(|page| {
                    let scanned = page.entries.len();
                    let entries = page
                        .entries
                        .into_iter()
                        .filter(|entry| match request.filter {
                            NativeManagedCatalogFilter::Current => {
                                entry.status != ManagedAgentState::Archived
                            }
                            NativeManagedCatalogFilter::Archived => {
                                entry.status == ManagedAgentState::Archived
                            }
                            NativeManagedCatalogFilter::All => true,
                        })
                        .map(|entry| NativeManagedCatalogEntry {
                            observation: NativeObservedManagedAgent {
                                identity: Arc::downgrade(&self.identity),
                                id: entry.id.clone(),
                                generation: entry.generation,
                                revision: entry.revision,
                            },
                            id: entry.id,
                            name: entry.name,
                            generation: entry.generation,
                            revision: entry.revision,
                            parent_id: entry.parent_id,
                            state: entry.status,
                            recovery_required: entry.recovery_required,
                        })
                        .collect();
                    NativeManagedCatalogPage {
                        entries,
                        scanned,
                        next: page.next.map(|inner| NativeManagedCatalogCursor {
                            identity: Arc::downgrade(&self.identity),
                            filter: request.filter,
                            inner,
                        }),
                    }
                })
        };
        self.outcome = Some(NativeManagedCatalogOutcome {
            request: request.token,
            result,
        });
        self.notify();
    }
}

impl ManagedManager {
    pub(crate) fn request_catalog(
        &mut self,
        filter: NativeManagedCatalogFilter,
        cursor: Option<NativeManagedCatalogCursor>,
        limit: usize,
    ) -> Result<NativeManagedCatalogRequest, NativeManagedCatalogError> {
        if self.closing {
            return Err(NativeManagedCatalogError::Closed);
        }
        if !(1..=64).contains(&limit) {
            return Err(NativeManagedCatalogError::InvalidLimit);
        }
        if self.catalog.pending.is_some()
            || self.catalog.in_flight
            || self.catalog.outcome.is_some()
        {
            return Err(NativeManagedCatalogError::Busy);
        }
        if cursor.as_ref().is_some_and(|cursor| {
            cursor.filter != filter
                || !cursor
                    .identity
                    .ptr_eq(&Arc::downgrade(&self.catalog.identity))
        }) {
            return Err(NativeManagedCatalogError::InvalidCursor);
        }
        self.catalog.next = self
            .catalog
            .next
            .checked_add(1)
            .ok_or(NativeManagedCatalogError::Unavailable)?;
        let token = NativeManagedCatalogRequest {
            identity: Arc::downgrade(&self.catalog.identity),
            sequence: self.catalog.next,
        };
        self.catalog.pending = Some(Request {
            token: token.clone(),
            filter,
            cursor: cursor.map(|cursor| cursor.inner),
            limit,
        });
        self.catalog.notify();
        Ok(token)
    }
    pub(crate) fn take_catalog_outcome(&mut self) -> Option<NativeManagedCatalogOutcome> {
        self.catalog.outcome.take()
    }
    pub(crate) fn owns_observation(&self, observed: &NativeObservedManagedAgent) -> bool {
        !self.closing
            && observed
                .identity
                .ptr_eq(&Arc::downgrade(&self.catalog.identity))
    }
    pub(super) fn begin_catalog(&mut self) -> bool {
        if self.closing {
            return false;
        }
        let Some(request) = self.catalog.pending.take() else {
            return false;
        };
        let future = self.journal.catalog(request.cursor.clone(), request.limit);
        self.catalog.in_flight = true;
        self.catalog.last_read = true;
        self.active = Some(Active::Catalog { request, future });
        true
    }
}
