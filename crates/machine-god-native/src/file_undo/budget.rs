//! Aggregate accounting shared by independent principal histories.

use super::{FileUndoError, MAX_FILE_UNDO_ENTRIES, MAX_FILE_UNDO_PREIMAGE_BYTES};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::Arc;
use std::sync::Mutex;

// One forward operation retains at most one byte preimage. Reserve framing,
// path labels, transient reads and descriptor overlap before the first open.
pub(super) const OPERATION_BYTES: usize = MAX_FILE_UNDO_PREIMAGE_BYTES + 128 * 1024;
pub(super) const OPERATION_DESCRIPTORS: usize = 32;

/// Combined retained and in-flight limits, not a separate allowance per child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_field_names,
    reason = "explicit maxima distinguish configured limits from observed usage"
)]
pub struct NativeUndoLimits {
    pub max_entries: usize,
    pub max_bytes: usize,
    pub max_descriptors: usize,
}

impl Default for NativeUndoLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_FILE_UNDO_ENTRIES + 1,
            max_bytes: (MAX_FILE_UNDO_ENTRIES + 1) * OPERATION_BYTES,
            max_descriptors: (MAX_FILE_UNDO_ENTRIES + 1) * OPERATION_DESCRIPTORS,
        }
    }
}

/// Point-in-time accounting. A reservation is not execution authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeUndoUsage {
    pub entries: usize,
    pub bytes: usize,
    pub descriptors: usize,
}

/// Inert, finite aggregate budget. No lock is retained across native effects.
#[derive(Debug)]
pub struct NativeUndoBudget {
    limits: NativeUndoLimits,
    used: Mutex<NativeUndoUsage>,
}

impl Default for NativeUndoBudget {
    fn default() -> Self {
        Self::new(NativeUndoLimits::default()).expect("valid default undo budget")
    }
}

impl NativeUndoBudget {
    /// Validates limits without reserving memory or opening descriptors.
    /// # Errors
    /// Rejects zero, insufficient single-operation or excessive limits.
    pub fn new(limits: NativeUndoLimits) -> Result<Self, FileUndoError> {
        if !(1..=65_536).contains(&limits.max_entries)
            || !(OPERATION_BYTES..=usize::MAX / 2).contains(&limits.max_bytes)
            || !(OPERATION_DESCRIPTORS..=1_048_576).contains(&limits.max_descriptors)
        {
            return Err(FileUndoError::ResourceLimit);
        }
        Ok(Self {
            limits,
            used: Mutex::new(NativeUndoUsage::default()),
        })
    }

    #[must_use]
    pub const fn limits(&self) -> NativeUndoLimits {
        self.limits
    }

    /// Observes reserved resources, including operations not yet committed.
    #[must_use]
    pub fn usage(&self) -> NativeUndoUsage {
        *self
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn reserve(self: &Arc<Self>, entry: bool) -> Result<Reservation, FileUndoError> {
        let charge = NativeUndoUsage {
            entries: usize::from(entry),
            bytes: if entry { OPERATION_BYTES } else { 64 * 1024 },
            descriptors: if entry { OPERATION_DESCRIPTORS } else { 16 },
        };
        let mut used = self.used.lock().map_err(|_| FileUndoError::ResourceLimit)?;
        let next = NativeUndoUsage {
            entries: used
                .entries
                .checked_add(charge.entries)
                .ok_or(FileUndoError::ResourceLimit)?,
            bytes: used
                .bytes
                .checked_add(charge.bytes)
                .ok_or(FileUndoError::ResourceLimit)?,
            descriptors: used
                .descriptors
                .checked_add(charge.descriptors)
                .ok_or(FileUndoError::ResourceLimit)?,
        };
        if next.entries > self.limits.max_entries
            || next.bytes > self.limits.max_bytes
            || next.descriptors > self.limits.max_descriptors
        {
            return Err(FileUndoError::ResourceLimit);
        }
        *used = next;
        Ok(Reservation {
            budget: self.clone(),
            charge,
        })
    }
}

/// Nonclone ticket. Declare after native resources so they drop before refund.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) struct Reservation {
    budget: Arc<NativeUndoBudget>,
    charge: NativeUndoUsage,
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Reservation {
    pub(super) fn split(&mut self, charge: NativeUndoUsage) -> Self {
        // Internal fixed-operation bound, never caller-supplied accounting.
        assert!(
            charge.entries <= self.charge.entries
                && charge.bytes <= self.charge.bytes
                && charge.descriptors <= self.charge.descriptors
        );
        self.charge.entries -= charge.entries;
        self.charge.bytes -= charge.bytes;
        self.charge.descriptors -= charge.descriptors;
        Self {
            budget: self.budget.clone(),
            charge,
        }
    }
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut used = self
            .budget
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        used.entries -= self.charge.entries;
        used.bytes -= self.charge.bytes;
        used.descriptors -= self.charge.descriptors;
    }
}
