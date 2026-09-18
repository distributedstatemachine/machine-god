//! Persisted settlement credits are distinct from transient operation reservations.
use super::super::{JournalError as Error, JournalLimits, Shared};
use super::{JournalHead, JournalMutation, JournalRecord};
use machine_god_core::ManagedAgentState;

// Two pending writes, two notice flags, summary + all-FIFO interruption,
// close intent + archive, and two recovery/final-maintenance publications.
const CLEANUP_PUBLICATIONS: usize = 10;
const IDLE_PUBLICATIONS: usize = 4;
// Each pending write can already own a staged notice, in addition to the flags.
const FUTURE_NOTICES: usize = 4;
const CLEANUP_PAGE_BYTES: usize = 256 * 1024;
const ACK_PAGE_BYTES: usize = 64 * 1024;
const FILE_OVERHEAD: usize = super::super::filesystem::FILE_OVERHEAD;

#[derive(Clone, Copy)]
pub(super) enum Admission {
    New,
    Ordinary,
    Settlement,
}
impl Admission {
    pub(super) fn mutation(mutation: &JournalMutation) -> Self {
        match mutation {
            JournalMutation::Enqueue(_)
            | JournalMutation::Reopen(_)
            | JournalMutation::Intent(_)
            | JournalMutation::ResolveHead { retry: true, .. } => Self::New,
            JournalMutation::HeadState { .. }
            | JournalMutation::CancelHead { .. }
            | JournalMutation::CancelIdle
            | JournalMutation::Archive
            | JournalMutation::Recover
            | JournalMutation::InterruptForPressure
            | JournalMutation::Milestone { .. }
            | JournalMutation::AppendHistory(_)
            | JournalMutation::SuppressedNotice(_) => Self::Settlement,
            _ => Self::Ordinary,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(in crate::managed::store) struct Protection {
    pub bytes: usize,
    pub entries: usize,
}
fn cleanup_unit(limits: JournalLimits) -> usize {
    limits.head_bytes + limits.page_bytes.min(CLEANUP_PAGE_BYTES) + 2 * FILE_OVERHEAD
}
fn acknowledgement_unit(limits: JournalLimits) -> usize {
    limits.head_bytes + ACK_PAGE_BYTES + 2 * FILE_OVERHEAD
}
fn cleanup_bytes(limits: JournalLimits, queue_entries: usize) -> usize {
    if queue_entries == 0 {
        return IDLE_PUBLICATIONS * cleanup_unit(limits);
    }
    CLEANUP_PUBLICATIONS * cleanup_unit(limits)
        + FUTURE_NOTICES * acknowledgement_unit(limits)
        + queue_entries.saturating_sub(1) * cleanup_unit(limits)
}
fn cleanup_entries(queue_entries: usize) -> usize {
    if queue_entries == 0 {
        return IDLE_PUBLICATIONS;
    }
    CLEANUP_PUBLICATIONS + FUTURE_NOTICES + queue_entries.saturating_sub(1)
}
pub(in crate::managed::store) fn operation_bytes(limits: JournalLimits) -> usize {
    limits.head_bytes * 4 + limits.page_bytes * 8
}
impl Protection {
    pub(in crate::managed::store) fn from_head(
        head: &JournalHead,
        limits: JournalLimits,
    ) -> Result<Self, Error> {
        if head.cleanup_bytes > cleanup_bytes(limits, limits.queue_entries)
            || head.cleanup_entries > cleanup_entries(limits.queue_entries)
            || head.notice_reservations.len() > 4096
            || head
                .notice_reservations
                .iter()
                .any(|sequence| *sequence == 0 || *sequence >= head.next_sequence)
            || head
                .notice_reservations
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            bytes: head.cleanup_bytes
                + head.notice_reservations.len() * acknowledgement_unit(limits),
            entries: head.cleanup_entries + head.notice_reservations.len(),
        })
    }
}

pub(super) fn prepare(
    head: &mut JournalHead,
    records: &[JournalRecord],
    limits: JournalLimits,
    admission: Admission,
) -> Result<(), Error> {
    if matches!(admission, Admission::New) {
        head.cleanup_bytes = cleanup_bytes(limits, head.queue.len());
        head.cleanup_entries = cleanup_entries(head.queue.len());
    } else if head.queue.is_empty() {
        // The final state can precede two already-owned notice flags. Retain
        // those plus close/recovery; release the active-turn/FIFO excess.
        let remaining =
            (IDLE_PUBLICATIONS + 2) * cleanup_unit(limits) + 2 * acknowledgement_unit(limits);
        head.cleanup_bytes = head.cleanup_bytes.min(remaining);
        head.cleanup_entries = head.cleanup_entries.min(IDLE_PUBLICATIONS + 4);
    }
    for record in records {
        match record {
            JournalRecord::Notice(notice) => {
                let sequence = notice.source_sequence.get();
                if head.notice_reservations.len() == 4096
                    || head
                        .notice_reservations
                        .last()
                        .is_some_and(|old| *old >= sequence)
                {
                    return Err(Error::Limit);
                }
                head.notice_reservations.push(sequence);
            }
            JournalRecord::NoticeAcknowledged { identity, .. }
                if identity.source.source.id == head.id =>
            {
                // Manager validation proves the exact original/checkpoint before
                // this mutation. Duplicate acknowledgements cannot refund twice.
                if let Ok(index) = head
                    .notice_reservations
                    .binary_search(&identity.source_sequence.get())
                {
                    head.notice_reservations.remove(index);
                }
            }
            _ => {}
        }
    }
    if head.status == ManagedAgentState::Archived {
        head.cleanup_bytes = 0;
        head.cleanup_entries = 0;
    }
    Ok(())
}

/// Reserve worst-case old/new/head staging plus the next read operation before
/// the first publication effect. Only the exact head's protected balance can be
/// spent; source ACK balances cannot be borrowed by ordinary settlement.
pub(super) fn admit(
    shared: &Shared,
    head: &mut JournalHead,
    original: Protection,
    page_bytes: usize,
    admission: Admission,
    new_head: bool,
) -> Result<(), Error> {
    if !matches!(admission, Admission::Settlement) && !ordinary_available(shared) {
        return Err(Error::Limit);
    }
    if !matches!(admission, Admission::Settlement)
        && super::encode(head, shared.limits.head_bytes)?
            .len()
            .saturating_add(4096)
            > shared.limits.head_bytes
    {
        // Future notice-credit identities and revision/sequence growth must
        // fit even after ordinary production is stopped by head pressure.
        return Err(Error::Limit);
    }
    let growth = page_bytes + shared.limits.head_bytes + 2 * FILE_OVERHEAD;
    let state = shared.state.lock().map_err(|_| Error::Invalid)?;
    let next = Protection::from_head(head, shared.limits)?;
    let projected = state
        .used
        .checked_add(operation_bytes(shared.limits))
        .and_then(|n| n.checked_add(growth))
        .and_then(|n| n.checked_add(state.protected_bytes.checked_sub(original.bytes)?))
        .and_then(|n| n.checked_add(next.bytes))
        .ok_or(Error::Limit)?;
    // A new page and a possible new/staging head plus an owner-epoch spare.
    let entries = state
        .entries
        .checked_add(3 + usize::from(new_head))
        .and_then(|n| n.checked_add(state.protected_entries.checked_sub(original.entries)?))
        .and_then(|n| n.checked_add(next.entries))
        .ok_or(Error::Limit)?;
    let bytes_needed = projected.saturating_sub(shared.limits.aggregate_bytes);
    let entries_needed = entries.saturating_sub(shared.limits.directory_entries);
    if bytes_needed == 0 && entries_needed == 0 {
        return Ok(());
    }
    if !matches!(admission, Admission::Settlement)
        || page_bytes > CLEANUP_PAGE_BYTES
        || bytes_needed > head.cleanup_bytes
        || entries_needed > head.cleanup_entries
    {
        return Err(Error::Limit);
    }
    head.cleanup_bytes -= bytes_needed;
    head.cleanup_entries -= entries_needed;
    Ok(())
}

pub(in crate::managed::store) fn ordinary_available(shared: &Shared) -> bool {
    let Ok(state) = shared.state.lock() else {
        return false;
    };
    let worst_growth = shared.limits.page_bytes
        + shared.limits.head_bytes
        + acknowledgement_unit(shared.limits)
        + 2 * FILE_OVERHEAD;
    state.headroom_low_heads == 0
        && state
            .used
            .checked_add(state.protected_bytes)
            .and_then(|n| n.checked_add(operation_bytes(shared.limits)))
            .and_then(|n| n.checked_add(worst_growth))
            .is_some_and(|n| n <= shared.limits.aggregate_bytes)
        && state
            .entries
            .checked_add(state.protected_entries)
            .and_then(|n| n.checked_add(4))
            .is_some_and(|n| n <= shared.limits.directory_entries)
}
