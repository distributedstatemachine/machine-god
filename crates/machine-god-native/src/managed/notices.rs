//! Manager-owned notice custody and one shared injected-clock deadline observer.
//! These records are projections, never principal or filesystem authority.
#[path = "notices/deadline.rs"]
mod deadline;
#[path = "notices/state.rs"]
mod state;
#[cfg(test)]
#[path = "notices/tests.rs"]
mod tests;

use crate::mcp::runtime::NativeMcpRuntimeClock;
pub(crate) use deadline::NoticeDeadline;
use machine_god_core::{CancellationToken, ManagedAgentState, ManagedNotifications};
use serde::{Deserialize, Serialize};
use state::{Inner, NoticeRecord, WorkIdentity};
use std::{
    fmt,
    num::NonZeroU64,
    sync::{Arc, Weak},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NoticeLimits {
    pub(crate) trackers: usize,
    pub(crate) records: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) notice_bytes: usize,
}
impl NoticeLimits {
    pub(crate) fn validate(self) -> Result<Self, NoticeError> {
        if self.trackers == 0
            || self.trackers > 4096
            || self.records == 0
            || self.records > 4096
            || self.notice_bytes == 0
            || self.notice_bytes > 16 * 1024
            || self.retained_bytes < self.notice_bytes
            || self.retained_bytes > 16 * 1024 * 1024
        {
            return Err(NoticeError::InvalidLimits);
        }
        Ok(self)
    }
}
impl Default for NoticeLimits {
    fn default() -> Self {
        Self {
            trackers: 64,
            records: 256,
            retained_bytes: 1024 * 1024,
            notice_bytes: 8 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NoticeError {
    InvalidLimits,
    InvalidInput,
    Capacity,
    Exhausted,
    ClockRegressed,
    DeadlineOverflow,
    Stale,
    StaleSource,
    Closed,
    Busy,
    UndeclaredMilestone,
    InvalidBatch,
    Cancelled,
    ClockViolation,
}
impl fmt::Display for NoticeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("managed notice operation rejected")
    }
}
impl std::error::Error for NoticeError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoticePrincipal {
    pub(crate) id: String,
    pub(crate) generation: NonZeroU64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkNoticeIdentity {
    pub(crate) source: NoticePrincipal,
    pub(crate) work_id: String,
    pub(crate) work_generation: NonZeroU64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NoticeRelationship {
    pub(crate) generation: NonZeroU64,
    pub(crate) parent: Option<NoticePrincipal>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoticeTarget {
    pub(crate) parent: NoticePrincipal,
    pub(crate) relationship_generation: NonZeroU64,
}
/// Bounded opaque journal reference, not a path or authority to read it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoticeHistoryRef {
    pub(crate) record_id: String,
    pub(crate) source_sequence: NonZeroU64,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NoticeTerminal {
    Completed,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum NoticeEvent {
    Started,
    Milestone {
        name: String,
    },
    Terminal {
        outcome: NoticeTerminal,
    },
    Interval {
        state: ManagedAgentState,
        first_tick: NonZeroU64,
        last_tick: NonZeroU64,
        coalesced_intervals: NonZeroU64,
        gap: bool,
    },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum NoticeKind {
    Started,
    Milestone {
        name: String,
    },
    Terminal {
        outcome: NoticeTerminal,
    },
    Interval {
        first_tick: NonZeroU64,
        last_tick: NonZeroU64,
    },
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoticeIdentity {
    pub(crate) source: WorkNoticeIdentity,
    pub(crate) source_sequence: NonZeroU64,
    pub(crate) kind: NoticeKind,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedNotice {
    pub(crate) source: WorkNoticeIdentity,
    pub(crate) source_sequence: NonZeroU64,
    pub(crate) target: NoticeTarget,
    pub(crate) event: NoticeEvent,
    pub(crate) history: Option<NoticeHistoryRef>,
}
impl ManagedNotice {
    pub(crate) fn identity(&self) -> NoticeIdentity {
        let kind = match &self.event {
            NoticeEvent::Started => NoticeKind::Started,
            NoticeEvent::Milestone { name } => NoticeKind::Milestone { name: name.clone() },
            NoticeEvent::Terminal { outcome } => NoticeKind::Terminal { outcome: *outcome },
            NoticeEvent::Interval {
                first_tick,
                last_tick,
                ..
            } => NoticeKind::Interval {
                first_tick: *first_tick,
                last_tick: *last_tick,
            },
        };
        NoticeIdentity {
            source: self.source.clone(),
            source_sequence: self.source_sequence,
            kind,
        }
    }
}

/// Exact weak allocation minted only by native manager admission.
#[derive(Clone)]
pub(crate) struct WorkNoticeRef {
    identity: Weak<WorkIdentity>,
    inner: Weak<Inner>,
}
impl fmt::Debug for WorkNoticeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkNoticeRef").finish_non_exhaustive()
    }
}
/// An explicit current state observation, not an inferred historical event.
pub(crate) struct NoticeObservation {
    pub(crate) work: WorkNoticeRef,
    pub(crate) state: ManagedAgentState,
    pub(crate) source_sequence: NonZeroU64,
    pub(crate) history: Option<NoticeHistoryRef>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NoticeEmission {
    Queued,
    Suppressed,
    AlreadyRecorded,
}
/// A notice is not a parent-context projection until its exact journal append confirms.
#[derive(Debug)]
pub(crate) enum PreparedNotice {
    Staged(StagedNotice),
    Suppressed,
    AlreadyRecorded,
}
/// Opaque observer of charged manager-owned staging custody. Dropping it neither
/// publishes nor discards the candidate; it retains no runtime or principal.
pub(crate) struct StagedNotice {
    inner: Weak<Inner>,
    record: Arc<NoticeRecord>,
}
impl StagedNotice {
    pub(crate) fn notice(&self) -> &ManagedNotice {
        &self.record.notice
    }
}
impl fmt::Debug for StagedNotice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StagedNotice(..)")
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NoticeUsage {
    pub(crate) trackers: usize,
    pub(crate) pending: usize,
    pub(crate) staged: usize,
    pub(crate) retained_records: usize,
    pub(crate) retained_bytes: usize,
}

/// A snapshot pins only its bounded immutable records, not runtimes/principals.
/// Reading it is non-consuming and does not prove current context admission.
pub(crate) struct NoticeBatch {
    inner: Weak<Inner>,
    target: NoticePrincipal,
    entries: Vec<NoticeBatchEntry>,
    bytes: usize,
    more: bool,
}
impl NoticeBatch {
    pub(crate) fn entries(&self) -> &[NoticeBatchEntry] {
        &self.entries
    }
    pub(crate) fn encoded_bytes(&self) -> usize {
        self.bytes
    }
    pub(crate) fn has_more(&self) -> bool {
        self.more
    }
}
pub(crate) struct NoticeBatchEntry {
    record: Arc<NoticeRecord>,
}
impl NoticeBatchEntry {
    pub(crate) fn notice(&self) -> &ManagedNotice {
        &self.record.notice
    }
    pub(crate) fn token(&self) -> NoticeAckToken {
        NoticeAckToken {
            record: Arc::downgrade(&self.record),
        }
    }
}
/// Tokens are opaque exact original allocations, not supplied sequence numbers.
#[derive(Clone)]
pub(crate) struct NoticeAckToken {
    record: Weak<NoticeRecord>,
}
impl fmt::Debug for NoticeBatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NoticeBatch")
            .field("entries", &self.entries.len())
            .field("bytes", &self.bytes)
            .field("more", &self.more)
            .finish_non_exhaustive()
    }
}

pub(crate) struct ManagedNotices {
    inner: Arc<Inner>,
}
impl ManagedNotices {
    pub(crate) fn new(
        limits: NoticeLimits,
        clock: Arc<dyn NativeMcpRuntimeClock>,
    ) -> Result<Self, NoticeError> {
        Ok(Self {
            inner: Arc::new(Inner::new(limits.validate()?, clock)),
        })
    }
    /// Policy capture occurs at acceptance; no timer starts until actual start.
    pub(super) fn register_work(
        &self,
        identity: &WorkNoticeIdentity,
        policy: ManagedNotifications,
        relationship: &NoticeRelationship,
        last_source_sequence: u64,
    ) -> Result<WorkNoticeRef, NoticeError> {
        let identity = self
            .inner
            .register(identity, policy, relationship, last_source_sequence)?;
        Ok(WorkNoticeRef {
            identity: Arc::downgrade(&identity),
            inner: Arc::downgrade(&self.inner),
        })
    }
    pub(crate) fn prepare_start(
        &self,
        work: &WorkNoticeRef,
        sequence: NonZeroU64,
        history: Option<&NoticeHistoryRef>,
    ) -> Result<PreparedNotice, NoticeError> {
        self.inner.start(work, sequence, history)
    }
    pub(crate) fn prepare_milestone(
        &self,
        work: &WorkNoticeRef,
        sequence: NonZeroU64,
        name: &str,
        history: Option<&NoticeHistoryRef>,
    ) -> Result<PreparedNotice, NoticeError> {
        self.inner.milestone(work, sequence, name, history)
    }
    pub(crate) fn prepare_terminal(
        &self,
        work: &WorkNoticeRef,
        sequence: NonZeroU64,
        outcome: NoticeTerminal,
        history: Option<&NoticeHistoryRef>,
    ) -> Result<PreparedNotice, NoticeError> {
        self.inner.terminal(work, sequence, outcome, history)
    }
    pub(crate) fn prepare_due(
        &self,
        observation: &NoticeObservation,
    ) -> Result<PreparedNotice, NoticeError> {
        self.inner.observe(observation)
    }
    /// Recovers the same pending original after observer loss; never regenerates an event.
    pub(crate) fn pending_notice(
        &self,
        work: &WorkNoticeRef,
    ) -> Result<Option<StagedNotice>, NoticeError> {
        self.inner.pending(work)
    }
    /// Manager-only boundary: call only for this exact `JournalRecord::Notice` after
    /// actual durable confirmation, including durability-repair confirmation.
    pub(super) fn confirm_durable(
        &self,
        notice: &StagedNotice,
    ) -> Result<NoticeEmission, NoticeError> {
        self.inner.confirm(notice)
    }
    /// Manager-only explicit `NotApplied` receipt. Ambiguity and token drop are not receipts.
    pub(super) fn discard_not_applied(&self, notice: &StagedNotice) -> Result<(), NoticeError> {
        self.inner.discard(notice)
    }
    /// Explicit journal replay of unacknowledged originals; no inferred events,
    /// relationships, timers or execution are created by loading a notice.
    pub(super) fn restore_notice(
        &self,
        work: &WorkNoticeRef,
        notice: &ManagedNotice,
    ) -> Result<NoticeEmission, NoticeError> {
        self.inner.restore(work, notice)
    }
    pub(crate) fn set_relationship(
        &self,
        work: &WorkNoticeRef,
        relationship: &NoticeRelationship,
    ) -> Result<(), NoticeError> {
        self.inner.set_relationship(work, relationship)
    }
    /// Explicitly stop new emissions; existing retained notices remain eligible.
    pub(crate) fn stop_work(&self, work: &WorkNoticeRef) -> Result<(), NoticeError> {
        self.inner.stop(work, false)
    }
    /// Call after durable close/history custody. Invalidates context snapshots.
    pub(crate) fn close_work(&self, work: &WorkNoticeRef) -> Result<(), NoticeError> {
        self.inner.stop(work, true)
    }
    pub(crate) fn retire_target(&self, target: &NoticePrincipal) -> Result<(), NoticeError> {
        self.inner.retire_target(target)
    }
    /// Reclaim stopped/terminal tracking independently of immutable queued
    /// records. Snapshot and reply owners retain their original byte/count charge.
    pub(crate) fn release_work(&self, work: &WorkNoticeRef) -> Result<(), NoticeError> {
        self.inner.release(work)
    }
    /// Exact durable source retirement also invalidates queued records whose
    /// stopped tracker has already been reclaimed. Labels here are projections;
    /// only the manager may invoke this after its original source closes.
    pub(super) fn retire_source(&self, source: &NoticePrincipal) -> Result<(), NoticeError> {
        self.inner.retire_source(source)
    }
    /// Manager-only exact original removal after confirmed source journal ACKs.
    /// This repairs replay visibility; it does not create private batch tokens.
    pub(super) fn acknowledge_recovered(
        &self,
        originals: &[ManagedNotice],
    ) -> Result<(), NoticeError> {
        self.inner.acknowledge_recovered(originals)
    }
    pub(crate) fn snapshot(
        &self,
        target: &NoticePrincipal,
        max_count: usize,
        max_bytes: usize,
    ) -> Result<NoticeBatch, NoticeError> {
        self.inner.snapshot(target, max_count, max_bytes)
    }
    /// Required immediately before root's serialized checkpoint/context admission.
    pub(crate) fn validate_batch(&self, batch: &NoticeBatch) -> Result<(), NoticeError> {
        self.inner.validate_batch(batch)
    }
    /// Memory-only receipt; root calls only after exact durable checkpoint success.
    pub(crate) fn acknowledge(
        &self,
        batch: &NoticeBatch,
        accepted: &[NoticeAckToken],
    ) -> Result<usize, NoticeError> {
        self.inner.acknowledge(batch, accepted)
    }
    /// Exactly one admitted manager-level waiter and one injected sleep at a time.
    pub(crate) fn wait_deadline(&self, cancellation: CancellationToken) -> NoticeDeadline {
        NoticeDeadline::new(
            Arc::downgrade(&self.inner),
            self.inner.clock.clone(),
            cancellation,
        )
    }
    pub(crate) fn usage(&self) -> NoticeUsage {
        self.inner.usage()
    }
}
impl fmt::Debug for ManagedNotices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedNotices").finish_non_exhaustive()
    }
}
