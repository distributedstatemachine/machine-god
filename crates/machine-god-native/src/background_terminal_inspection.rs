//! Read-only recorded terminal history, never recovered process authority.
//!
//! This synchronous entrypoint belongs on a caller-owned native worker. It
//! borrows explicit state-root authority; it never prepares a runtime or store.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fmt;

use machine_god_core::{
    BackgroundOutputOwner, CancellationToken, TerminalCursor, TerminalLifecycle, TerminalSessionId,
};
use rustix::fd::AsFd;

use crate::terminal_catalog::{TerminalCatalogError, canonical_workspace};
use crate::terminal_journal::TerminalJournalError;
use crate::terminal_monitor::TerminalProcessOutcome;

mod topology;

pub(crate) const MAX_TERMINAL_BACKGROUND_RECORDS: usize = 128;
pub(crate) const MAX_TERMINAL_BACKGROUND_TEXT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_TERMINAL_BACKGROUND_INPUT_BYTES: usize = 64 * 1024 * 1024;

/// Committed observations only. `lifecycle` does not assert current liveness.
pub(crate) struct NativeTerminalBackgroundHistoryRecord {
    pub(crate) session_id: TerminalSessionId,
    pub(crate) owner: BackgroundOutputOwner,
    pub(crate) created_at_ms: i64,
    pub(crate) last_output_ms: i64,
    pub(crate) workspace: String,
    pub(crate) command: Option<String>,
    pub(crate) cwd: String,
    pub(crate) lifecycle: TerminalLifecycle,
    pub(crate) outcome: Option<TerminalProcessOutcome>,
    pub(crate) earliest: TerminalCursor,
    pub(crate) latest: TerminalCursor,
    /// Facts may precede the latest committed raw-output cursor.
    pub(crate) facts_cursor: TerminalCursor,
}

impl fmt::Debug for NativeTerminalBackgroundHistoryRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeTerminalBackgroundHistoryRecord")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeTerminalBackgroundHistoryError {
    Invalid,
    NotFound,
    Busy,
    Corrupt,
    ResourceLimit,
    Cancelled,
    Unavailable,
}

impl fmt::Display for NativeTerminalBackgroundHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid terminal history request",
            Self::NotFound => "terminal history not found",
            Self::Busy => "terminal history observation busy",
            Self::Corrupt => "terminal history corrupt",
            Self::ResourceLimit => "terminal history resource limit",
            Self::Cancelled => "terminal history observation cancelled",
            Self::Unavailable => "terminal history observation unavailable",
        })
    }
}
impl std::error::Error for NativeTerminalBackgroundHistoryError {}

type Error = NativeTerminalBackgroundHistoryError;
pub(crate) type Result<T> = std::result::Result<T, Error>;

impl From<TerminalCatalogError> for Error {
    fn from(error: TerminalCatalogError) -> Self {
        match error {
            TerminalCatalogError::Invalid => Self::Invalid,
            TerminalCatalogError::NotFound => Self::NotFound,
            TerminalCatalogError::Busy => Self::Busy,
            TerminalCatalogError::Conflict | TerminalCatalogError::Corrupt => Self::Corrupt,
            TerminalCatalogError::ResourceLimit => Self::ResourceLimit,
            TerminalCatalogError::Unavailable => Self::Unavailable,
        }
    }
}

impl From<TerminalJournalError> for Error {
    fn from(error: TerminalJournalError) -> Self {
        match error {
            TerminalJournalError::Invalid => Self::Invalid,
            TerminalJournalError::NotFound => Self::NotFound,
            TerminalJournalError::Busy => Self::Busy,
            TerminalJournalError::Conflict | TerminalJournalError::Corrupt => Self::Corrupt,
            TerminalJournalError::ResourceLimit => Self::ResourceLimit,
            TerminalJournalError::Unavailable => Self::Unavailable,
        }
    }
}

/// Complete bounded observation or error, never a silently truncated listing.
/// Matching facts are ordered by last output, creation time, then exact ID,
/// descending. Exact lookup still validates every namespace before selecting.
pub(crate) fn inspect_terminal_background_history(
    state_root: impl AsFd,
    workspace: &str,
    exact: Option<&TerminalSessionId>,
    cancellation: &CancellationToken,
) -> Result<Vec<NativeTerminalBackgroundHistoryRecord>> {
    let mut budget = HistoryReadBudget::new(cancellation);
    budget.checkpoint()?;
    if !canonical_workspace(workspace) {
        return Err(Error::Invalid);
    }
    let Some(profile) = topology::ReadProfile::open(state_root.as_fd())? else {
        return if exact.is_some() {
            Err(Error::NotFound)
        } else {
            Ok(Vec::new())
        };
    };
    let before = profile.topology(&budget)?;
    let mut records: Vec<NativeTerminalBackgroundHistoryRecord> = Vec::new();
    let mut retained_text = 0_usize;
    for owner in &before {
        for session in &owner.sessions {
            budget.checkpoint()?;
            let directory = profile.open_session(owner, session)?;
            let verified = crate::terminal_journal::inspection::read_history(
                &directory,
                &session.id,
                &mut budget,
            )?;
            let facts = verified.facts;
            facts
                .validate_profile_binding(&owner.name)
                .map_err(|_| Error::Corrupt)?;
            let identity = facts.owner_identity();
            let metadata = facts.metadata.ok_or(Error::Corrupt)?;
            if metadata.workspace != workspace || exact.is_some_and(|id| id != &facts.session_id) {
                continue;
            }
            // An ID cannot ambiguously select two owners in the same workspace.
            if records
                .iter()
                .any(|record| record.session_id == facts.session_id)
            {
                return Err(Error::Corrupt);
            }
            if records.len() == MAX_TERMINAL_BACKGROUND_RECORDS {
                return Err(Error::ResourceLimit);
            }
            for text in [
                facts.session_id.as_str(),
                identity.session_id().as_str(),
                identity.session_incarnation_id().as_str(),
                &metadata.workspace,
                &metadata.cwd,
                metadata.command.as_deref().unwrap_or_default(),
            ] {
                retained_text = retained_text
                    .checked_add(text.len())
                    .filter(|bytes| *bytes <= MAX_TERMINAL_BACKGROUND_TEXT_BYTES)
                    .ok_or(Error::ResourceLimit)?;
            }
            records.push(NativeTerminalBackgroundHistoryRecord {
                session_id: facts.session_id,
                owner: identity,
                created_at_ms: facts.created_at_ms,
                last_output_ms: facts.last_output_ms,
                workspace: metadata.workspace,
                command: metadata.command,
                cwd: metadata.cwd,
                lifecycle: facts.context.lifecycle,
                outcome: facts.outcome,
                earliest: verified.earliest,
                latest: verified.latest,
                facts_cursor: facts.context.cursor,
            });
        }
    }
    #[cfg(test)]
    BEFORE_FINAL_VALIDATION.with(|callback| {
        if let Some(callback) = callback.borrow_mut().take() {
            callback();
        }
    });
    if profile.topology(&budget)? != before {
        return Err(Error::Corrupt);
    }
    profile.validate()?;
    budget.checkpoint()?;
    if exact.is_some() && records.is_empty() {
        return Err(Error::NotFound);
    }
    records.sort_unstable_by(|left, right| {
        right
            .last_output_ms
            .cmp(&left.last_output_ms)
            .then_with(|| right.created_at_ms.cmp(&left.created_at_ms))
            .then_with(|| right.session_id.as_str().cmp(left.session_id.as_str()))
    });
    Ok(records)
}

/// Full state bodies are hashed in fixed chunks; only their bounded facts
/// prefix is retained. This budget also includes every metadata byte read.
pub(crate) struct HistoryReadBudget<'a> {
    cancellation: &'a CancellationToken,
    remaining: usize,
}

impl<'a> HistoryReadBudget<'a> {
    fn new(cancellation: &'a CancellationToken) -> Self {
        Self {
            cancellation,
            remaining: MAX_TERMINAL_BACKGROUND_INPUT_BYTES,
        }
    }

    pub(crate) fn checkpoint(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn charge(&mut self, bytes: usize) -> Result<()> {
        self.checkpoint()?;
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or(Error::ResourceLimit)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    static BEFORE_FINAL_VALIDATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
