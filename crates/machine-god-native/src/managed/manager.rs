//! Outer native managed-agent ownership, independent of the foreground UI.

mod command;
mod delivery;
mod durability;
pub(crate) mod factory;
mod foreground;
mod notification;
mod projection;
mod pump;
mod replay;
mod reservation;
#[cfg(test)]
mod tests;
mod waiting;

use super::{
    mailbox::{ManagedMailbox, ManagedMailboxJob},
    notices::{ManagedNotices, NoticeDeadline, WorkNoticeRef},
    scheduler::{RunRef, RunSettlement},
    store::{
        JournalMutation, JournalOwner, JournalRecord, JournalSnapshot, JournalWork, ManagedJournal,
    },
};
use crate::{
    NativeConversationRuntime, NativeConversationRuntimeError, NativeConversationRuntimeTurn,
    mcp::runtime::NativeMcpRuntimeClock,
};
use factory::{
    ManagedRelationshipAuthorizer, ManagedRuntimeError, ManagedRuntimeFactory,
    PreparedManagedRuntime,
};
pub(crate) use foreground::ManagedForegroundSelection;
use machine_god_core::{
    BoxFuture, CancellationToken, ManagedAgentState, ManagedQueueStatus, ToolCallId,
};
pub(crate) use projection::{ManagedChildProjection, ManagedSelection};
pub(crate) use reservation::ManagedForegroundReservation;
use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Weak},
    task::{Context, Poll},
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ManagerLimits {
    pub residents: usize,
    pub waiters: usize,
    pub buffered_bytes: usize,
    pub work_per_poll: usize,
}
impl Default for ManagerLimits {
    fn default() -> Self {
        Self {
            residents: 64,
            waiters: 64,
            buffered_bytes: 64 * 1024 * 1024,
            work_per_poll: 64,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagerBlock {
    Capacity,
    Journal,
    Preparation,
    Cleanup,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ManagerProgress {
    pub residents: usize,
    pub executing: usize,
    pub waiters: usize,
    pub closing: bool,
    pub blocked: Option<ManagerBlock>,
}

#[allow(clippy::struct_excessive_bools)] // Independent admission, cleanup, intent and projection custody.
struct Child {
    snapshot: JournalSnapshot,
    prepared: PreparedManagedRuntime,
    selection: Arc<projection::SelectionIdentity>,
    starting: Option<
        BoxFuture<
            'static,
            Result<Option<NativeConversationRuntimeTurn>, NativeConversationRuntimeError>,
        >,
    >,
    turn: Option<NativeConversationRuntimeTurn>,
    work: Option<JournalWork>,
    settlement: Option<(RunRef, RunSettlement)>,
    actual_settled: bool,
    admission_pending: bool,
    pending: VecDeque<ChildWrite>,
    assistant: String,
    assistant_truncated: bool,
    tools: Vec<(ToolCallId, String)>,
    notice: Option<WorkNoticeRef>,
    closing: bool,
    control: Option<ManagedMailboxJob>,
    control_operation: Option<String>,
    control_requested: bool,
    notice_started: bool,
    notice_terminal: Option<ManagedQueueStatus>,
}
impl Child {
    fn busy(&self) -> bool {
        self.starting.is_some()
            || self.turn.is_some()
            || self.work.is_some()
            || self.settlement.is_some()
            || !self.pending.is_empty()
            || self.notice_started
            || self.notice_terminal.is_some()
            || self.admission_pending
    }
}
struct ChildWrite {
    mutation: JournalMutation,
    after: WriteAfter,
}
enum WriteAfter {
    Observe,
    Start(JournalWork),
    Terminal,
    Archived,
    Notice {
        stage: Option<super::notices::StagedNotice>,
        reply: Option<(ManagedMailboxJob, String)>,
    },
}
struct Retiring {
    prepared: PreparedManagedRuntime,
    settlement: Option<(RunRef, RunSettlement)>,
}
#[allow(clippy::large_enum_variant)] // Exactly one owned operation, never a resident-sized array.
enum Active {
    Replay(BoxFuture<'static, replay::Outcome>),
    Delivery(BoxFuture<'static, delivery::Outcome>),
    Command {
        future: BoxFuture<'static, command::Outcome>,
        target: Option<String>,
        operation: String,
    },
    Child {
        id: String,
        future: BoxFuture<'static, Result<JournalSnapshot, durability::Failure>>,
        mutation: JournalMutation,
        after: WriteAfter,
    },
    Load {
        id: String,
        future: BoxFuture<'static, Result<JournalWork, super::store::JournalError>>,
    },
}

/// One outer driver. UI observations never own this queue or its children.
pub(crate) struct ManagedManager {
    journal: ManagedJournal,
    journal_owner: JournalOwner,
    mailbox: ManagedMailbox,
    factory: Arc<dyn ManagedRuntimeFactory>,
    authorizer: Arc<dyn ManagedRelationshipAuthorizer>,
    notices: Arc<ManagedNotices>,
    clock: Arc<dyn NativeMcpRuntimeClock>,
    limits: ManagerLimits,
    children: Vec<Child>,
    retiring: Vec<Retiring>,
    foregrounds: Vec<foreground::Foreground>,
    foreground_reservations: Vec<Weak<reservation::State>>,
    reservation_wake: Arc<futures_util::task::AtomicWaker>,
    active: Option<Active>,
    waiters: Vec<waiting::Waiter>,
    approvals: Vec<waiting::Approval>,
    ready_jobs: VecDeque<(ManagedMailboxJob, bool, String)>,
    pending_job: Option<(ManagedMailboxJob, Option<bool>, String)>,
    principals: Vec<Weak<super::principal::NativePrincipal>>,
    retained_notices: Vec<WorkNoticeRef>,
    parents: Vec<delivery::Parent>,
    repaired_heads: Arc<std::sync::Mutex<Vec<JournalSnapshot>>>,
    replay: replay::Replay,
    replay_reset: bool,
    retry: Arc<durability::RetryGate>,
    cursors: Arc<std::sync::Mutex<projection::CursorBook>>,
    cancellation: CancellationToken,
    deadline: Option<NoticeDeadline>,
    wait_sleep: Option<(Instant, BoxFuture<'static, ()>)>,
    next_operation: u64,
    round_robin: usize,
    closing: bool,
}
impl ManagedManager {
    #[allow(clippy::too_many_arguments)] // Explicit independent authority injections.
    pub(crate) fn new(
        journal: ManagedJournal,
        mailbox: ManagedMailbox,
        factory: Arc<dyn ManagedRuntimeFactory>,
        authorizer: Arc<dyn ManagedRelationshipAuthorizer>,
        notices: Arc<ManagedNotices>,
        clock: Arc<dyn NativeMcpRuntimeClock>,
        limits: ManagerLimits,
    ) -> Result<Self, ManagedRuntimeError> {
        if !(1..=64).contains(&limits.residents)
            || !(1..=256).contains(&limits.waiters)
            || !(1024 * 1024..=256 * 1024 * 1024).contains(&limits.buffered_bytes)
            || !(1..=256).contains(&limits.work_per_poll)
        {
            return Err(ManagedRuntimeError::Capacity);
        }
        let journal_owner = journal.owner_lease();
        Ok(Self {
            journal,
            journal_owner,
            mailbox,
            factory,
            authorizer,
            notices,
            clock,
            limits,
            children: Vec::new(),
            retiring: Vec::new(),
            foregrounds: Vec::new(),
            foreground_reservations: Vec::new(),
            reservation_wake: Arc::new(futures_util::task::AtomicWaker::new()),
            active: None,
            waiters: Vec::new(),
            approvals: Vec::new(),
            ready_jobs: VecDeque::new(),
            pending_job: None,
            principals: Vec::new(),
            retained_notices: Vec::new(),
            parents: Vec::new(),
            repaired_heads: Arc::default(),
            replay: replay::Replay::default(),
            replay_reset: false,
            retry: Arc::new(durability::RetryGate::default()),
            cursors: Arc::default(),
            cancellation: CancellationToken::new(),
            deadline: None,
            wait_sleep: None,
            next_operation: 1,
            round_robin: 0,
            closing: false,
        })
    }
    pub(crate) fn retry_reconciliation(&self) {
        self.retry.retry();
    }
    pub(crate) fn request_shutdown(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.wake_foreground_reservations();
        self.mailbox.close();
        self.pending_job.take();
        self.ready_jobs.clear();
        self.cancellation.cancel();
        self.deadline.take();
        self.wait_sleep.take();
        for foreground in &mut self.foregrounds {
            foreground.close();
        }
        for child in &self.children {
            let _ = child.prepared.runtime.request_active_cancel();
            let _ = child.prepared.runtime.clear_queued();
        }
    }
    pub(crate) fn poll_progress(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Poll<Result<ManagerProgress, ManagedRuntimeError>> {
        self.pump(cx, now_ms)
    }
    pub(crate) fn poll_shutdown(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Poll<Result<(), ManagedRuntimeError>> {
        self.request_shutdown();
        if let Poll::Ready(Err(error)) = self.pump(cx, now_ms) {
            return Poll::Ready(Err(error));
        }
        if self.children.is_empty()
            && self.retiring.is_empty()
            && self.foregrounds.is_empty()
            && !self.has_foreground_reservations()
            && self.active.is_none()
            && self.waiters.is_empty()
            && self.approvals.is_empty()
            && self.ready_jobs.is_empty()
            && self.parents.iter().all(|parent| parent.clearing.is_none())
        {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
    fn progress(&self) -> ManagerProgress {
        ManagerProgress {
            residents: self.children.len()
                + self.retiring.len()
                + self.foregrounds.len()
                + self.reserved_foregrounds(),
            executing: self
                .children
                .iter()
                .filter(|child| {
                    child
                        .prepared
                        .owner
                        .run()
                        .is_some_and(|run| run.is_executing())
                })
                .count()
                + self
                    .foregrounds
                    .iter()
                    .filter(|parent| parent.executing())
                    .count(),
            waiters: self.waiters.len(),
            closing: self.closing,
            blocked: self.retry.issue().or_else(|| {
                if self.children.iter().any(|child| child.settlement.is_some())
                    || !self.retiring.is_empty()
                    || self
                        .foregrounds
                        .iter()
                        .any(foreground::Foreground::settling)
                {
                    Some(ManagerBlock::Cleanup)
                } else if self.pending_job.is_some() || self.waiting_foreground() {
                    Some(ManagerBlock::Capacity)
                } else {
                    None
                }
            }),
        }
    }
}
impl Drop for ManagedManager {
    fn drop(&mut self) {
        self.request_shutdown();
        for child in &mut self.children {
            child.prepared.resources.begin_close();
        }
        for child in &mut self.retiring {
            child.prepared.resources.begin_close();
        }
        // Factory resources retain their journal-owner clones through actual cleanup.
        // Dropping RunSettlement never manufactures a successful completion.
    }
}
impl fmt::Debug for ManagedManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedManager")
            .field("progress", &self.progress())
            .finish_non_exhaustive()
    }
}
