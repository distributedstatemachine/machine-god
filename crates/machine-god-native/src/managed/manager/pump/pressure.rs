use super::{ManagedManager, ManagedQueueStatus};

impl ManagedManager {
    /// This observation grants no journal authority. It only stops producers
    /// before already-accepted settlement has to use its protected store credit.
    pub(super) fn observe_journal_pressure(&mut self) -> bool {
        if self.journal.ordinary_publication_available() {
            return false;
        }
        let mut changed = false;
        for child in &mut self.children {
            if child.pressure_interrupted
                || (child.work.is_none()
                    && child.starting.is_none()
                    && child.turn.is_none()
                    && !child.snapshot.head.queue.iter().any(|work| {
                        matches!(
                            work.status,
                            ManagedQueueStatus::Pending
                                | ManagedQueueStatus::Running
                                | ManagedQueueStatus::AwaitingApproval
                        )
                    }))
            {
                continue;
            }
            child.pressure_interrupted = true;
            let _ = child.prepared.runtime.request_active_cancel();
            let _ = child.prepared.runtime.clear_queued();
            changed = true;
        }
        changed
    }
}
