//! Bounded resident ownership for the native terminal host's blocking owner loop.
//! Tool calls borrow sessions; only this owner releases native resources. The
//! disk catalog is separate, so releasing inactive residency never deletes history.

use crate::terminal_catalog::{canonical_workspace, owner_name};
use crate::terminal_history::{TerminalHistoryError, TerminalHistoryEviction};
use crate::terminal_journal::{TerminalJournalPage, TerminalJournalPhysicalUsage};
use crate::terminal_profile::{
    TerminalJournalPersistence, TerminalProfileBudget, TerminalProfileError,
    TerminalProfileMutationContext,
};
use crate::terminal_profile_store::TerminalProfileStore;
use crate::terminal_session::{
    TerminalRecoveredSession, TerminalSession, TerminalSessionBackend, TerminalSessionError,
    TerminalSessionStep,
};
use crate::terminal_session_record::TerminalSessionFacts;
use machine_god_core::{
    BackgroundOutputOwner, TerminalBackend, TerminalClosePolicy, TerminalCursor,
    TerminalEventQuery, TerminalLifecycle, TerminalMonitorEvent, TerminalScreen, TerminalSessionId,
};
use std::fmt;

pub(crate) const MAX_RESIDENT_TERMINALS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalRegistryError {
    Invalid,
    NotFound,
    Conflict,
    Capacity,
    Busy,
    Closed,
    Clock,
    Session(TerminalSessionError),
}
impl From<TerminalSessionError> for TerminalRegistryError {
    fn from(error: TerminalSessionError) -> Self {
        Self::Session(error)
    }
}
type Result<T> = std::result::Result<T, TerminalRegistryError>;

#[derive(Default)]
pub(crate) struct TerminalRegistryFilter {
    pub(crate) lifecycle: Option<TerminalLifecycle>,
    pub(crate) backend: Option<TerminalBackend>,
}

enum Resident<B: TerminalSessionBackend> {
    Live(Box<TerminalSession<B>>),
    Recovered(Box<TerminalRecoveredSession>),
}
struct Entry<B: TerminalSessionBackend> {
    id: TerminalSessionId,
    owner: BackgroundOutputOwner,
    resident: Resident<B>,
}
impl<B: TerminalSessionBackend> Entry<B> {
    fn facts(&self) -> Result<TerminalSessionFacts> {
        Ok(match &self.resident {
            Resident::Live(session) => session.inspect(&self.owner)?,
            Resident::Recovered(session) => session.facts(&self.owner)?.clone(),
        })
    }
    fn active(&self) -> bool {
        matches!(&self.resident, Resident::Live(session) if matches!(session.context().lifecycle, TerminalLifecycle::Starting | TerminalLifecycle::Running))
    }
    fn now_ms(&self) -> i64 {
        match &self.resident {
            Resident::Live(session) => session.context().now_ms,
            // Registration validated ownership; this read cannot fail.
            Resident::Recovered(session) => {
                session
                    .facts(&self.owner)
                    .expect("owned recovered entry")
                    .context
                    .now_ms
            }
        }
    }
}

/// Probe descriptions remain unexecuted and must pass separate host authority.
pub(crate) struct TerminalRegistryStep {
    pub(crate) session_id: TerminalSessionId,
    pub(crate) owner: BackgroundOutputOwner,
    pub(crate) result: std::result::Result<TerminalSessionStep, TerminalSessionError>,
    /// A successful read receipt survives a separate follow-on cleanup failure.
    pub(crate) cleanup_error: Option<TerminalSessionError>,
}
pub(crate) struct TerminalRegistryFailure {
    pub(crate) session_id: TerminalSessionId,
    pub(crate) owner: BackgroundOutputOwner,
    pub(crate) error: TerminalSessionError,
}

pub(crate) struct TerminalRegistry<B: TerminalSessionBackend> {
    workspace: String,
    entries: Vec<Entry<B>>,
    next: usize,
    now_ms: i64,
    closing: bool,
}
impl<B: TerminalSessionBackend> fmt::Debug for TerminalRegistry<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalRegistry").finish_non_exhaustive()
    }
}
impl<B: TerminalSessionBackend> TerminalRegistry<B> {
    /// Pure memory construction. Native initialization and all calls belong on
    /// the host's blocking owner, never inside an async future's poll or Drop.
    pub(crate) fn new(workspace: String) -> Result<Self> {
        if !canonical_workspace(&workspace) {
            return Err(TerminalRegistryError::Invalid);
        }
        Ok(Self {
            workspace,
            entries: Vec::with_capacity(MAX_RESIDENT_TERMINALS),
            next: 0,
            now_ms: 0,
            closing: false,
        })
    }

    /// Reject duplicate/full/closing admission before invoking a native factory.
    /// Successful creation stays owned even if the requesting future disappears.
    pub(crate) fn start(
        &mut self,
        owner: BackgroundOutputOwner,
        id: TerminalSessionId,
        create: impl FnOnce() -> std::result::Result<TerminalSession<B>, TerminalSessionError>,
    ) -> Result<()> {
        self.admit(&owner, &id)?;
        let session = create()?;
        let facts = session.inspect(&owner)?;
        self.validate_facts(&facts, &id, true)?;
        self.entries.push(Entry {
            id,
            owner,
            resident: Resident::Live(Box::new(session)),
        });
        Ok(())
    }

    pub(crate) fn recover(
        &mut self,
        owner: BackgroundOutputOwner,
        id: TerminalSessionId,
        recover: impl FnOnce() -> std::result::Result<TerminalRecoveredSession, TerminalSessionError>,
    ) -> Result<()> {
        self.admit(&owner, &id)?;
        let session = recover()?;
        self.validate_facts(session.facts(&owner)?, &id, false)?;
        self.entries.push(Entry {
            id,
            owner,
            resident: Resident::Recovered(Box::new(session)),
        });
        Ok(())
    }
    fn admit(&self, owner: &BackgroundOutputOwner, id: &TerminalSessionId) -> Result<()> {
        if self.closing {
            return Err(TerminalRegistryError::Closed);
        }
        if self
            .entries
            .iter()
            .any(|entry| &entry.owner == owner && &entry.id == id)
        {
            return Err(TerminalRegistryError::Conflict);
        }
        if self.entries.len() == MAX_RESIDENT_TERMINALS {
            return Err(TerminalRegistryError::Capacity);
        }
        Ok(())
    }
    fn validate_facts(
        &self,
        facts: &TerminalSessionFacts,
        id: &TerminalSessionId,
        live: bool,
    ) -> Result<()> {
        if &facts.session_id != id
            || (live && facts.context.now_ms < self.now_ms)
            || facts
                .metadata
                .as_ref()
                .is_some_and(|metadata| metadata.workspace != self.workspace)
            || (live && facts.metadata.is_none())
        {
            return Err(TerminalRegistryError::Invalid);
        }
        Ok(())
    }
    fn index(&self, owner: &BackgroundOutputOwner, id: &TerminalSessionId) -> Result<usize> {
        self.entries
            .iter()
            .position(|entry| &entry.owner == owner && &entry.id == id)
            .ok_or(TerminalRegistryError::NotFound)
    }
    pub(crate) fn live_mut(
        &mut self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<&mut TerminalSession<B>> {
        let index = self.index(owner, id)?;
        if self.closing {
            return Err(TerminalRegistryError::Closed);
        }
        match &mut self.entries[index].resident {
            Resident::Live(session) => Ok(session),
            Resident::Recovered(_) => Err(TerminalRegistryError::Closed),
        }
    }
    pub(crate) fn inspect(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<TerminalSessionFacts> {
        self.entries[self.index(owner, id)?].facts()
    }
    pub(crate) fn list(
        &self,
        owner: &BackgroundOutputOwner,
        after: Option<&TerminalSessionId>,
        limit: usize,
        filter: &TerminalRegistryFilter,
    ) -> Result<Vec<TerminalSessionFacts>> {
        if limit == 0 || limit > 256 {
            return Err(TerminalRegistryError::Invalid);
        }
        let mut selected: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                &entry.owner == owner
                    && after.is_none_or(|after| entry.id.as_str() > after.as_str())
            })
            .collect();
        selected.sort_unstable_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        let mut facts = Vec::new();
        for entry in selected {
            let fact = entry.facts()?;
            if filter
                .lifecycle
                .is_none_or(|lifecycle| fact.context.lifecycle == lifecycle)
                && filter.backend.is_none_or(|backend| {
                    fact.metadata
                        .as_ref()
                        .is_some_and(|metadata| metadata.backend == backend)
                })
            {
                facts.push(fact);
                if facts.len() == limit {
                    break;
                }
            }
        }
        Ok(facts)
    }
    pub(crate) fn read(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        cursor: &TerminalCursor,
        maximum: usize,
    ) -> Result<TerminalJournalPage> {
        Ok(match &self.entries[self.index(owner, id)?].resident {
            Resident::Live(session) => session.read(owner, cursor, maximum)?,
            Resident::Recovered(session) => session.read(owner, cursor, maximum)?,
        })
    }
    pub(crate) fn screen(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<TerminalScreen> {
        Ok(match &self.entries[self.index(owner, id)?].resident {
            Resident::Live(session) => session.screen(owner)?,
            Resident::Recovered(session) => session.screen(owner)?,
        })
    }
    #[cfg(test)]
    pub(crate) fn events(
        &mut self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        query: &TerminalEventQuery,
    ) -> Result<Vec<TerminalMonitorEvent>> {
        self.events_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            id,
            query,
        )
    }
    pub(crate) fn events_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        query: &TerminalEventQuery,
    ) -> Result<Vec<TerminalMonitorEvent>> {
        let index = self.index(owner, id)?;
        Ok(match &mut self.entries[index].resident {
            Resident::Live(session) => session.events_with(persistence, owner, query)?,
            Resident::Recovered(session) => session.events_with(persistence, owner, query)?,
        })
    }

    pub(crate) fn physical_usage(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<TerminalJournalPhysicalUsage> {
        Ok(match &self.entries[self.index(owner, id)?].resident {
            Resident::Live(session) => session.physical_usage(owner)?,
            Resident::Recovered(session) => session.physical_usage(owner)?,
        })
    }
    pub(crate) fn eviction_bytes(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        Ok(match &self.entries[self.index(owner, id)?].resident {
            Resident::Live(session) => session.eviction_bytes(owner, kind)?,
            Resident::Recovered(session) => session.eviction_bytes(owner, kind)?,
        })
    }
    /// Only the profile coordinator selects a victim and holds the profile
    /// transaction. Session dispatch additionally enforces exact owner/lifecycle.
    #[cfg(test)]
    pub(crate) fn evict(
        &mut self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        self.evict_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            id,
            kind,
        )
    }
    pub(crate) fn evict_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        let index = self.index(owner, id)?;
        Ok(match &mut self.entries[index].resident {
            Resident::Live(session) => session.evict_with(persistence, owner, kind)?,
            Resident::Recovered(session) => session.evict_with(persistence, owner, kind)?,
        })
    }

    /// Fair bounded round-robin. One failed session cannot starve its neighbors.
    /// Returned raw chunks/probes are not queued or retained a second time here.
    #[cfg(test)]
    pub(crate) fn pump(
        &mut self,
        now_ms: i64,
        maximum: usize,
    ) -> Result<Vec<TerminalRegistryStep>> {
        self.pump_dispatch(now_ms, maximum, |session, _| {
            session.pump(now_ms).map(|step| (step, None))
        })
    }

    /// Each running session reserves bounded headroom before any native read.
    /// Pre-read profile refusals are typed per-session results, not observations:
    /// they neither consume output nor quiesce the session. Fairness advances
    /// even on refusal. No transaction survives this call or spans two sessions.
    pub(crate) fn pump_with_profile(
        &mut self,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        now_ms: i64,
        maximum: usize,
    ) -> Result<Vec<TerminalRegistryStep>> {
        let workspace = self.workspace.clone();
        self.pump_dispatch(now_ms, maximum, |session, owner| {
            let namespace = owner_name(&workspace, owner);
            let cleanup = session.needs_native_cleanup()?;
            let mut transaction = match store.transaction() {
                Ok(transaction) => transaction,
                Err(error) if cleanup => {
                    return session
                        .teardown_without_persistence(true, now_ms, profile_error(error.into()))
                        .and(Err(TerminalSessionError::InvalidState));
                }
                Err(error) => return Err(profile_error(error.into())),
            };
            if cleanup {
                let mut context =
                    TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                return session
                    .pump_with(&mut context, now_ms)
                    .map(|step| (step, None));
            }
            let mut permit = session.reserve_profile_read(&mut transaction, budget, &namespace)?;
            let mut step = session.pump_read_with(&mut permit, now_ms)?;
            drop(permit);
            let cleanup_error = if step.cleanup_needed {
                // Keep the same transaction, but release the one-read permit:
                // exited cleanup can drain multiple bounded native chunks.
                let mut context =
                    TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                let error = session.pump_with(&mut context, now_ms).err();
                let observed = session.context();
                step.cursor = observed.cursor;
                step.lifecycle = observed.lifecycle;
                step.cleanup_needed = false;
                // Cleanup quiesces monitors; pre-cleanup probes are no longer live.
                step.probes.clear();
                error
            } else {
                None
            };
            Ok((step, cleanup_error))
        })
    }

    fn pump_dispatch(
        &mut self,
        now_ms: i64,
        maximum: usize,
        mut pump: impl FnMut(
            &mut TerminalSession<B>,
            &BackgroundOutputOwner,
        ) -> std::result::Result<
            (TerminalSessionStep, Option<TerminalSessionError>),
            TerminalSessionError,
        >,
    ) -> Result<Vec<TerminalRegistryStep>> {
        if maximum == 0 || maximum > MAX_RESIDENT_TERMINALS {
            return Err(TerminalRegistryError::Invalid);
        }
        if self.closing {
            return Err(TerminalRegistryError::Closed);
        }
        self.check_time(now_ms)?;
        self.now_ms = now_ms;
        let mut steps = Vec::new();
        for _ in 0..self.entries.len() {
            let index = self.next % self.entries.len();
            self.next = (index + 1) % self.entries.len();
            let entry = &mut self.entries[index];
            if entry.active()
                && let Resident::Live(session) = &mut entry.resident
            {
                let (result, cleanup_error) = match pump(session, &entry.owner) {
                    Ok((step, error)) => (Ok(step), error),
                    Err(error) => (Err(error), None),
                };
                steps.push(TerminalRegistryStep {
                    session_id: entry.id.clone(),
                    owner: entry.owner.clone(),
                    result,
                    cleanup_error,
                });
                if steps.len() == maximum {
                    break;
                }
            }
        }
        Ok(steps)
    }
    fn check_time(&self, now_ms: i64) -> Result<()> {
        if now_ms < self.now_ms || self.entries.iter().any(|entry| now_ms < entry.now_ms()) {
            return Err(TerminalRegistryError::Clock);
        }
        Ok(())
    }
    /// Safe lower bound for owner-loop cleanup after a rejected clock reading.
    pub(crate) fn minimum_time_ms(&self) -> i64 {
        self.entries
            .iter()
            .map(Entry::now_ms)
            .fold(self.now_ms, i64::max)
    }
    /// Release only inactive residency. Its journal remains on disk. Lost
    /// sessions with unfinished native cleanup cannot be evicted as mere history.
    pub(crate) fn release(
        &mut self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<()> {
        let index = self.index(owner, id)?;
        if let Resident::Live(session) = &self.entries[index].resident {
            if session.owns_backend() {
                return Err(TerminalRegistryError::Busy);
            }
            if let Some(error) = session.publication_error() {
                return Err(error.into());
            }
        }
        if let Resident::Recovered(session) = &self.entries[index].resident
            && let Some(error) = session.publication_error()
        {
            return Err(error.into());
        }
        self.entries.remove(index);
        self.next = 0;
        Ok(())
    }
    /// Explicitly transfer a failed, natively closed history to the host's
    /// recovery owner. Unlike release, this preserves the journal lock and all
    /// in-memory facts; it neither claims durability nor silently drops them.
    pub(crate) fn take_failed_history(
        &mut self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<Box<TerminalSession<B>>> {
        let index = self.index(owner, id)?;
        match &self.entries[index].resident {
            Resident::Live(session) if session.owns_backend() => {
                return Err(TerminalRegistryError::Busy);
            }
            Resident::Live(session) if session.publication_error().is_some() => {}
            _ => return Err(TerminalRegistryError::Invalid),
        }
        let Resident::Live(session) = self.entries.remove(index).resident else {
            unreachable!("validated live history");
        };
        self.next = 0;
        Ok(session)
    }

    /// Preserve a recovered history whose new facts could not be published.
    /// Ordinary release must not silently discard that failure and journal lock.
    pub(crate) fn take_failed_recovered_history(
        &mut self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<Box<TerminalRecoveredSession>> {
        let index = self.index(owner, id)?;
        if !matches!(&self.entries[index].resident, Resident::Recovered(session) if session.publication_error().is_some())
        {
            return Err(TerminalRegistryError::Invalid);
        }
        let Resident::Recovered(session) = self.entries.remove(index).resident else {
            unreachable!("validated recovered history");
        };
        self.next = 0;
        Ok(session)
    }
    /// Stop admissions first, then attempt every owned cleanup even after error.
    /// Failed native cleanup retains authority for repeated shutdown; Drop forces one
    /// final bounded pass on the owning blocking worker.
    #[cfg(test)]
    pub(crate) fn shutdown(
        &mut self,
        now_ms: i64,
        policy: TerminalClosePolicy,
    ) -> Result<Vec<TerminalRegistryFailure>> {
        self.shutdown_dispatch(now_ms, |session, owner| {
            session.close(owner, policy, now_ms)
        })
    }

    pub(crate) fn shutdown_with_profile(
        &mut self,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        now_ms: i64,
        policy: TerminalClosePolicy,
    ) -> Result<Vec<TerminalRegistryFailure>> {
        let workspace = self.workspace.clone();
        self.shutdown_dispatch(now_ms, |session, owner| match store.transaction() {
            Ok(mut transaction) => {
                let namespace = owner_name(&workspace, owner);
                let mut context =
                    TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                session.close_with(&mut context, owner, policy, now_ms)
            }
            Err(error) => session.teardown_without_persistence(
                policy == TerminalClosePolicy::Force,
                now_ms,
                profile_error(error.into()),
            ),
        })
    }

    fn shutdown_dispatch(
        &mut self,
        now_ms: i64,
        mut close: impl FnMut(
            &mut TerminalSession<B>,
            &BackgroundOutputOwner,
        ) -> std::result::Result<(), TerminalSessionError>,
    ) -> Result<Vec<TerminalRegistryFailure>> {
        self.check_time(now_ms)?;
        self.now_ms = now_ms;
        self.closing = true;
        let mut failures = Vec::new();
        for entry in &mut self.entries {
            if let Resident::Live(session) = &mut entry.resident
                && (session.owns_backend() || session.publication_error().is_some())
                && let Err(error) = close(session, &entry.owner)
            {
                failures.push(TerminalRegistryFailure {
                    session_id: entry.id.clone(),
                    owner: entry.owner.clone(),
                    error,
                });
            }
            if let Resident::Recovered(session) = &entry.resident
                && let Some(error) = session.publication_error()
            {
                failures.push(TerminalRegistryFailure {
                    session_id: entry.id.clone(),
                    owner: entry.owner.clone(),
                    error,
                });
            }
        }
        Ok(failures)
    }
}
impl<B: TerminalSessionBackend> Drop for TerminalRegistry<B> {
    fn drop(&mut self) {
        let now_ms = self.minimum_time_ms();
        let _ = self.shutdown_dispatch(now_ms, |session, _| {
            session.teardown_without_persistence(true, now_ms, TerminalSessionError::InvalidState)
        });
    }
}

fn profile_error(error: TerminalProfileError) -> TerminalSessionError {
    TerminalSessionError::History(TerminalHistoryError::Profile(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_input::{BackgroundInputReceipt, BackgroundInputStatus};
    use crate::terminal_catalog::TerminalCatalog;
    use crate::terminal_history::TerminalHistory;
    use crate::terminal_journal::{TerminalJournal, TerminalJournalError, TerminalJournalLimits};
    use crate::terminal_monitor::TerminalMonitorActivation;
    use crate::terminal_profile::{TerminalProfileLimits, TerminalTestPersistence};
    use crate::terminal_profile_store::TerminalProfileStoreError;
    use crate::terminal_pty::{TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
    use crate::terminal_session_record::test_metadata;
    use machine_god_core::{
        SessionId, SessionIncarnationId, TerminalDimensions, TerminalMonitorCondition,
        TerminalMonitorDefinition, TerminalMonitorLifetime, TerminalMonitorOperation,
        TerminalNotifySchedule,
    };
    use rustix::fd::OwnedFd;
    use rustix::fs::{Mode, OFlags};
    use std::collections::VecDeque;
    use std::fs::DirBuilder;
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct State {
        output: VecDeque<Vec<u8>>,
        reads: usize,
        closes: usize,
        dropped: usize,
        read_fails: bool,
        read_closed: bool,
        close_fails: bool,
        statuses: VecDeque<TerminalPtyStatus>,
        tail: Vec<Vec<u8>>,
        corrupt_on_close: Option<PathBuf>,
    }
    struct Backend(Arc<Mutex<State>>);
    impl Drop for Backend {
        fn drop(&mut self) {
            self.0.lock().unwrap().dropped += 1;
        }
    }
    impl TerminalSessionBackend for Backend {
        fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            let mut state = self.0.lock().unwrap();
            state.reads += 1;
            if state.read_fails {
                return Err(());
            }
            let output = state.output.pop_front().unwrap_or_default();
            assert!(output.len() <= buffer.len());
            buffer[..output.len()].copy_from_slice(&output);
            Ok(TerminalPtyRead {
                bytes_read: output.len(),
                closed: state.read_closed,
            })
        }
        fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            Ok(BackgroundInputReceipt::new(
                bytes.len(),
                false,
                BackgroundInputStatus::Written,
            ))
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .statuses
                .pop_front()
                .unwrap_or(TerminalPtyStatus::Running))
        }
        fn resize(&mut self, _: &TerminalDimensions) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal(&mut self, _: machine_god_core::TerminalSignal) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal_may_discard_output(&self) -> bool {
            false
        }
        fn close(
            &mut self,
            _: bool,
            output: &mut dyn FnMut(&[u8]),
        ) -> std::result::Result<TerminalPtyClose, ()> {
            let mut state = self.0.lock().unwrap();
            state.closes += 1;
            if state.close_fails {
                return Err(());
            }
            if let Some(path) = state.corrupt_on_close.take() {
                // Fault injection after the normal read has committed, but
                // before the profile coordinator admits the close drain.
                std::fs::write(path, b"fault").unwrap();
            }
            for bytes in state.tail.drain(..) {
                output(&bytes);
            }
            Ok(TerminalPtyClose {
                status: TerminalPtyStatus::Exited(0),
                output_incomplete: false,
            })
        }
    }
    struct Fixture {
        path: PathBuf,
        state: Arc<Mutex<State>>,
    }
    impl Fixture {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-registry-{:032x}",
                u128::from_le_bytes(random)
            ));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self {
                path,
                state: Arc::new(Mutex::new(State::default())),
            }
        }
        fn fd(&self) -> OwnedFd {
            rustix::fs::open(
                &self.path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .unwrap()
        }
        fn live(
            &self,
            owner: &BackgroundOutputOwner,
            id: &TerminalSessionId,
            now_ms: i64,
        ) -> std::result::Result<TerminalSession<Backend>, TerminalSessionError> {
            let journal =
                TerminalJournal::create(self.fd(), id.clone(), TerminalJournalLimits::default())
                    .unwrap();
            let history =
                TerminalHistory::create(journal, &TerminalDimensions::new(3, 20).unwrap())?;
            TerminalSession::new(
                Backend(Arc::clone(&self.state)),
                history,
                owner.clone(),
                id.clone(),
                test_metadata(),
                now_ms,
            )
        }
        fn profile_live(
            &self,
            owner: &BackgroundOutputOwner,
            id: &TerminalSessionId,
        ) -> (TerminalProfileStore, TerminalSession<Backend>) {
            let store = TerminalProfileStore::prepare(self.fd()).unwrap();
            let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            let mut transaction = store.transaction().unwrap();
            let mut catalog = transaction
                .prepare_catalog("/workspace".into(), owner.clone())
                .unwrap();
            drop(transaction.create_session(&mut catalog, id).unwrap());
            let namespace = catalog.namespace_key();
            let completion = budget
                .create_journal(
                    &mut transaction,
                    namespace,
                    id,
                    TerminalJournalLimits::default(),
                )
                .unwrap();
            completion.accounting.unwrap();
            let journal = completion.operation.unwrap();
            let mut context =
                TerminalProfileMutationContext::new(&mut transaction, budget, namespace);
            let history = TerminalHistory::create_with(
                &mut context,
                journal,
                &TerminalDimensions::new(3, 20).unwrap(),
            )
            .unwrap();
            let session = TerminalSession::new_with(
                &mut context,
                Backend(Arc::clone(&self.state)),
                history,
                owner.clone(),
                id.clone(),
                test_metadata(),
                0,
            )
            .unwrap();
            drop(transaction);
            (store, session)
        }
        fn recovered(
            &self,
            owner: &BackgroundOutputOwner,
            id: &TerminalSessionId,
            now_ms: i64,
        ) -> std::result::Result<TerminalRecoveredSession, TerminalSessionError> {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let journal = loop {
                match TerminalJournal::open_existing(
                    self.fd(),
                    id,
                    TerminalJournalLimits::default(),
                ) {
                    Err(TerminalJournalError::Busy) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    result => break result.unwrap(),
                }
            };
            TerminalRecoveredSession::recover(TerminalHistory::recover(journal)?, owner, now_ms)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn owner(incarnation: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new(incarnation).unwrap(),
        )
    }
    fn id(value: &str) -> TerminalSessionId {
        TerminalSessionId::new(value).unwrap()
    }
    fn registry() -> TerminalRegistry<Backend> {
        TerminalRegistry::new("/workspace".into()).unwrap()
    }

    #[test]
    fn profile_busy_and_capacity_defer_before_read_and_later_progress() {
        let fixture = Fixture::new();
        let owner = owner("profile");
        let id = id("profile-read");
        let (store, session) = fixture.profile_live(&owner, &id);
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"kept".to_vec());
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let held = store.transaction().unwrap();
        let before = registry.inspect(&owner, &id).unwrap();
        let steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
        assert!(matches!(
            steps[0].result,
            Err(TerminalSessionError::History(
                TerminalHistoryError::Profile(TerminalProfileError::Store(
                    TerminalProfileStoreError::Busy
                ))
            ))
        ));
        drop(held);
        let mut limits = TerminalProfileLimits::default();
        limits.retained.output_bytes = 1;
        let limited = TerminalProfileBudget::new(limits).unwrap();
        let steps = registry.pump_with_profile(&store, &limited, 2, 1).unwrap();
        assert!(matches!(
            steps[0].result,
            Err(TerminalSessionError::History(
                TerminalHistoryError::Profile(TerminalProfileError::ResourceLimit)
            ))
        ));
        assert_eq!(fixture.state.lock().unwrap().reads, 0);
        let after = registry.inspect(&owner, &id).unwrap();
        assert_eq!(after.context.cursor, before.context.cursor);
        assert_eq!(after.context.lifecycle, before.context.lifecycle);
        assert_eq!(after.context.now_ms, before.context.now_ms);
        assert!(
            registry
                .live_mut(&owner, &id)
                .unwrap()
                .publication_error()
                .is_none()
        );
        let mut steps = registry.pump_with_profile(&store, &budget, 3, 1).unwrap();
        assert_eq!(steps.remove(0).result.unwrap().output, b"kept");
        assert_eq!(fixture.state.lock().unwrap().reads, 1);
        assert!(store.transaction().is_ok());
        assert!(
            registry
                .shutdown_with_profile(&store, &budget, 3, TerminalClosePolicy::Force)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn profile_shutdown_without_lock_closes_native_and_retains_failed_history() {
        let fixture = Fixture::new();
        let owner = owner("profile");
        let id = id("shutdown");
        let (store, session) = fixture.profile_live(&owner, &id);
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let held = store.transaction().unwrap();
        let before = held.inventory().unwrap().usage;
        let failures = registry
            .shutdown_with_profile(&store, &budget, 1, TerminalClosePolicy::Force)
            .unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert_eq!(held.inventory().unwrap().usage, before);
        assert!(matches!(
            registry.release(&owner, &id),
            Err(TerminalRegistryError::Session(_))
        ));
        let failed = registry.take_failed_history(&owner, &id).unwrap();
        assert!(!failed.owns_backend());
        assert!(failed.publication_error().is_some());
        drop(failed);
        drop(held);
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
    }

    #[test]
    fn known_exit_is_cleaned_even_when_normal_read_capacity_is_full() {
        let fixture = Fixture::new();
        let owner = owner("exit");
        let id = id("full-profile-exit");
        let (store, session) = fixture.profile_live(&owner, &id);
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        let existing = registry.physical_usage(&owner, &id).unwrap().output_bytes;
        let mut limits = TerminalProfileLimits::default();
        limits.retained.output_bytes = existing.max(1);
        let budget = TerminalProfileBudget::new(limits).unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .statuses
            .extend([TerminalPtyStatus::Exited(0); 2]);
        let steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
        assert!(steps[0].result.is_ok());
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.reads, 0);
        assert_eq!(state.closes, 1);
        drop(state);
        assert!(!registry.live_mut(&owner, &id).unwrap().owns_backend());
        assert_eq!(
            registry.inspect(&owner, &id).unwrap().context.lifecycle,
            TerminalLifecycle::Exited
        );
    }

    #[test]
    fn known_exit_without_profile_lock_still_closes_native() {
        let fixture = Fixture::new();
        let owner = owner("exit");
        let id = id("busy-profile-exit");
        let (store, session) = fixture.profile_live(&owner, &id);
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let held = store.transaction().unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .statuses
            .push_back(TerminalPtyStatus::Exited(0));
        let steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
        assert!(steps[0].result.is_err());
        assert_eq!(fixture.state.lock().unwrap().reads, 0);
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert!(
            registry
                .live_mut(&owner, &id)
                .unwrap()
                .publication_error()
                .is_some()
        );
        drop(held);
    }

    #[test]
    fn exit_after_preflight_releases_read_permit_before_multichunk_drain() {
        for after_read in [false, true] {
            let fixture = Fixture::new();
            let owner = owner("exit");
            let id = id("exit-race");
            let (store, session) = fixture.profile_live(&owner, &id);
            let mut registry = registry();
            registry
                .start(owner.clone(), id.clone(), || Ok(session))
                .unwrap();
            let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            let mut expected = Vec::new();
            {
                let mut state = fixture.state.lock().unwrap();
                state.statuses.push_back(TerminalPtyStatus::Running);
                if after_read {
                    state.statuses.push_back(TerminalPtyStatus::Running);
                    state.output.push_back(vec![b'r'; 16 * 1024]);
                    state.read_closed = true;
                    expected.extend(vec![b'r'; 16 * 1024]);
                }
                state.statuses.extend([TerminalPtyStatus::Exited(0); 2]);
                state.tail = vec![vec![b'a'; 16 * 1024], vec![b'b'; 16 * 1024], vec![b'c'; 7]];
                for bytes in &state.tail {
                    expected.extend(bytes);
                }
            }
            let mut steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
            let receipt = steps.remove(0);
            assert!(receipt.cleanup_error.is_none());
            let step = receipt.result.unwrap();
            assert_eq!(step.output.len(), if after_read { 16 * 1024 } else { 0 });
            assert_eq!(step.lifecycle, TerminalLifecycle::Exited);
            assert!(!step.cleanup_needed);
            assert!(step.probes.is_empty());
            assert_eq!(
                step.cursor,
                registry.inspect(&owner, &id).unwrap().context.cursor
            );
            let page = registry
                .read(&owner, &id, &TerminalCursor::new(1, 0).unwrap(), 64 * 1024)
                .unwrap();
            assert_eq!(page.bytes, expected);
            assert_eq!(fixture.state.lock().unwrap().reads, usize::from(after_read));
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
            assert!(store.transaction().is_ok());
        }
    }

    #[test]
    fn committed_read_receipt_survives_follow_on_cleanup_publication_failure() {
        let fixture = Fixture::new();
        let owner = owner("exit");
        let id = id("cleanup-receipt");
        let (store, session) = fixture.profile_live(&owner, &id);
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        {
            let mut state = fixture.state.lock().unwrap();
            state.statuses.extend([
                TerminalPtyStatus::Running,
                TerminalPtyStatus::Running,
                TerminalPtyStatus::Exited(0),
                TerminalPtyStatus::Exited(0),
            ]);
            state.read_closed = true;
            state.output.push_back(b"receipt".to_vec());
            state.tail = vec![b"tail".to_vec()];
            state.corrupt_on_close = Some(
                fixture
                    .path
                    .join("terminal-v1")
                    .join(owner_name("/workspace", &owner))
                    .join("sessions")
                    .join(id.as_str())
                    .join("unrecognized"),
            );
        }
        let mut steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
        let receipt = steps.remove(0);
        assert!(
            receipt.result.is_ok(),
            "pre-read refusal: {:?}",
            receipt.result.as_ref().err()
        );
        assert!(receipt.cleanup_error.is_some());
        let step = receipt.result.unwrap();
        assert_eq!(step.output, b"receipt");
        assert!(step.probes.is_empty());
        assert_eq!(
            step.cursor,
            registry.inspect(&owner, &id).unwrap().context.cursor
        );
        assert_eq!(
            step.lifecycle,
            registry.inspect(&owner, &id).unwrap().context.lifecycle
        );
        assert!(
            registry
                .live_mut(&owner, &id)
                .unwrap()
                .publication_error()
                .is_some()
        );
        assert_eq!(fixture.state.lock().unwrap().reads, 1);
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        let page = registry
            .read(&owner, &id, &TerminalCursor::new(1, 0).unwrap(), 64 * 1024)
            .unwrap();
        assert!(page.bytes.starts_with(b"receipt"));
        assert_eq!(
            page.bytes
                .windows(7)
                .filter(|bytes| *bytes == b"receipt")
                .count(),
            1
        );
    }

    #[test]
    fn profile_owner_releases_transaction_before_observer_and_request_callback() {
        let fixture = Fixture::new();
        let owner = owner("profile");
        let id = id("owner-profile");
        let (store, session) = fixture.profile_live(&owner, &id);
        let store = Arc::new(store);
        let mut registry = registry();
        registry.start(owner, id, || Ok(session)).unwrap();
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let (worker, handle) = crate::terminal_owner::TerminalOwnerLoop::new();
        let callback_store = Arc::clone(&store);
        let stop = handle.clone();
        let mut request = handle.request(
            machine_god_core::CancellationToken::new(),
            move |_, _, _| {
                assert!(callback_store.transaction().is_ok());
                stop.shutdown();
                true
            },
        );
        assert!(poll_owner(&mut request).is_pending());
        let mut observed = false;
        let exit = worker.run_with_profile(
            &mut registry,
            &store,
            &budget,
            || 1,
            |steps| {
                assert!(store.transaction().is_ok());
                assert!(steps[0].result.is_ok());
                observed = true;
            },
        );
        assert!(observed);
        assert!(exit.error.is_none());
        assert!(exit.shutdown.unwrap().is_empty());
        assert_eq!(poll_owner(&mut request), std::task::Poll::Ready(Ok(true)));
    }

    #[test]
    fn recovered_publication_failure_requires_explicit_transfer() {
        struct Denied;
        impl TerminalJournalPersistence for Denied {
            fn mutate(
                &mut self,
                _: &mut TerminalJournal,
                _: crate::terminal_journal::TerminalJournalMutation<'_>,
            ) -> std::result::Result<
                crate::terminal_profile::TerminalProfileCompletion<
                    crate::terminal_journal::TerminalJournalReceipt,
                    TerminalJournalError,
                >,
                TerminalProfileError,
            > {
                Err(TerminalProfileError::ResourceLimit)
            }
        }
        for acknowledge in [false, true] {
            let fixture = Fixture::new();
            let owner = owner("recovered");
            let id = id("failed-recovered");
            let mut live = fixture.live(&owner, &id, 0).unwrap();
            live.monitor(
                &owner,
                TerminalMonitorOperation::Add {
                    definition: TerminalMonitorDefinition {
                        condition: TerminalMonitorCondition::OutputContains {
                            pattern: "kept".into(),
                        },
                        check_schedule: None,
                        notify: TerminalNotifySchedule::OnMatch,
                        lifetime: TerminalMonitorLifetime::UntilSessionEnd,
                    },
                },
                TerminalMonitorActivation::default(),
                0,
            )
            .unwrap();
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"kept".to_vec());
            live.pump(1).unwrap();
            live.close(&owner, TerminalClosePolicy::Force, 2).unwrap();
            drop(live);
            let mut registry = registry();
            registry
                .recover(owner.clone(), id.clone(), || {
                    fixture.recovered(&owner, &id, 2)
                })
                .unwrap();
            if acknowledge {
                let mut query = TerminalEventQuery {
                    after_event_id: 0,
                    acknowledge_event_id: None,
                    max_events: 256,
                };
                let events = registry
                    .events_with(&mut TerminalTestPersistence, &owner, &id, &query)
                    .unwrap();
                query.acknowledge_event_id = Some(events.last().unwrap().event_id);
                assert!(
                    registry
                        .events_with(&mut Denied, &owner, &id, &query)
                        .is_err()
                );
            } else {
                assert!(
                    registry
                        .evict_with(
                            &mut Denied,
                            &owner,
                            &id,
                            TerminalHistoryEviction::CompletedOutput
                        )
                        .is_err()
                );
            }
            assert!(matches!(
                registry.release(&owner, &id),
                Err(TerminalRegistryError::Session(_))
            ));
            assert!(matches!(
                registry.take_failed_recovered_history(&self::owner("wrong"), &id),
                Err(TerminalRegistryError::NotFound)
            ));
            assert_eq!(
                registry
                    .shutdown(2, TerminalClosePolicy::Force)
                    .unwrap()
                    .len(),
                1
            );
            let failed = registry.take_failed_recovered_history(&owner, &id).unwrap();
            assert!(failed.publication_error().is_some());
        }
    }

    fn poll_owner<T: Send + 'static>(
        future: &mut crate::terminal_owner::TerminalOwnerFuture<Backend, T>,
    ) -> std::task::Poll<std::result::Result<T, crate::terminal_owner::TerminalOwnerError>> {
        use std::future::Future;
        std::pin::Pin::new(future)
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
    }

    #[test]
    fn retention_dispatch_checks_owner_lifecycle_and_preserves_facts_across_recovery() {
        use TerminalHistoryEviction::{CompletedCheckpoint, CompletedOutput, LiveCoveredOutput};
        let fixture = Fixture::new();
        let mut registry = registry();
        let other_owner = owner("other");
        let owner = owner("one");
        let id = id("retention");
        registry
            .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
            .unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"retain facts".to_vec());
        registry.pump(1, 1).unwrap()[0].result.as_ref().unwrap();
        let before = registry.physical_usage(&owner, &id).unwrap();
        for kind in [CompletedOutput, CompletedCheckpoint] {
            assert!(matches!(
                registry.eviction_bytes(&owner, &id, kind),
                Err(TerminalRegistryError::Session(
                    TerminalSessionError::InvalidState
                ))
            ));
            assert!(matches!(
                registry.evict(&owner, &id, kind),
                Err(TerminalRegistryError::Session(
                    TerminalSessionError::InvalidState
                ))
            ));
        }
        assert!(matches!(
            registry.physical_usage(&other_owner, &id),
            Err(TerminalRegistryError::NotFound)
        ));
        assert!(matches!(
            registry.evict(&other_owner, &id, LiveCoveredOutput),
            Err(TerminalRegistryError::NotFound)
        ));
        assert_eq!(registry.physical_usage(&owner, &id).unwrap(), before);
        registry
            .live_mut(&owner, &id)
            .unwrap()
            .close(&owner, TerminalClosePolicy::Force, 2)
            .unwrap();
        let facts = serde_json::to_value(registry.inspect(&owner, &id).unwrap()).unwrap();
        assert_eq!(
            registry
                .eviction_bytes(&owner, &id, CompletedOutput)
                .unwrap(),
            b"retain facts".len()
        );
        assert_eq!(
            registry.evict(&owner, &id, CompletedOutput).unwrap(),
            b"retain facts".len()
        );
        assert_eq!(registry.physical_usage(&owner, &id).unwrap().raw_bytes, 0);
        assert!(matches!(
            registry.evict(&owner, &id, LiveCoveredOutput),
            Err(TerminalRegistryError::Session(
                TerminalSessionError::InvalidState
            ))
        ));
        registry.release(&owner, &id).unwrap();
        registry
            .recover(owner.clone(), id.clone(), || {
                fixture.recovered(&owner, &id, 2)
            })
            .unwrap();
        assert_eq!(
            serde_json::to_value(registry.inspect(&owner, &id).unwrap()).unwrap(),
            facts
        );
        assert!(registry.evict(&owner, &id, CompletedCheckpoint).unwrap() > 0);
        let retained = registry.physical_usage(&owner, &id).unwrap();
        assert_eq!(retained.checkpoint_bytes, 0);
        assert_eq!(
            serde_json::to_value(registry.inspect(&owner, &id).unwrap()).unwrap(),
            facts
        );
        registry.release(&owner, &id).unwrap();
        registry
            .recover(owner.clone(), id.clone(), || {
                fixture.recovered(&owner, &id, 2)
            })
            .unwrap();
        assert_eq!(
            serde_json::to_value(registry.inspect(&owner, &id).unwrap()).unwrap(),
            facts
        );
        assert!(matches!(
            registry.screen(&owner, &id),
            Err(TerminalRegistryError::Session(
                TerminalSessionError::History(_)
            ))
        ));
    }

    #[test]
    fn failed_publication_is_not_completed_retention_authority() {
        let fixture = Fixture::new();
        let mut registry = registry();
        let owner = owner("one");
        let id = id("failed-retention");
        registry
            .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
            .unwrap();
        let _temporary = rustix::fs::openat(
            fixture.fd(),
            "tj-meta.tmp",
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_bits_retain(0o600),
        )
        .unwrap();
        assert!(
            registry
                .live_mut(&owner, &id)
                .unwrap()
                .close(&owner, TerminalClosePolicy::Force, 1)
                .is_err()
        );
        assert!(matches!(
            registry.evict(&owner, &id, TerminalHistoryEviction::CompletedOutput),
            Err(TerminalRegistryError::Session(
                TerminalSessionError::History(_)
            ))
        ));
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
    }

    #[test]
    fn owner_pumps_between_tool_calls_and_closes_native_only_on_host_shutdown() {
        use crate::terminal_owner::TerminalOwnerLoop;
        use machine_god_core::CancellationToken;
        let fixture = Fixture::new();
        let mut registry = registry();
        let owner = owner("one");
        let id = id("continuous");
        registry
            .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
            .unwrap();
        let (worker, handle) = TerminalOwnerLoop::new();
        let (output_sender, output_receiver) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let exit = worker.run(
                &mut registry,
                || i64::try_from(start.elapsed().as_millis()).unwrap(),
                |steps| {
                    for step in steps {
                        if let Ok(step) = step.result
                            && !step.output.is_empty()
                        {
                            output_sender.try_send(step.output).unwrap();
                        }
                    }
                },
            );
            assert!(exit.error.is_none());
            assert!(exit.shutdown.unwrap().is_empty());
            registry.inspect(&owner, &id).unwrap()
        });
        let mut request = handle.request(CancellationToken::new(), |_, _, _| 17);
        assert_eq!(futures_executor::block_on(&mut request), Ok(17));
        drop(request);
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"between calls".to_vec());
        assert_eq!(
            output_receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            b"between calls"
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        drop(handle);
        assert_eq!(
            thread.join().unwrap().context.lifecycle,
            TerminalLifecycle::Closed
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
    }

    #[test]
    fn owner_requests_are_inert_cancel_before_effect_and_preserve_committed_results() {
        use crate::terminal_owner::{TerminalOwnerError, TerminalOwnerLoop};
        use machine_god_core::CancellationToken;
        let (worker, handle) = TerminalOwnerLoop::<Backend>::new();
        let unpolled = handle.request(CancellationToken::new(), |_, _, _| panic!("unpolled"));
        drop(unpolled);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut cancelled = handle.request(cancellation, |_, _, _| {
            panic!("cancelled before submission")
        });
        assert!(matches!(
            poll_owner(&mut cancelled),
            std::task::Poll::Ready(Err(TerminalOwnerError::Cancelled))
        ));
        let cancellation = CancellationToken::new();
        let mut queued =
            handle.request(cancellation.clone(), |_, _, _| panic!("cancelled in queue"));
        assert!(poll_owner(&mut queued).is_pending());
        cancellation.cancel();
        let mut abandoned = handle.request(CancellationToken::new(), |_, _, _| {
            panic!("abandoned in queue")
        });
        assert!(poll_owner(&mut abandoned).is_pending());
        drop(abandoned);
        let thread = std::thread::spawn(move || worker.run(&mut registry(), || 0, |_| {}));
        assert_eq!(
            futures_executor::block_on(queued),
            Err(TerminalOwnerError::Cancelled)
        );
        let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(1);
        let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(1);
        let cancellation = CancellationToken::new();
        let mut committed = handle.request(cancellation.clone(), move |_, _, operation| {
            started_sender.send(()).unwrap();
            release_receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            assert!(operation.is_cancelled());
            42 // committed receipts must not be relabelled as cancellation
        });
        assert!(poll_owner(&mut committed).is_pending());
        started_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        cancellation.cancel();
        assert!(poll_owner(&mut committed).is_pending());
        release_sender.send(()).unwrap();
        assert_eq!(futures_executor::block_on(committed), Ok(42));
        handle.shutdown();
        assert!(thread.join().unwrap().shutdown.unwrap().is_empty());
    }

    #[test]
    fn owner_bounds_queued_and_unconsumed_results_and_resolves_closed_requests() {
        use crate::terminal_owner::{TerminalOwnerError, TerminalOwnerLoop};
        use machine_god_core::CancellationToken;
        let (worker, handle) = TerminalOwnerLoop::<Backend>::new();
        let (sent, received) = std::sync::mpsc::sync_channel(32);
        let mut requests: Vec<_> = (0..32)
            .map(|n| {
                let sent = sent.clone();
                let mut request = handle.request(CancellationToken::new(), move |_, _, _| {
                    sent.send(()).unwrap();
                    n
                });
                assert!(poll_owner(&mut request).is_pending());
                request
            })
            .collect();
        let mut excess = handle.request(CancellationToken::new(), |_, _, _| ());
        assert_eq!(
            poll_owner(&mut excess),
            std::task::Poll::Ready(Err(TerminalOwnerError::Busy))
        );
        let thread = std::thread::spawn(move || worker.run(&mut registry(), || 0, |_| {}));
        for _ in 0..32 {
            received
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
        }
        let mut excess = handle.request(CancellationToken::new(), |_, _, _| ());
        assert_eq!(
            poll_owner(&mut excess),
            std::task::Poll::Ready(Err(TerminalOwnerError::Busy))
        );
        assert_eq!(futures_executor::block_on(requests.remove(0)), Ok(0));
        assert_eq!(
            futures_executor::block_on(handle.request(CancellationToken::new(), |_, _, _| 33)),
            Ok(33)
        );
        handle.shutdown();
        assert!(thread.join().unwrap().shutdown.unwrap().is_empty());
        for (n, request) in requests.into_iter().enumerate() {
            assert_eq!(futures_executor::block_on(request), Ok(n + 1));
        }
        let mut closed = handle.request(CancellationToken::new(), |_, _, _| panic!("closed"));
        assert!(matches!(
            poll_owner(&mut closed),
            std::task::Poll::Ready(Err(TerminalOwnerError::Closed))
        ));
        let (worker, handle) = TerminalOwnerLoop::<Backend>::new();
        let mut request = handle.request(CancellationToken::new(), |_, _, _| ());
        assert!(poll_owner(&mut request).is_pending());
        drop(worker);
        assert_eq!(
            futures_executor::block_on(request),
            Err(TerminalOwnerError::Closed)
        );
    }

    #[test]
    fn owner_rejects_every_request_and_cleans_up_after_panicking_wakers_or_destructors() {
        use crate::terminal_owner::{TerminalOwnerError, TerminalOwnerLoop};
        use machine_god_core::CancellationToken;
        use std::future::Future;
        use std::task::{Context, Wake, Waker};
        struct PanickingWake;
        impl Wake for PanickingWake {
            fn wake(self: Arc<Self>) {
                panic!("caller wake panic");
            }
        }
        struct PanickingDrop;
        impl Drop for PanickingDrop {
            fn drop(&mut self) {
                panic!("rejected closure capture panic");
            }
        }
        let fixture = Fixture::new();
        let mut registry = registry();
        let owner = owner("one");
        let id = id("cleanup");
        registry
            .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
            .unwrap();
        let (worker, handle) = TerminalOwnerLoop::new();
        let mut waking = handle.request(CancellationToken::new(), |_, _, _| {
            panic!("rejected operation")
        });
        let wake = Waker::from(Arc::new(PanickingWake));
        assert!(
            std::pin::Pin::new(&mut waking)
                .poll(&mut Context::from_waker(&wake))
                .is_pending()
        );
        let capture = PanickingDrop;
        let mut dropping = handle.request(CancellationToken::new(), move |_, _, _| {
            drop(capture);
        });
        // Both callbacks on this same rejected request panic independently.
        assert!(
            std::pin::Pin::new(&mut dropping)
                .poll(&mut Context::from_waker(&wake))
                .is_pending()
        );
        let mut last = handle.request(CancellationToken::new(), |_, _, _| ());
        assert!(poll_owner(&mut last).is_pending());
        handle.shutdown();
        let exit = worker.run(&mut registry, || 0, |_| {});
        assert_eq!(exit.error, Some(TerminalOwnerError::Panicked));
        assert!(exit.shutdown.unwrap().is_empty());
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert_eq!(
            futures_executor::block_on(waking),
            Err(TerminalOwnerError::Closed)
        );
        assert_eq!(
            futures_executor::block_on(dropping),
            Err(TerminalOwnerError::Closed)
        );
        assert_eq!(
            futures_executor::block_on(last),
            Err(TerminalOwnerError::Closed)
        );

        // Dropping an unstarted loop must contain rejection panics as well,
        // including when Drop is itself reached during another unwind.
        let (worker, handle) = TerminalOwnerLoop::<Backend>::new();
        let mut pending = handle.request(CancellationToken::new(), |_, _, _| ());
        assert!(
            std::pin::Pin::new(&mut pending)
                .poll(&mut Context::from_waker(&wake))
                .is_pending()
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _worker = worker;
            panic!("outer panic");
        }));
        assert!(result.is_err());
        assert_eq!(
            futures_executor::block_on(pending),
            Err(TerminalOwnerError::Closed)
        );
    }

    #[test]
    fn owner_secondary_panic_payload_drop_cannot_bypass_shutdown() {
        use crate::terminal_owner::{TerminalOwnerError, TerminalOwnerLoop};
        use machine_god_core::CancellationToken;
        use std::future::Future;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::task::{Context, Wake, Waker};
        struct Payload(Arc<AtomicBool>);
        impl Drop for Payload {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
                panic!("secondary payload destructor must not run");
            }
        }
        struct Capture(Arc<AtomicBool>);
        impl Drop for Capture {
            fn drop(&mut self) {
                std::panic::panic_any(Payload(Arc::clone(&self.0)));
            }
        }
        struct CallerWake(Arc<AtomicBool>);
        impl Wake for CallerWake {
            fn wake(self: Arc<Self>) {
                std::panic::panic_any(Payload(Arc::clone(&self.0)));
            }
        }
        let secondary = Arc::new(AtomicBool::new(false));
        let wake = Waker::from(Arc::new(CallerWake(Arc::clone(&secondary))));
        let fixture = Fixture::new();
        let mut registry = registry();
        let owner = owner("one");
        let id = id("secondary-panic");
        registry
            .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
            .unwrap();
        let (worker, handle) = TerminalOwnerLoop::new();
        let capture = Capture(Arc::clone(&secondary));
        let mut request = handle.request(CancellationToken::new(), move |_, _, _| drop(capture));
        assert!(
            std::pin::Pin::new(&mut request)
                .poll(&mut Context::from_waker(&wake))
                .is_pending()
        );
        handle.shutdown();
        let exit = worker.run(&mut registry, || 0, |_| {});
        assert_eq!(exit.error, Some(TerminalOwnerError::Panicked));
        assert!(exit.shutdown.unwrap().is_empty());
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert_eq!(
            futures_executor::block_on(request),
            Err(TerminalOwnerError::Closed)
        );
        assert!(!secondary.load(Ordering::Acquire));
    }

    #[test]
    fn owner_panic_and_clock_failure_stop_admissions_and_cleanup() {
        use crate::terminal_owner::{TerminalOwnerError, TerminalOwnerLoop};
        use machine_god_core::CancellationToken;
        for panic_in_observer in [false, true] {
            let fixture = Fixture::new();
            let mut registry = registry();
            let owner = owner("one");
            let id = id("panic");
            registry
                .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
                .unwrap();
            let (worker, handle) = TerminalOwnerLoop::new();
            let mut request = handle.request(CancellationToken::new(), |_, _, _| {
                panic!("operation panic")
            });
            assert!(poll_owner(&mut request).is_pending());
            let mut rejected = handle.request(CancellationToken::new(), |_, _, _| {
                panic!("must not execute after failure")
            });
            assert!(poll_owner(&mut rejected).is_pending());
            let exit = worker.run(
                &mut registry,
                || 0,
                |_| {
                    assert!(!panic_in_observer, "observer panic");
                },
            );
            assert_eq!(exit.error, Some(TerminalOwnerError::Panicked));
            assert!(exit.shutdown.unwrap().is_empty());
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
            assert_eq!(
                futures_executor::block_on(request),
                Err(if panic_in_observer {
                    TerminalOwnerError::Closed
                } else {
                    TerminalOwnerError::Panicked
                })
            );
            assert_eq!(
                futures_executor::block_on(rejected),
                Err(TerminalOwnerError::Closed)
            );
        }
        let (worker, _handle) = TerminalOwnerLoop::<Backend>::new();
        let mut time = 2;
        let exit = worker.run(
            &mut registry(),
            || {
                time -= 1;
                time
            },
            |_| {},
        );
        assert_eq!(
            exit.error,
            Some(TerminalOwnerError::Registry(TerminalRegistryError::Clock))
        );
        assert!(exit.shutdown.unwrap().is_empty());
    }

    #[test]
    fn descriptor_catalog_and_registry_compose_across_host_shutdown() {
        let fixture = Fixture::new();
        let owner = owner("one");
        let id = id("terminal");
        let mut catalog =
            TerminalCatalog::prepare(fixture.fd(), "/workspace".into(), owner.clone()).unwrap();
        let mut first = registry();
        first
            .start(owner.clone(), id.clone(), || {
                let journal = TerminalJournal::create(
                    catalog.create(&id).unwrap(),
                    id.clone(),
                    TerminalJournalLimits::default(),
                )
                .unwrap();
                let history =
                    TerminalHistory::create(journal, &TerminalDimensions::new(3, 20).unwrap())?;
                TerminalSession::new(
                    Backend(Arc::clone(&fixture.state)),
                    history,
                    owner.clone(),
                    id.clone(),
                    test_metadata(),
                    0,
                )
            })
            .unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"durable catalog".to_vec());
        first.pump(1, 1).unwrap();
        drop(first);
        assert_eq!(catalog.list().unwrap(), vec![id.clone()]);
        let mut second = registry();
        second.pump(50, 1).unwrap();
        second
            .recover(owner.clone(), id.clone(), || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                let journal = loop {
                    match TerminalJournal::open_existing(
                        catalog.open(&id).unwrap(),
                        &id,
                        TerminalJournalLimits::default(),
                    ) {
                        Err(TerminalJournalError::Busy) if std::time::Instant::now() < deadline => {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        result => break result.unwrap(),
                    }
                };
                TerminalRecoveredSession::recover(TerminalHistory::recover(journal)?, &owner, 50)
            })
            .unwrap();
        assert_eq!(
            second
                .read(&owner, &id, &TerminalCursor::new(1, 0).unwrap(), 64)
                .unwrap()
                .bytes,
            b"durable catalog"
        );
        let facts = second
            .list(&owner, None, 1, &TerminalRegistryFilter::default())
            .unwrap();
        assert_eq!(facts[0].context.lifecycle, TerminalLifecycle::Lost);
        assert_eq!(facts[0].metadata.as_ref().unwrap().workspace, "/workspace");
        assert!(matches!(
            second.live_mut(&owner, &id),
            Err(TerminalRegistryError::Closed)
        ));
        assert!(
            second
                .events(
                    &owner,
                    &id,
                    &TerminalEventQuery {
                        after_event_id: 0,
                        acknowledge_event_id: None,
                        max_events: 1
                    }
                )
                .unwrap()
                .is_empty()
        );
        second.release(&owner, &id).unwrap();
        assert_eq!(catalog.list().unwrap(), vec![id]);
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
    }

    #[test]
    fn factory_facts_cannot_cross_owner_workspace_or_identifier() {
        for mismatch in 0..3 {
            let fixture = Fixture::new();
            let owner = owner("one");
            let id = id("requested");
            let mut registry = TerminalRegistry::new(
                if mismatch == 0 {
                    "/other"
                } else {
                    "/workspace"
                }
                .into(),
            )
            .unwrap();
            let actual_owner = if mismatch == 1 {
                super::tests::owner("two")
            } else {
                owner.clone()
            };
            let actual_id = if mismatch == 2 {
                super::tests::id("actual")
            } else {
                id.clone()
            };
            assert!(
                registry
                    .start(owner.clone(), id.clone(), || fixture.live(
                        &actual_owner,
                        &actual_id,
                        0
                    ))
                    .is_err()
            );
            assert!(
                registry
                    .list(&owner, None, 256, &TerminalRegistryFilter::default())
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(fixture.state.lock().unwrap().dropped, 1);
        }
    }

    #[test]
    fn borrowed_calls_do_not_own_lifetime_and_drop_retains_readable_history() {
        let fixture = Fixture::new();
        let owner = owner("one");
        let id = id("terminal");
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
            .unwrap();
        {
            let session = registry.live_mut(&owner, &id).unwrap();
            session.shell_ready(0).unwrap();
        }
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"between calls".to_vec());
        let steps = registry.pump(1, 1).unwrap();
        assert_eq!(steps[0].owner, owner);
        assert_eq!(steps[0].result.as_ref().unwrap().output, b"between calls");
        assert_eq!(
            registry
                .read(&owner, &id, &TerminalCursor::new(1, 0).unwrap(), 64)
                .unwrap()
                .bytes,
            b"between calls"
        );
        assert_eq!(
            registry.release(&owner, &id),
            Err(TerminalRegistryError::Busy)
        );
        drop(registry);
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        let recovered = fixture.recovered(&owner, &id, 100).unwrap();
        assert_eq!(
            recovered.facts(&owner).unwrap().context.lifecycle,
            TerminalLifecycle::Lost
        );
        assert_eq!(
            recovered
                .read(&owner, &TerminalCursor::new(1, 0).unwrap(), 64)
                .unwrap()
                .bytes,
            b"between calls"
        );
    }

    #[test]
    fn catalog_is_owner_scoped_sorted_filtered_and_paged() {
        let fixtures: Vec<_> = (0..3).map(|_| Fixture::new()).collect();
        let mut registry = registry();
        for (fixture, (owner, id)) in fixtures.iter().zip([
            (owner("one"), id("b")),
            (owner("one"), id("a")),
            (owner("two"), id("a")),
        ]) {
            registry
                .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
                .unwrap();
        }
        assert!(
            registry
                .list(
                    &owner("wrong"),
                    None,
                    256,
                    &TerminalRegistryFilter::default()
                )
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            registry.inspect(&owner("wrong"), &id("a")),
            Err(TerminalRegistryError::NotFound)
        ));
        assert!(matches!(
            registry.live_mut(&owner("wrong"), &id("a")),
            Err(TerminalRegistryError::NotFound)
        ));
        let first = registry
            .list(&owner("one"), None, 1, &TerminalRegistryFilter::default())
            .unwrap();
        assert_eq!(first[0].session_id, id("a"));
        let next = registry
            .list(
                &owner("one"),
                Some(&first[0].session_id),
                1,
                &TerminalRegistryFilter::default(),
            )
            .unwrap();
        assert_eq!(next[0].session_id, id("b"));
        registry
            .live_mut(&owner("one"), &id("b"))
            .unwrap()
            .shell_ready(1)
            .unwrap();
        let filter = TerminalRegistryFilter {
            lifecycle: Some(TerminalLifecycle::Running),
            backend: Some(TerminalBackend::Native),
        };
        let matches = registry.list(&owner("one"), None, 256, &filter).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].session_id, id("b"));
        assert!(matches!(
            registry.list(&owner("one"), None, 0, &filter),
            Err(TerminalRegistryError::Invalid)
        ));
    }

    #[test]
    fn admission_rejects_before_factory_and_release_never_deletes_history() {
        let fixtures: Vec<_> = (0..MAX_RESIDENT_TERMINALS)
            .map(|_| Fixture::new())
            .collect();
        let mut registry = registry();
        let owner = owner("one");
        for (number, fixture) in fixtures.iter().enumerate() {
            let id = id(&format!("t-{number}"));
            registry
                .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
                .unwrap();
        }
        assert_eq!(
            registry.start(owner.clone(), id("t-0"), || panic!("duplicate factory")),
            Err(TerminalRegistryError::Conflict)
        );
        assert_eq!(
            registry.start(owner.clone(), id("overflow"), || panic!("capacity factory")),
            Err(TerminalRegistryError::Capacity)
        );
        registry
            .live_mut(&owner, &id("t-0"))
            .unwrap()
            .close(&owner, TerminalClosePolicy::Force, 0)
            .unwrap();
        registry.release(&owner, &id("t-0")).unwrap();
        assert!(fixtures[0].path.join("tj-meta").is_file());
        registry.pump(10, 1).unwrap();
        registry
            .recover(owner.clone(), id("t-0"), || {
                fixtures[0].recovered(&owner, &id("t-0"), 10)
            })
            .unwrap();
        assert!(matches!(
            registry.live_mut(&owner, &id("t-0")),
            Err(TerminalRegistryError::Closed)
        ));
        assert_eq!(
            registry.inspect(&owner, &id("t-0")).unwrap().context.now_ms,
            0
        );
        registry.release(&owner, &id("t-0")).unwrap();
    }

    #[test]
    fn round_robin_continues_after_error_and_clock_rejection_is_effect_free() {
        let fixtures: Vec<_> = (0..3).map(|_| Fixture::new()).collect();
        let mut registry = registry();
        let owner = owner("one");
        for (number, fixture) in fixtures.iter().enumerate() {
            let id = id(&format!("t-{number}"));
            registry
                .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
                .unwrap();
        }
        fixtures[0].state.lock().unwrap().read_fails = true;
        assert!(registry.pump(1, 1).unwrap()[0].result.is_err());
        assert_eq!(
            registry.release(&owner, &id("t-0")),
            Err(TerminalRegistryError::Busy)
        );
        assert_eq!(registry.pump(2, 1).unwrap()[0].session_id, id("t-1"));
        assert_eq!(registry.pump(3, 1).unwrap()[0].session_id, id("t-2"));
        registry
            .live_mut(&owner, &id("t-1"))
            .unwrap()
            .shell_ready(10)
            .unwrap();
        assert!(matches!(
            registry.pump(9, 3),
            Err(TerminalRegistryError::Clock)
        ));
        assert!(matches!(
            registry.pump(10, 0),
            Err(TerminalRegistryError::Invalid)
        ));
        for fixture in &fixtures {
            assert_eq!(fixture.state.lock().unwrap().reads, 1);
        }
        assert_eq!(registry.pump(10, 1).unwrap()[0].session_id, id("t-1"));
    }

    #[test]
    fn failed_final_publication_survives_shutdown_and_requires_explicit_transfer() {
        for direct_close in [false, true] {
            let fixture = Fixture::new();
            let mut registry = registry();
            let other_owner = owner("other");
            let owner = owner("one");
            let id = id("failed-publication");
            registry
                .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
                .unwrap();
            assert!(matches!(
                registry.take_failed_history(&owner, &id),
                Err(TerminalRegistryError::Busy)
            ));
            let temporary = rustix::fs::openat(
                fixture.fd(),
                "tj-meta.tmp",
                OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::from_bits_retain(0o600),
            )
            .unwrap();
            if direct_close {
                assert!(
                    registry
                        .live_mut(&owner, &id)
                        .unwrap()
                        .close(&owner, TerminalClosePolicy::Force, 1)
                        .is_err()
                );
            }
            for now_ms in [2, 3] {
                let failures = registry
                    .shutdown(now_ms, TerminalClosePolicy::Force)
                    .unwrap();
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0].session_id, id);
                assert_eq!(failures[0].owner, owner);
                assert!(matches!(
                    failures[0].error,
                    TerminalSessionError::History(_)
                ));
                assert_eq!(fixture.state.lock().unwrap().closes, 1);
                assert!(matches!(
                    registry.release(&owner, &id),
                    Err(TerminalRegistryError::Session(
                        TerminalSessionError::History(_)
                    ))
                ));
                let facts = registry.inspect(&owner, &id).unwrap();
                assert_eq!(facts.context.lifecycle, TerminalLifecycle::Closed);
                assert!(facts.outcome.is_some());
            }
            assert!(matches!(
                registry.take_failed_history(&other_owner, &id),
                Err(TerminalRegistryError::NotFound)
            ));
            let failed = registry.take_failed_history(&owner, &id).unwrap();
            assert!(!failed.owns_backend());
            assert!(failed.publication_error().is_some());
            assert!(failed.inspect(&owner).unwrap().outcome.is_some());
            assert!(matches!(
                TerminalJournal::open_existing(fixture.fd(), &id, TerminalJournalLimits::default()),
                Err(TerminalJournalError::Busy)
            ));
            assert!(matches!(
                registry.inspect(&owner, &id),
                Err(TerminalRegistryError::NotFound)
            ));
            drop(registry);
            drop(failed);
            drop(temporary);
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
        }
    }

    #[test]
    fn shutdown_attempts_all_owned_sessions_and_retains_failed_cleanup() {
        let fixtures: Vec<_> = (0..2).map(|_| Fixture::new()).collect();
        let mut registry = registry();
        let owner = owner("one");
        for (number, fixture) in fixtures.iter().enumerate() {
            let id = id(&format!("t-{number}"));
            registry
                .start(owner.clone(), id.clone(), || fixture.live(&owner, &id, 0))
                .unwrap();
        }
        fixtures[0].state.lock().unwrap().close_fails = true;
        let failures = registry.shutdown(1, TerminalClosePolicy::Graceful).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].session_id, id("t-0"));
        assert_eq!(failures[0].owner, owner);
        assert_eq!(failures[0].error, TerminalSessionError::Native);
        assert_eq!(
            registry.release(&owner, &id("t-0")),
            Err(TerminalRegistryError::Busy)
        );
        assert_eq!(
            registry.start(owner.clone(), id("new"), || panic!("closed factory")),
            Err(TerminalRegistryError::Closed)
        );
        assert!(matches!(
            registry.live_mut(&owner, &id("t-1")),
            Err(TerminalRegistryError::Closed)
        ));
        assert!(registry.screen(&owner, &id("t-1")).is_ok());
        fixtures[0].state.lock().unwrap().close_fails = false;
        assert!(
            registry
                .shutdown(2, TerminalClosePolicy::Force)
                .unwrap()
                .is_empty()
        );
        drop(registry);
        assert_eq!(fixtures[0].state.lock().unwrap().closes, 2);
        assert_eq!(fixtures[1].state.lock().unwrap().closes, 1);
    }
}
