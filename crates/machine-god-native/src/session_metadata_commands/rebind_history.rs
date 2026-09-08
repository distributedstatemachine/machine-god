use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use machine_god_core::SessionRevision;
use serde_json::{Value, json};

use super::NativeSessionMetadataMutationError as Error;
use crate::{
    NativeSessionMetadata,
    session_metadata::{decode_workspace, encode_workspace},
};

/// Reserved, versioned workspace-change history; never filesystem authority.
pub const NATIVE_WORKSPACE_REBINDINGS_KEY: &str = "machine_god.workspace_rebindings";
/// History is never silently truncated; the next real change fails at capacity.
pub const MAX_NATIVE_WORKSPACE_REBINDINGS: usize = 64;

/// One recorded metadata transaction's previous/next association and injected time.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeWorkspaceRebinding {
    previous: Option<PathBuf>,
    next: PathBuf,
    at_ms: i64,
    previous_revision: SessionRevision,
}
impl NativeWorkspaceRebinding {
    #[must_use]
    pub fn previous_workspace(&self) -> Option<&Path> {
        self.previous.as_deref()
    }
    #[must_use]
    pub fn next_workspace(&self) -> &Path {
        &self.next
    }
    #[must_use]
    pub const fn at_ms(&self) -> i64 {
        self.at_ms
    }
    #[must_use]
    pub const fn previous_revision(&self) -> SessionRevision {
        self.previous_revision
    }
}
impl fmt::Debug for NativeWorkspaceRebinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeWorkspaceRebinding { .. }")
    }
}

/// Bounded strict history. Decoding validates adjacent association/time/revision
/// continuity; mutation also checks its final entry against the canonical state.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct NativeWorkspaceRebindingHistory {
    entries: Vec<NativeWorkspaceRebinding>,
}
impl fmt::Debug for NativeWorkspaceRebindingHistory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeWorkspaceRebindingHistory")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}
impl NativeWorkspaceRebindingHistory {
    /// Absent history is unknown/empty, not a fabricated creation event.
    /// # Errors
    /// Rejects extra/missing fields, unsupported schemas, invalid paths/types,
    /// over-64-entry arrays and inconsistent previous/next/time/revision chains.
    /// Only this shallow reserved entry is traversed; unrelated values are ignored.
    pub fn from_metadata(metadata: &BTreeMap<String, Value>) -> Result<Self, Error> {
        let Some(value) = metadata.get(NATIVE_WORKSPACE_REBINDINGS_KEY) else {
            return Ok(Self::default());
        };
        let object = value.as_object().ok_or(Error::InvalidHistory)?;
        if object.len() != 2 || object.get("schema_version").and_then(Value::as_u64) != Some(1) {
            return Err(Error::InvalidHistory);
        }
        let entries = object
            .get("entries")
            .and_then(Value::as_array)
            .ok_or(Error::InvalidHistory)?;
        if entries.len() > MAX_NATIVE_WORKSPACE_REBINDINGS {
            return Err(Error::HistoryLimit);
        }
        let mut history = Self {
            entries: Vec::with_capacity(entries.len()),
        };
        for entry in entries {
            let object = entry.as_object().ok_or(Error::InvalidHistory)?;
            if object.len() != 4 {
                return Err(Error::InvalidHistory);
            }
            let previous = match object.get("previous_workspace_hex") {
                Some(Value::Null) => None,
                Some(Value::String(path)) => {
                    Some(decode_workspace(path).map_err(|_| Error::InvalidHistory)?)
                }
                _ => return Err(Error::InvalidHistory),
            };
            let next = object
                .get("next_workspace_hex")
                .and_then(Value::as_str)
                .ok_or(Error::InvalidHistory)?;
            let next = decode_workspace(next).map_err(|_| Error::InvalidHistory)?;
            let time = object
                .get("at_ms")
                .and_then(Value::as_i64)
                .ok_or(Error::InvalidHistory)?;
            let revision = object
                .get("previous_revision")
                .and_then(Value::as_u64)
                .ok_or(Error::InvalidHistory)?;
            history.append(previous, next, time, SessionRevision(revision))?;
        }
        Ok(history)
    }

    #[must_use]
    pub fn entries(&self) -> &[NativeWorkspaceRebinding] {
        &self.entries
    }

    /// Encodes checked values only; encoding itself is not a persistence receipt.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({"schema_version": 1, "entries": self.entries.iter().map(|entry| json!({
            "previous_workspace_hex": entry.previous.as_deref().map(encode_workspace),
            "next_workspace_hex": encode_workspace(&entry.next),
            "at_ms": entry.at_ms,
            "previous_revision": entry.previous_revision.0,
        })).collect::<Vec<_>>()})
    }

    pub(crate) fn validate_current(
        &self,
        metadata: &NativeSessionMetadata,
        revision: SessionRevision,
    ) -> Result<(), Error> {
        if self.entries.first().is_some_and(|first| {
            metadata
                .created_at_ms()
                .is_some_and(|created| first.at_ms < created)
        }) {
            return Err(Error::InvalidHistory);
        }
        if self.entries.last().is_some_and(|last| {
            metadata.workspace() != Some(last.next_workspace())
                || metadata
                    .updated_at_ms()
                    .is_none_or(|time| time < last.at_ms)
                || revision <= last.previous_revision
        }) {
            return Err(Error::InvalidHistory);
        }
        Ok(())
    }

    pub(super) fn append(
        &mut self,
        previous: Option<PathBuf>,
        next: PathBuf,
        at_ms: i64,
        previous_revision: SessionRevision,
    ) -> Result<(), Error> {
        if self.entries.len() == MAX_NATIVE_WORKSPACE_REBINDINGS {
            return Err(Error::HistoryLimit);
        }
        if previous.as_ref() == Some(&next)
            || previous_revision.0 == u64::MAX
            || self.entries.last().is_some_and(|last| {
                previous.as_deref() != Some(last.next_workspace())
                    || at_ms < last.at_ms
                    || previous_revision <= last.previous_revision
            })
        {
            return Err(Error::InvalidHistory);
        }
        self.entries.push(NativeWorkspaceRebinding {
            previous,
            next,
            at_ms,
            previous_revision,
        });
        Ok(())
    }
}
