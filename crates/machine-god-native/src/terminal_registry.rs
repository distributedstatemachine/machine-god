//! Bounded resident ownership for the native terminal host's blocking owner loop.
//! Tool calls borrow sessions; only this owner releases native resources. The
//! disk catalog is separate, so releasing inactive residency never deletes history.

use crate::terminal_catalog::{canonical_workspace, owner_name};
use crate::terminal_history::{TerminalHistory, TerminalHistoryError, TerminalHistoryEviction};
use crate::terminal_journal::{
    TerminalJournal, TerminalJournalError, TerminalJournalMutation, TerminalJournalPage,
    TerminalJournalPhysicalUsage, TerminalJournalReceipt,
};
use crate::terminal_monitor::{TerminalMonitorContext, TerminalProcessOutcome};
use crate::terminal_profile::{
    TerminalJournalPersistence, TerminalProfileBudget, TerminalProfileError,
    TerminalProfileMutationContext,
};
use crate::terminal_profile_store::{TerminalProfileStore, TerminalProfileTransaction};
use crate::terminal_session::{
    TerminalRecoveredSession, TerminalSession, TerminalSessionBackend, TerminalSessionError,
    TerminalSessionStep,
};
use crate::terminal_session_record::TerminalSessionFacts;
use machine_god_core::{
    BackgroundOutputOwner, TerminalActorRole, TerminalAttentionState, TerminalBackend,
    TerminalClosePolicy, TerminalCursor, TerminalEventQuery, TerminalLifecycle,
    TerminalMonitorEvent, TerminalScreen, TerminalSessionId,
};
use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

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

/// A process-local admission token, not native process or persisted authority.
/// Dropping it has no registry effect: the preparation job must withdraw through
/// the owner on cancellation, or let host shutdown invalidate all pending work.
pub(crate) struct TerminalStartReservation {
    registry: Arc<()>,
    serial: NonZeroU64,
    owner: BackgroundOutputOwner,
    id: TerminalSessionId,
}
impl TerminalStartReservation {
    pub(crate) fn owner(&self) -> &BackgroundOutputOwner {
        &self.owner
    }

    pub(crate) fn session_id(&self) -> &TerminalSessionId {
        &self.id
    }
}
impl fmt::Debug for TerminalStartReservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalStartReservation")
            .finish_non_exhaustive()
    }
}

struct PendingStart {
    serial: NonZeroU64,
    owner: BackgroundOutputOwner,
    id: TerminalSessionId,
}

pub(crate) struct TerminalRegistry<B: TerminalSessionBackend> {
    workspace: String,
    entries: Vec<Entry<B>>,
    identity: Arc<()>,
    pending_starts: Vec<PendingStart>,
    next_start_serial: Option<NonZeroU64>,
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
            identity: Arc::new(()),
            pending_starts: Vec::with_capacity(MAX_RESIDENT_TERMINALS),
            next_start_serial: NonZeroU64::new(1),
            next: 0,
            now_ms: 0,
            closing: false,
        })
    }

    /// Reserves capacity before slow off-owner preparation without admitting a
    /// resident or invoking a native factory. Tokens remain exact to this registry
    /// even if it moves, and cannot become valid in a later registry allocation.
    pub(crate) fn reserve_start(
        &mut self,
        owner: BackgroundOutputOwner,
        id: TerminalSessionId,
    ) -> Result<TerminalStartReservation> {
        self.admit(&owner, &id)?;
        let serial = self
            .next_start_serial
            .ok_or(TerminalRegistryError::Capacity)?;
        self.pending_starts.push(PendingStart {
            serial,
            owner: owner.clone(),
            id: id.clone(),
        });
        self.next_start_serial = serial.get().checked_add(1).and_then(NonZeroU64::new);
        Ok(TerminalStartReservation {
            registry: Arc::clone(&self.identity),
            serial,
            owner,
            id,
        })
    }

    /// Borrows the token so a foreign-registry rejection cannot consume the
    /// caller's ability to withdraw from its correct owner. No factory runs on
    /// stale, foreign, cancelled or shutdown-invalidated admission. Once accepted,
    /// the token is consumed before the factory, including failure and unwind.
    pub(crate) fn commit_reserved_start(
        &mut self,
        reservation: &TerminalStartReservation,
        create: impl FnOnce() -> std::result::Result<TerminalSession<B>, TerminalSessionError>,
    ) -> Result<()> {
        let index = self.reservation_index(reservation)?;
        let pending = self.pending_starts.remove(index);
        let session = create()?;
        let facts = session.inspect(&pending.owner)?;
        self.validate_facts(&facts, &pending.id, true)?;
        self.entries.push(Entry {
            id: pending.id,
            owner: pending.owner,
            resident: Resident::Live(Box::new(session)),
        });
        Ok(())
    }

    /// Releases uncommitted admission only. Cleanup of any already-prepared
    /// backend remains the preparation job's responsibility on its owned worker.
    pub(crate) fn withdraw_reserved_start(
        &mut self,
        reservation: &TerminalStartReservation,
    ) -> Result<()> {
        let index = self.reservation_index(reservation)?;
        self.pending_starts.remove(index);
        Ok(())
    }

    fn reservation_index(&self, reservation: &TerminalStartReservation) -> Result<usize> {
        if !Arc::ptr_eq(&self.identity, &reservation.registry) {
            return Err(TerminalRegistryError::Invalid);
        }
        if self.closing {
            return Err(TerminalRegistryError::Closed);
        }
        self.pending_starts
            .iter()
            .position(|pending| {
                pending.serial == reservation.serial
                    && pending.owner == reservation.owner
                    && pending.id == reservation.id
            })
            .ok_or(TerminalRegistryError::NotFound)
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
            || self
                .pending_starts
                .iter()
                .any(|pending| &pending.owner == owner && &pending.id == id)
        {
            return Err(TerminalRegistryError::Conflict);
        }
        if self.entries.len() + self.pending_starts.len() >= MAX_RESIDENT_TERMINALS {
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

    pub(crate) fn workspace(&self) -> &str {
        &self.workspace
    }

    /// Identity-only admission check; does not clone retained command metadata.
    pub(crate) fn authorize_resident(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<()> {
        self.index(owner, id).map(|_| ())
    }

    /// Full inspect and acknowledgement projection for either resident kind.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit projection and owner authority"
    )]
    pub(crate) fn inspect_result_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        actor: TerminalActorRole,
        query: &TerminalEventQuery,
        controls: &machine_god_core::TerminalAllowedControls,
    ) -> Result<machine_god_core::TerminalActionResult> {
        let index = self.index(owner, id)?;
        match &mut self.entries[index].resident {
            Resident::Live(session) => {
                Ok(session.inspect_result_with(persistence, owner, actor, query, controls)?)
            }
            Resident::Recovered(session) => {
                Ok(session.inspect_result_with(persistence, owner, actor, query, controls)?)
            }
        }
    }

    /// Final input receipts remain observable after registry admission closes.
    /// Recovery never fabricates an input receipt or regains writer authority.
    pub(crate) fn write_receipt(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        actor: TerminalActorRole,
        writer: crate::terminal_input::TerminalWriterId,
        operation: std::num::NonZeroU64,
    ) -> Result<crate::terminal_input::TerminalInputReceipt> {
        match &self.entries[self.index(owner, id)?].resident {
            Resident::Live(session) => {
                Ok(session.write_receipt_with_actor(owner, actor, writer, operation)?)
            }
            Resident::Recovered(_) => Err(TerminalRegistryError::Closed),
        }
    }

    /// Identity-only catalog overlay; do not clone bounded but large commands.
    pub(crate) fn owner_ids(&self, owner: &BackgroundOutputOwner) -> Vec<TerminalSessionId> {
        self.entries
            .iter()
            .filter(|entry| &entry.owner == owner)
            .map(|entry| entry.id.clone())
            .collect()
    }

    pub(crate) fn project_facts_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        actor: TerminalActorRole,
        controls: &machine_god_core::TerminalAllowedControls,
    ) -> Result<machine_god_core::TerminalSessionFacts> {
        let index = self.index(owner, id)?;
        match &mut self.entries[index].resident {
            Resident::Live(session) => {
                session.prepare_public_facts_with(persistence, owner)?;
                Ok(session.public_facts(owner, actor, controls)?)
            }
            Resident::Recovered(session) => {
                session.prepare_public_facts_with(persistence, owner)?;
                Ok(session.public_facts(owner, actor, controls)?)
            }
        }
    }

    /// Small observation snapshot for owner-side waits. Does not clone command,
    /// shell, workspace or monitor payloads on each scheduler tick.
    pub(crate) fn wait_observation(
        &self,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
    ) -> Result<(TerminalMonitorContext, i64, Option<TerminalProcessOutcome>)> {
        Ok(match &self.entries[self.index(owner, id)?].resident {
            Resident::Live(session) => (
                session.context(),
                session.last_output_ms(),
                session.outcome(),
            ),
            Resident::Recovered(session) => {
                let facts = session.facts(owner)?;
                if facts.observation_gap.is_some() {
                    return Err(TerminalSessionError::InvalidState.into());
                }
                (facts.context.clone(), facts.last_output_ms, facts.outcome)
            }
        })
    }

    /// Executes one exact-owner mutation under this registry's own namespace.
    /// Neither persistence guards nor session borrows can escape the callback.
    pub(crate) fn mutate_with_profile<T>(
        &mut self,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        operation: impl FnOnce(
            &mut TerminalSession<B>,
            &mut dyn TerminalJournalPersistence,
        ) -> std::result::Result<T, TerminalSessionError>,
    ) -> Result<T> {
        // Authorize before acquiring profile authority or executing callbacks.
        let index = self.index(owner, id)?;
        if !matches!(self.entries[index].resident, Resident::Live(_)) {
            return Err(TerminalRegistryError::Closed);
        }
        let namespace = owner_name(&self.workspace, owner);
        let mut transaction = store
            .transaction()
            .map_err(|error| profile_error(error.into()))?;
        let mut persistence =
            TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
        let Resident::Live(session) = &mut self.entries[index].resident else {
            unreachable!("resident validated before profile admission")
        };
        Ok(operation(session, &mut persistence)?)
    }

    /// Finishes only the matching actor's attention. The transaction is gone
    /// when this returns, before the owner may publish an asynchronous reply.
    #[allow(
        clippy::too_many_arguments,
        reason = "explicit profile and actor authority"
    )]
    pub(crate) fn finish_attention_with(
        &mut self,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        actor: TerminalActorRole,
        writer: crate::terminal_input::TerminalWriterId,
        now_ms: i64,
        cancelled: bool,
    ) -> Result<TerminalAttentionState> {
        self.check_time(now_ms)?;
        let index = self.index(owner, id)?;
        if let Resident::Recovered(session) = &self.entries[index].resident {
            if let Some(error) = session.publication_error() {
                return Err(error.into());
            }
            // Successful recovery durably cleared all former actor authority.
            let attention = session.facts(owner)?.attention.clone();
            if attention != TerminalAttentionState::default() {
                return Err(TerminalRegistryError::Invalid);
            }
            return Ok(attention);
        }
        self.mutate_with_profile(store, budget, owner, id, |session, persistence| {
            session.finish_attention_with(persistence, owner, actor, writer, now_ms, cancelled)
        })
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
        self.pump_dispatch(now_ms, maximum, |entries, index| {
            let Resident::Live(session) = &mut entries[index].resident else {
                unreachable!("active entry is live");
            };
            session.pump(now_ms).map(|step| (step, None))
        })
    }

    /// Each running session reserves bounded headroom before any native read.
    /// Pre-read profile refusals are typed per-session results, not observations:
    /// they neither consume output nor quiesce the session. Fairness advances
    /// even on refusal. Bounded victim mutations share the active admission's
    /// transaction; no transaction survives a scheduler turn or host callback.
    pub(crate) fn pump_with_profile(
        &mut self,
        store: &TerminalProfileStore,
        budget: &TerminalProfileBudget,
        now_ms: i64,
        maximum: usize,
    ) -> Result<Vec<TerminalRegistryStep>> {
        let workspace = self.workspace.clone();
        self.pump_dispatch(now_ms, maximum, |entries, index| {
            let namespace = owner_name(&workspace, &entries[index].owner);
            let Resident::Live(session) = &mut entries[index].resident else {
                unreachable!("active entry is live");
            };
            let Ok(cleanup) = session.needs_native_cleanup() else {
                // Native authority loss must take the driver's failed
                // observation path, not leave a Running session parked as
                // though only pre-read capacity were temporarily absent.
                return match store.transaction() {
                    Ok(mut transaction) => {
                        let mut context = TerminalProfileMutationContext::new(
                            &mut transaction,
                            *budget,
                            &namespace,
                        );
                        session.native_status_failed_with(Some(&mut context), now_ms)
                    }
                    Err(_) => session.native_status_failed_with(None, now_ms),
                }
                .map(|step| (step, None));
            };
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
            session.preflight_profile_read(&mut transaction, budget, &namespace)?;
            let growth = session.required_profile_read_growth()?;
            reclaim_profile_capacity(entries, &workspace, index, &mut transaction, budget, growth)?;
            let Resident::Live(session) = &mut entries[index].resident else {
                unreachable!("active entry is live");
            };
            {
                let mut context =
                    TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                session.ensure_checkpoint_reserve_with(&mut context)?;
            }
            let mut permit = session.reserve_profile_read(&mut transaction, budget, &namespace)?;
            let step = session.pump_read_with(&mut permit, now_ms);
            drop(permit);
            let mut step = step.map_err(|error| {
                let mut context =
                    TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                session.fail_pending_startup_with(&mut context, now_ms, error)
            })?;
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
                if step.output.is_empty() {
                    let mut context =
                        TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                    session.advance_startup_with(&mut context, now_ms)?;
                    step.lifecycle = session.context().lifecycle;
                }
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
            &mut [Entry<B>],
            usize,
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
            if self.entries[index].active() {
                let (result, cleanup_error) = match pump(&mut self.entries, index) {
                    Ok((step, error)) => (Ok(step), error),
                    Err(error) => (Err(error), None),
                };
                let entry = &self.entries[index];
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
        self.check_time(now_ms)?;
        // Invalidate slow preparation before recovered-publication retries too,
        // not only before the subsequent native cleanup pass.
        self.closing = true;
        self.pending_starts.clear();
        // Recovered histories have no native cleanup, but retain failed
        // durable/accounting obligations. Retry those on this same owner.
        for entry in &mut self.entries {
            if let Resident::Recovered(session) = &mut entry.resident
                && session.publication_error().is_some()
                && let Ok(mut transaction) = store.transaction()
            {
                let namespace = owner_name(&workspace, &entry.owner);
                let mut context =
                    TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                // The common shutdown pass below reports any retained error.
                let _ = session.retry_publication_with(&mut context, &entry.owner);
            }
        }
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
        self.pending_starts.clear();
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

struct RetentionCandidate {
    namespace: String,
    id: TerminalSessionId,
    created_at_ms: i64,
    // Metadata-only completed reserve retirement precedes payload eviction.
    kind: Option<TerminalHistoryEviction>,
    resident: Option<usize>,
    identity: Option<crate::terminal_journal::TerminalJournalRetentionIdentity>,
}

fn retention_classes(lifecycle: TerminalLifecycle) -> &'static [TerminalHistoryEviction] {
    use TerminalHistoryEviction::{CompletedCheckpoint, CompletedOutput, LiveCoveredOutput};
    match lifecycle {
        TerminalLifecycle::Closed | TerminalLifecycle::Exited => {
            &[CompletedOutput, CompletedCheckpoint]
        }
        TerminalLifecycle::Starting | TerminalLifecycle::Running => &[LiveCoveredOutput],
        TerminalLifecycle::Lost => &[],
    }
}

fn retention_rank(kind: Option<TerminalHistoryEviction>) -> u8 {
    match kind {
        None => 0,
        Some(TerminalHistoryEviction::CompletedOutput) => 1,
        Some(TerminalHistoryEviction::CompletedCheckpoint) => 2,
        Some(TerminalHistoryEviction::LiveCoveredOutput) => 3,
    }
}

fn retention_class_has_output(
    kind: Option<TerminalHistoryEviction>,
    usage: &TerminalJournalPhysicalUsage,
) -> bool {
    match kind {
        None => true,
        Some(
            TerminalHistoryEviction::CompletedOutput | TerminalHistoryEviction::LiveCoveredOutput,
        ) => usage.raw_bytes != 0,
        Some(TerminalHistoryEviction::CompletedCheckpoint) => usage.checkpoint_bytes != 0,
    }
}

fn resident_retention_facts<B: TerminalSessionBackend>(
    entry: &Entry<B>,
) -> std::result::Result<Option<TerminalSessionFacts>, TerminalSessionError> {
    match &entry.resident {
        Resident::Live(session) => {
            if session.publication_error().is_some() {
                return Ok(None);
            }
            let facts = session.inspect(&entry.owner)?;
            let live = matches!(
                facts.context.lifecycle,
                TerminalLifecycle::Starting | TerminalLifecycle::Running
            );
            if live != session.owns_backend() {
                return Ok(None);
            }
            Ok(Some(facts))
        }
        Resident::Recovered(session) => {
            if session.publication_error().is_some() {
                return Ok(None);
            }
            Ok(Some(session.facts(&entry.owner)?.clone()))
        }
    }
}

/// Reads and validates facts without recovery's lifecycle/state publications.
/// A stale fact cursor cannot establish completed history after a torn update.
fn retention_facts(
    journal: &TerminalJournal,
    namespace: &str,
) -> std::result::Result<Option<TerminalSessionFacts>, TerminalSessionError> {
    let Some(state) = journal.load_state().map_err(TerminalHistoryError::from)? else {
        return Ok(None);
    };
    let (facts, monitors) =
        TerminalSessionFacts::decode(&state.bytes, journal.session_id(), &state.source)?;
    if facts.metadata.is_none() {
        return Ok(None);
    }
    facts.validate_profile_binding(namespace)?;
    // Decoding only the JSON header would accept a corrupt protected record.
    drop(facts.restore_monitors(monitors)?);
    if state.source != journal.latest() {
        return Ok(None);
    }
    Ok(Some(facts))
}

/// A crash between publishing completed facts and retiring their live floor
/// leaves only accounting work. The validated completed cursor grants no
/// process authority, but does permit this admitted metadata-only release.
fn release_completed_reserve(
    journal: &mut TerminalJournal,
    transaction: &mut TerminalProfileTransaction<'_>,
    budget: &TerminalProfileBudget,
    namespace: &str,
    facts: &TerminalSessionFacts,
) -> std::result::Result<(), TerminalSessionError> {
    if journal.checkpoint_reserve_bytes() == 0
        || !matches!(
            facts.context.lifecycle,
            TerminalLifecycle::Closed | TerminalLifecycle::Exited
        )
    {
        return Ok(());
    }
    let mut context = TerminalProfileMutationContext::new(transaction, *budget, namespace);
    let completion = context
        .mutate(journal, TerminalJournalMutation::CheckpointReserve(0))
        .map_err(profile_error)?;
    let receipt = completion.operation.map_err(TerminalHistoryError::from)?;
    completion
        .accounting
        .map_err(TerminalHistoryError::Accounting)?;
    if receipt != TerminalJournalReceipt::Published {
        return Err(
            TerminalHistoryError::Accounting(TerminalProfileError::AccountingMismatch).into(),
        );
    }
    Ok(())
}

fn retention_candidates<B: TerminalSessionBackend>(
    entries: &[Entry<B>],
    workspace: &str,
    active: usize,
    transaction: &mut TerminalProfileTransaction<'_>,
) -> std::result::Result<Vec<RetentionCandidate>, TerminalSessionError> {
    let active_namespace = owner_name(workspace, &entries[active].owner);
    let inventory = transaction
        .inventory()
        .map_err(|error| profile_error(error.into()))?;
    let mut candidates = Vec::new();
    for usage in inventory.sessions {
        if usage.owner_namespace == active_namespace && usage.session_id == entries[active].id {
            continue;
        }
        let resident = entries.iter().position(|entry| {
            entry.id == usage.session_id
                && owner_name(workspace, &entry.owner) == usage.owner_namespace
        });
        let (facts, identity, reserve) = if let Some(index) = resident {
            let reserve = match &entries[index].resident {
                Resident::Live(session) => session.checkpoint_reserve_bytes(),
                Resident::Recovered(session) => session.checkpoint_reserve_bytes(),
            };
            (resident_retention_facts(&entries[index])?, None, reserve)
        } else {
            let directory = transaction
                .open_session(&usage.owner_namespace, &usage.session_id)
                .map_err(|error| profile_error(error.into()))?;
            let Some(hint) = TerminalJournal::inspect_retention_hint(
                directory,
                &usage.session_id,
                usage.usage.output_bytes,
            )
            .map_err(TerminalHistoryError::from)?
            else {
                continue;
            };
            if hint.source != hint.latest {
                continue;
            }
            let facts = TerminalSessionFacts::decode_prefix_hint(
                &hint.facts_prefix,
                hint.state_bytes,
                &usage.session_id,
                &hint.source,
            )?;
            (
                Some(facts),
                Some(hint.identity),
                hint.checkpoint_reserve_bytes,
            )
        };
        let Some(facts) = facts else { continue };
        // Legacy recovered records are readable, but cannot provide the
        // workspace binding needed for profile-wide retention authority.
        if facts.metadata.is_none() {
            continue;
        }
        facts.validate_profile_binding(&usage.owner_namespace)?;
        let completed_reserve = reserve > 0
            && matches!(
                facts.context.lifecycle,
                TerminalLifecycle::Closed | TerminalLifecycle::Exited
            );
        for kind in completed_reserve.then_some(None).into_iter().chain(
            retention_classes(facts.context.lifecycle)
                .iter()
                .copied()
                .map(Some),
        ) {
            // Physical bytes include recoverable orphan generations. Do not
            // open a history for a class that cannot reclaim any of its bytes.
            if !retention_class_has_output(kind, &usage.usage) {
                continue;
            }
            candidates.push(RetentionCandidate {
                namespace: usage.owner_namespace.clone(),
                id: usage.session_id.clone(),
                created_at_ms: facts.created_at_ms,
                kind,
                resident,
                identity: identity.clone(),
            });
        }
    }
    candidates.sort_by(|left, right| {
        (
            retention_rank(left.kind),
            left.created_at_ms,
            &left.namespace,
            left.id.as_str(),
        )
            .cmp(&(
                retention_rank(right.kind),
                right.created_at_ms,
                &right.namespace,
                right.id.as_str(),
            ))
    });
    Ok(candidates)
}

/// One bounded inventory, at most three candidates per retained history, then one
/// attempt per sorted candidate. No retry loop, promised-byte credit, or writer
/// lease bypass. The profile transaction stays held across selection, metadata-
/// first eviction and physical/reservation re-accounting.
fn reclaim_profile_capacity<B: TerminalSessionBackend>(
    entries: &mut [Entry<B>],
    workspace: &str,
    active: usize,
    transaction: &mut TerminalProfileTransaction<'_>,
    budget: &TerminalProfileBudget,
    additional: u64,
) -> std::result::Result<(), TerminalSessionError> {
    if additional > budget.output_limit() {
        return Err(profile_error(TerminalProfileError::ResourceLimit));
    }
    let fits = |transaction: &TerminalProfileTransaction<'_>| {
        TerminalProfileBudget::output_charge(transaction).map(|charge| {
            charge
                .checked_add(additional)
                .is_some_and(|total| total <= budget.output_limit())
        })
    };
    if fits(transaction).map_err(profile_error)? {
        return Ok(());
    }
    // The active session is never a victim. Refuse an impossible admission
    // before discarding somebody else's output, even if all other histories
    // together could be reclaimed.
    let active_namespace = owner_name(workspace, &entries[active].owner);
    let directory = transaction
        .open_session(&active_namespace, &entries[active].id)
        .map_err(|error| profile_error(error.into()))?;
    let active_reserve =
        TerminalJournal::inspect_checkpoint_reserve(&directory, &entries[active].id)
            .map_err(TerminalHistoryError::from)?;
    let active_output = TerminalJournal::inspect_physical(directory)
        .map_err(TerminalHistoryError::from)?
        .output_bytes;
    if active_output
        .checked_add(active_reserve)
        .and_then(|charge| charge.checked_add(additional))
        .is_none_or(|charge| charge > budget.output_limit())
    {
        return Err(profile_error(TerminalProfileError::ResourceLimit));
    }
    let candidates = retention_candidates(entries, workspace, active, transaction)?;
    for candidate in candidates {
        if let Some(index) = candidate.resident {
            let entry = &mut entries[index];
            let mut context =
                TerminalProfileMutationContext::new(transaction, *budget, &candidate.namespace);
            match (&mut entry.resident, candidate.kind) {
                (Resident::Live(session), Some(kind)) => {
                    session.evict_with(&mut context, &entry.owner, kind)?;
                }
                (Resident::Recovered(session), Some(kind)) => {
                    session.evict_with(&mut context, &entry.owner, kind)?;
                }
                (Resident::Live(session), None) => {
                    session.release_completed_reserve_with(&mut context)?;
                }
                (Resident::Recovered(session), None) => {
                    session.retire_completed_checkpoint_reserve_with(&mut context, &entry.owner)?;
                }
            }
        } else {
            apply_nonresident_candidate(&candidate, transaction, budget, additional)?;
        }
        // In particular, removing raw bytes does not remove a live checkpoint
        // floor, and orphan/unlinked-publication failures grant no guessed credit.
        if fits(transaction).map_err(profile_error)? {
            return Ok(());
        }
    }
    Err(profile_error(TerminalProfileError::ResourceLimit))
}

fn apply_nonresident_candidate(
    candidate: &RetentionCandidate,
    transaction: &mut TerminalProfileTransaction<'_>,
    budget: &TerminalProfileBudget,
    additional: u64,
) -> std::result::Result<(), TerminalSessionError> {
    let directory = transaction
        .open_session(&candidate.namespace, &candidate.id)
        .map_err(|error| profile_error(error.into()))?;
    let mut journal = match TerminalJournal::open_for_retention(directory, &candidate.id) {
        Ok(journal) => journal,
        Err(TerminalJournalError::Busy | TerminalJournalError::NotFound) => return Ok(()),
        Err(error) => return Err(TerminalHistoryError::from(error).into()),
    };
    // Recovery may already have reclaimed orphaned payloads. Re-account before
    // discarding committed history, even if this candidate has since changed.
    if TerminalProfileBudget::output_charge(transaction)
        .map_err(profile_error)?
        .checked_add(additional)
        .is_some_and(|total| total <= budget.output_limit())
    {
        return Ok(());
    }
    let Some(identity) = &candidate.identity else {
        return Err(TerminalSessionError::InvalidState);
    };
    if !journal
        .matches_retention_identity(identity)
        .map_err(TerminalHistoryError::from)?
    {
        return Ok(());
    }
    let Some(facts) = retention_facts(&journal, &candidate.namespace)? else {
        return Ok(());
    };
    if facts.created_at_ms != candidate.created_at_ms
        || !candidate.kind.map_or(
            matches!(
                facts.context.lifecycle,
                TerminalLifecycle::Closed | TerminalLifecycle::Exited
            ),
            |kind| retention_classes(facts.context.lifecycle).contains(&kind),
        )
    {
        return Ok(());
    }
    if let Some(kind) = candidate.kind {
        let mut history = TerminalHistory::recover(journal)?;
        let mut context =
            TerminalProfileMutationContext::new(transaction, *budget, &candidate.namespace);
        history.evict_with(&mut context, kind)?;
    } else {
        release_completed_reserve(
            &mut journal,
            transaction,
            budget,
            &candidate.namespace,
            &facts,
        )?;
    }
    Ok(())
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

    #[allow(
        clippy::struct_excessive_bools,
        reason = "independent native backend fault injections"
    )]
    #[derive(Default)]
    struct State {
        output: VecDeque<Vec<u8>>,
        reads: usize,
        closes: usize,
        dropped: usize,
        read_fails: bool,
        status_fails: bool,
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
            let mut state = self.0.lock().unwrap();
            if state.status_fails {
                return Err(());
            }
            Ok(state
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
    fn full_inspect_forwarding_supports_live_and_recovered_residents() {
        let fixture = Fixture::new();
        let owner = owner("inspect");
        let id = id("inspect");
        let mut live = fixture.live(&owner, &id, 0).unwrap();
        live.shell_ready(0).unwrap();
        live.close(&owner, TerminalClosePolicy::Force, 0).unwrap();
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(live))
            .unwrap();
        let query = TerminalEventQuery {
            after_event_id: 0,
            acknowledge_event_id: None,
            max_events: 10,
        };
        let controls = machine_god_core::TerminalAllowedControls::default();
        let live = registry
            .inspect_result_with(
                &mut TerminalTestPersistence,
                &owner,
                &id,
                TerminalActorRole::Agent,
                &query,
                &controls,
            )
            .unwrap();
        live.validate().unwrap();
        registry.release(&owner, &id).unwrap();
        registry
            .recover(owner.clone(), id.clone(), || {
                fixture.recovered(&owner, &id, 0)
            })
            .unwrap();
        let recovered = registry
            .inspect_result_with(
                &mut TerminalTestPersistence,
                &owner,
                &id,
                TerminalActorRole::Agent,
                &query,
                &controls,
            )
            .unwrap();
        recovered.validate().unwrap();
        assert_eq!(live, recovered);
        let foreign = BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new("foreign").unwrap(),
        );
        assert!(matches!(
            registry.inspect_result_with(
                &mut TerminalTestPersistence,
                &foreign,
                &id,
                TerminalActorRole::Agent,
                &query,
                &controls
            ),
            Err(TerminalRegistryError::NotFound)
        ));
    }

    #[test]
    fn profile_attention_dispatch_checks_owner_and_releases_transaction_before_return() {
        use crate::terminal_input::TerminalWriterId;
        use machine_god_core::TerminalAttention;
        use std::num::NonZeroU64;
        let fixture = Fixture::new();
        let owner = owner("attention");
        let id = id("attention");
        let (store, session) = fixture.profile_live(&owner, &id);
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        let writer = TerminalWriterId::new(NonZeroU64::new(1).unwrap());
        let foreign = BackgroundOutputOwner::new(
            SessionId::new("owner").unwrap(),
            SessionIncarnationId::new("foreign").unwrap(),
        );
        let transaction = store.transaction().unwrap();
        assert_eq!(
            registry.mutate_with_profile(&store, &budget, &foreign, &id, |_, _| {
                panic!("foreign owner must not reach mutation")
            }),
            Err::<(), _>(TerminalRegistryError::NotFound)
        );
        drop(transaction);
        let attention = registry
            .mutate_with_profile(&store, &budget, &owner, &id, |session, persistence| {
                session.begin_attention_with(
                    persistence,
                    &owner,
                    TerminalActorRole::Agent,
                    writer,
                    1,
                )
            })
            .unwrap();
        assert_eq!(attention.attention(), TerminalAttention::AgentWait);
        assert!(store.transaction().is_ok());
        let attention = registry
            .finish_attention_with(
                &store,
                &budget,
                &owner,
                &id,
                TerminalActorRole::Agent,
                writer,
                2,
                false,
            )
            .unwrap();
        assert_eq!(attention, TerminalAttentionState::default());
        assert!(store.transaction().is_ok());
        assert_eq!(
            registry.wait_observation(&owner, &id).unwrap().0.cursor,
            registry.inspect(&owner, &id).unwrap().context.cursor
        );
        assert!(matches!(
            registry.wait_observation(&foreign, &id),
            Err(TerminalRegistryError::NotFound)
        ));
        registry
            .shutdown_with_profile(&store, &budget, 2, TerminalClosePolicy::Force)
            .unwrap();
    }

    #[test]
    fn recovered_wait_observation_rejects_unknown_output_observation() {
        let fixture = Fixture::new();
        let owner = owner("gap");
        let id = id("gap");
        let mut session = fixture.live(&owner, &id, 0).unwrap();
        session
            .close(&owner, TerminalClosePolicy::Force, 1)
            .unwrap();
        drop(session);
        let mut journal =
            TerminalJournal::open_existing(fixture.fd(), &id, TerminalJournalLimits::default())
                .unwrap();
        journal.append(b"unobserved").unwrap();
        drop(journal);
        let mut registry = registry();
        registry
            .recover(owner.clone(), id.clone(), || {
                fixture.recovered(&owner, &id, 2)
            })
            .unwrap();
        assert!(
            registry
                .inspect(&owner, &id)
                .unwrap()
                .observation_gap
                .is_some()
        );
        assert!(matches!(
            registry.wait_observation(&owner, &id),
            Err(TerminalRegistryError::Session(
                TerminalSessionError::InvalidState
            ))
        ));
    }

    #[test]
    fn native_status_failure_is_lost_even_when_profile_publication_is_unavailable() {
        for unavailable in 0..3 {
            let fixture = Fixture::new();
            let owner = owner("status-failure");
            let id = id("status-failure");
            let (store, session) = fixture.profile_live(&owner, &id);
            let mut registry = registry();
            registry
                .start(owner.clone(), id.clone(), || Ok(session))
                .unwrap();
            fixture.state.lock().unwrap().status_fails = true;
            let mut limits = TerminalProfileLimits::default();
            if unavailable == 2 {
                limits.retained.output_bytes = 1;
            }
            let budget = TerminalProfileBudget::new(limits).unwrap();
            let held = (unavailable == 1).then(|| store.transaction().unwrap());
            let steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
            assert!(matches!(steps[0].result, Err(TerminalSessionError::Native)));
            assert_eq!(
                registry.inspect(&owner, &id).unwrap().context.lifecycle,
                TerminalLifecycle::Lost
            );
            let session = registry.live_mut(&owner, &id).unwrap();
            assert!(session.owns_backend());
            assert_eq!(session.publication_error().is_some(), unavailable != 0);
            assert_eq!(fixture.state.lock().unwrap().reads, 0);
            assert_eq!(fixture.state.lock().unwrap().closes, 0);
            assert!(
                registry
                    .pump_with_profile(&store, &budget, 2, 1)
                    .unwrap()
                    .is_empty()
            );
            // Failure quiesces observation but does not abandon native cleanup.
            drop(held);
            let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            assert!(
                registry
                    .shutdown_with_profile(&store, &normal, 2, TerminalClosePolicy::Force)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
        }
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

    fn retained_history(
        store: &TerminalProfileStore,
        owner: &BackgroundOutputOwner,
        id: &TerminalSessionId,
        lifecycle: TerminalLifecycle,
        created_at_ms: i64,
        raw_bytes: usize,
    ) -> String {
        let mut transaction = store.transaction().unwrap();
        let mut catalog = transaction
            .prepare_catalog("/workspace".into(), owner.clone())
            .unwrap();
        let directory = transaction.create_session(&mut catalog, id).unwrap();
        let namespace = catalog.namespace_key().to_owned();
        let journal = TerminalJournal::create(
            directory,
            id.clone(),
            TerminalJournalLimits {
                segment_bytes: 128 * 1024,
                session_bytes: 16 * 1024 * 1024,
            },
        )
        .unwrap();
        let mut history =
            TerminalHistory::create(journal, &TerminalDimensions::new(3, 20).unwrap()).unwrap();
        for bytes in vec![b'x'; raw_bytes].chunks(16 * 1024) {
            history.append(bytes).unwrap();
        }
        history.checkpoint().unwrap();
        let context = crate::terminal_monitor::TerminalMonitorContext {
            now_ms: created_at_ms,
            cursor: history.latest(),
            lifecycle,
        };
        let mut facts = TerminalSessionFacts::new(
            id.clone(),
            owner,
            context.clone(),
            created_at_ms,
            created_at_ms,
            None,
        )
        .unwrap();
        facts.metadata = Some(test_metadata());
        let monitors =
            crate::terminal_monitor::TerminalMonitorSet::new(id.clone(), context).unwrap();
        history
            .publish_state(&facts.encode(&monitors).unwrap())
            .unwrap();
        if lifecycle == TerminalLifecycle::Closed {
            history
                .release_checkpoint_reserve_with(&mut TerminalTestPersistence)
                .unwrap();
        }
        namespace
    }

    fn retained_usage(
        store: &TerminalProfileStore,
        namespace: &str,
        id: &TerminalSessionId,
    ) -> TerminalJournalPhysicalUsage {
        store
            .transaction()
            .unwrap()
            .inventory()
            .unwrap()
            .sessions
            .into_iter()
            .find(|session| session.owner_namespace == namespace && session.session_id == *id)
            .unwrap()
            .usage
    }

    fn capacity_budget(output_bytes: u64) -> TerminalProfileBudget {
        let mut limits = TerminalProfileLimits::default();
        limits.retained.output_bytes = output_bytes;
        TerminalProfileBudget::new(limits).unwrap()
    }

    #[test]
    fn profile_retention_orders_classes_then_age_and_preserves_active_full_key() {
        let fixture = Fixture::new();
        let active_owner = owner("active");
        let shared_id = id("same-id");
        let (store, session) = fixture.profile_live(&active_owner, &shared_id);
        let mut registry = registry();
        registry
            .start(active_owner.clone(), shared_id.clone(), || Ok(session))
            .unwrap();
        // The oldest history has only a checkpoint. Completed raw always takes
        // priority, even when it is newer and has the active session's short id.
        let checkpoint = retained_history(
            &store,
            &owner("checkpoint"),
            &id("checkpoint"),
            TerminalLifecycle::Closed,
            1,
            0,
        );
        let newer = retained_history(
            &store,
            &owner("newer"),
            &id("newer"),
            TerminalLifecycle::Closed,
            3,
            64,
        );
        let older = retained_history(
            &store,
            &owner("older"),
            &shared_id,
            TerminalLifecycle::Closed,
            2,
            64,
        );
        let checkpoint_before = retained_usage(&store, &checkpoint, &id("checkpoint"));
        let active_before = registry.physical_usage(&active_owner, &shared_id).unwrap();
        let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let mut transaction = store.transaction().unwrap();
        let budget = capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
        reclaim_profile_capacity(
            &mut registry.entries,
            "/workspace",
            0,
            &mut transaction,
            &budget,
            32,
        )
        .unwrap();
        drop(transaction);
        assert_eq!(retained_usage(&store, &older, &shared_id).raw_bytes, 0);
        assert_eq!(retained_usage(&store, &newer, &id("newer")).raw_bytes, 64);
        assert_eq!(
            retained_usage(&store, &checkpoint, &id("checkpoint")),
            checkpoint_before
        );
        assert_eq!(
            registry.physical_usage(&active_owner, &shared_id).unwrap(),
            active_before
        );
        registry
            .shutdown_with_profile(&store, &normal, 3, TerminalClosePolicy::Force)
            .unwrap();
    }

    #[test]
    fn profile_pump_reclaims_nonresident_completed_output_before_native_read() {
        let fixture = Fixture::new();
        let active_owner = owner("active");
        let active_id = id("active");
        let (store, session) = fixture.profile_live(&active_owner, &active_id);
        let mut registry = registry();
        registry
            .start(active_owner.clone(), active_id.clone(), || Ok(session))
            .unwrap();
        let victim_id = id("victim");
        let victim = retained_history(
            &store,
            &owner("foreign"),
            &victim_id,
            TerminalLifecycle::Closed,
            0,
            32 * 1024,
        );
        let before = retained_usage(&store, &victim, &victim_id);
        let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let budget = capacity_budget(
            TerminalProfileBudget::output_charge(&store.transaction().unwrap()).unwrap(),
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"progress".to_vec());
        let mut steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
        let step = steps.remove(0).result.unwrap();
        assert_eq!(step.output, b"progress");
        assert_eq!(fixture.state.lock().unwrap().reads, 1);
        let after = retained_usage(&store, &victim, &victim_id);
        assert_eq!(after.raw_bytes, 0);
        assert_eq!(after.checkpoint_bytes, before.checkpoint_bytes);
        assert_eq!(after.state_bytes, before.state_bytes);
        registry
            .shutdown_with_profile(&store, &normal, 1, TerminalClosePolicy::Force)
            .unwrap();
    }

    #[test]
    fn profile_retention_busy_foreign_writer_defers_without_native_consumption() {
        let fixture = Fixture::new();
        let active_owner = owner("active");
        let active_id = id("active");
        let (store, session) = fixture.profile_live(&active_owner, &active_id);
        let mut registry = registry();
        registry
            .start(active_owner.clone(), active_id.clone(), || Ok(session))
            .unwrap();
        let victim_id = id("victim");
        let victim = retained_history(
            &store,
            &owner("foreign"),
            &victim_id,
            TerminalLifecycle::Closed,
            0,
            32 * 1024,
        );
        let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let transaction = store.transaction().unwrap();
        let budget = capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
        let held = TerminalJournal::open_for_retention(
            transaction.open_session(&victim, &victim_id).unwrap(),
            &victim_id,
        )
        .unwrap();
        let before = held.physical_usage().unwrap();
        drop(transaction);
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"pending".to_vec());
        let steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
        assert_eq!(
            steps[0].result.as_ref().err(),
            Some(&profile_error(TerminalProfileError::ResourceLimit))
        );
        assert_eq!(fixture.state.lock().unwrap().reads, 0);
        assert_eq!(
            registry
                .inspect(&active_owner, &active_id)
                .unwrap()
                .context
                .lifecycle,
            TerminalLifecycle::Starting
        );
        assert_eq!(held.physical_usage().unwrap(), before);
        drop(held);
        let mut steps = registry.pump_with_profile(&store, &budget, 2, 1).unwrap();
        assert_eq!(steps.remove(0).result.unwrap().output, b"pending");
        registry
            .shutdown_with_profile(&store, &normal, 2, TerminalClosePolicy::Force)
            .unwrap();
    }

    #[test]
    fn profile_retention_uses_validated_live_covered_prefix_after_completed_classes() {
        let fixture = Fixture::new();
        let active_owner = owner("active");
        let active_id = id("active");
        let (store, session) = fixture.profile_live(&active_owner, &active_id);
        let mut registry = registry();
        registry
            .start(active_owner, active_id, || Ok(session))
            .unwrap();
        let live_id = id("live-foreign");
        let live = retained_history(
            &store,
            &owner("live-foreign"),
            &live_id,
            TerminalLifecycle::Running,
            0,
            256 * 1024 + 1,
        );
        let completed_id = id("completed");
        let completed = retained_history(
            &store,
            &owner("completed"),
            &completed_id,
            TerminalLifecycle::Closed,
            1,
            0,
        );
        let live_before = retained_usage(&store, &live, &live_id);
        let completed_before = retained_usage(&store, &completed, &completed_id);
        let mut transaction = store.transaction().unwrap();
        let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let budget = capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
        // Raw is absent in the completed history, so its checkpoint is selected
        // before the older live history loses any covered raw prefix.
        reclaim_profile_capacity(
            &mut registry.entries,
            "/workspace",
            0,
            &mut transaction,
            &budget,
            1,
        )
        .unwrap();
        drop(transaction);
        assert!(completed_before.checkpoint_bytes > 0);
        assert_eq!(
            retained_usage(&store, &completed, &completed_id).checkpoint_bytes,
            0
        );
        assert_eq!(retained_usage(&store, &live, &live_id), live_before);
        let mut transaction = store.transaction().unwrap();
        let budget = capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
        reclaim_profile_capacity(
            &mut registry.entries,
            "/workspace",
            0,
            &mut transaction,
            &budget,
            1,
        )
        .unwrap();
        drop(transaction);
        let after = retained_usage(&store, &live, &live_id);
        assert_eq!(after.raw_bytes, 1);
        assert_eq!(after.checkpoint_bytes, live_before.checkpoint_bytes);
        assert_eq!(after.state_bytes, live_before.state_bytes);
        let transaction = store.transaction().unwrap();
        let journal = TerminalJournal::open_for_retention(
            transaction.open_session(&live, &live_id).unwrap(),
            &live_id,
        )
        .unwrap();
        let facts = retention_facts(&journal, &live).unwrap().unwrap();
        assert_eq!(facts.context.lifecycle, TerminalLifecycle::Running);
        assert!(TerminalHistory::recover(journal).unwrap().screen().is_ok());
        drop(transaction);
        registry
            .shutdown_with_profile(&store, &normal, 1, TerminalClosePolicy::Force)
            .unwrap();
    }

    #[test]
    fn profile_retention_corrupt_records_or_live_projection_fail_before_read() {
        for projection in [false, true] {
            let fixture = Fixture::new();
            let active_owner = owner("active");
            let active_id = id("active");
            let (store, session) = fixture.profile_live(&active_owner, &active_id);
            let mut registry = registry();
            registry
                .start(active_owner, active_id, || Ok(session))
                .unwrap();
            let victim_id = id("victim");
            let victim = retained_history(
                &store,
                &owner("foreign"),
                &victim_id,
                TerminalLifecycle::Running,
                0,
                256 * 1024 + 1,
            );
            let transaction = store.transaction().unwrap();
            let mut journal = TerminalJournal::open_for_retention(
                transaction.open_session(&victim, &victim_id).unwrap(),
                &victim_id,
            )
            .unwrap();
            if projection {
                journal
                    .publish_checkpoint(journal.latest(), b"invalid checkpoint")
                    .unwrap();
            } else {
                journal
                    .publish_state(journal.latest(), b"invalid facts")
                    .unwrap();
            }
            let before = journal.physical_usage().unwrap();
            drop(journal);
            let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            let budget =
                capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
            drop(transaction);
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"pending".to_vec());
            let steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
            assert!(steps[0].result.is_err());
            assert_eq!(fixture.state.lock().unwrap().reads, 0);
            assert_eq!(retained_usage(&store, &victim, &victim_id), before);
            registry
                .shutdown_with_profile(&store, &normal, 1, TerminalClosePolicy::Force)
                .unwrap();
        }
    }

    #[test]
    fn profile_retention_uses_resident_writer_without_reopening_or_stale_cache() {
        let fixture = Fixture::new();
        let active_owner = owner("active");
        let active_id = id("active");
        let (store, session) = fixture.profile_live(&active_owner, &active_id);
        let mut registry = registry();
        registry
            .start(active_owner, active_id, || Ok(session))
            .unwrap();
        let victim_owner = owner("resident");
        let victim_id = id("resident");
        let (_, mut victim) = fixture.profile_live(&victim_owner, &victim_id);
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(vec![b'x'; 16 * 1024]);
        victim.pump(0).unwrap();
        victim
            .close(&victim_owner, TerminalClosePolicy::Force, 1)
            .unwrap();
        registry
            .start(victim_owner.clone(), victim_id.clone(), || Ok(victim))
            .unwrap();
        let before = registry.physical_usage(&victim_owner, &victim_id).unwrap();
        assert_eq!(before.raw_bytes, 16 * 1024);
        let mut transaction = store.transaction().unwrap();
        let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let budget = capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
        reclaim_profile_capacity(
            &mut registry.entries,
            "/workspace",
            0,
            &mut transaction,
            &budget,
            1,
        )
        .unwrap();
        drop(transaction);
        let after = registry.physical_usage(&victim_owner, &victim_id).unwrap();
        assert_eq!(after.raw_bytes, 0);
        assert_eq!(after.state_bytes, before.state_bytes);
        let page = registry
            .read(
                &victim_owner,
                &victim_id,
                &TerminalCursor::new(1, 0).unwrap(),
                32,
            )
            .unwrap();
        assert!(page.bytes.is_empty());
        assert!(page.gap.is_some());
        registry
            .shutdown_with_profile(&store, &normal, 1, TerminalClosePolicy::Force)
            .unwrap();
    }

    #[test]
    fn profile_retention_retires_abandoned_completed_floor_before_output_eviction() {
        for resident in [false, true] {
            let fixture = Fixture::new();
            let active_owner = owner("active");
            let active_id = id("active");
            let (store, session) = fixture.profile_live(&active_owner, &active_id);
            let mut registry = registry();
            registry
                .start(active_owner, active_id, || Ok(session))
                .unwrap();
            let victim_id = id("completed");
            let victim = retained_history(
                &store,
                &owner("foreign"),
                &victim_id,
                TerminalLifecycle::Closed,
                0,
                64,
            );
            let transaction = store.transaction().unwrap();
            let mut journal = TerminalJournal::open_for_retention(
                transaction.open_session(&victim, &victim_id).unwrap(),
                &victim_id,
            )
            .unwrap();
            journal
                .prepare_mutation(TerminalJournalMutation::CheckpointReserve(10 * 1024 * 1024))
                .unwrap()
                .execute()
                .unwrap();
            let before = journal.physical_usage().unwrap();
            let state_before = journal.load_state().unwrap().unwrap().bytes;
            if resident {
                registry
                    .recover(owner("foreign"), victim_id.clone(), || {
                        TerminalRecoveredSession::recover(
                            TerminalHistory::recover(journal)?,
                            &owner("foreign"),
                            0,
                        )
                    })
                    .unwrap();
            } else {
                drop(journal);
            }
            let budget =
                capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
            drop(transaction);
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"progress".to_vec());
            let mut steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
            assert_eq!(steps.remove(0).result.unwrap().output, b"progress");
            if resident {
                registry.release(&owner("foreign"), &victim_id).unwrap();
            }
            let transaction = store.transaction().unwrap();
            let journal = TerminalJournal::open_for_retention(
                transaction.open_session(&victim, &victim_id).unwrap(),
                &victim_id,
            )
            .unwrap();
            assert_eq!(journal.checkpoint_reserve_bytes(), 0);
            assert_eq!(journal.load_state().unwrap().unwrap().bytes, state_before);
            let after = journal.physical_usage().unwrap();
            assert_eq!(after.raw_bytes, before.raw_bytes);
            assert_eq!(after.checkpoint_bytes, before.checkpoint_bytes);
            drop(journal);
            drop(transaction);
            let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            registry
                .shutdown_with_profile(&store, &normal, 1, TerminalClosePolicy::Force)
                .unwrap();
        }
    }

    #[test]
    fn profile_retention_preserves_committed_output_when_recovery_frees_capacity() {
        let fixture = Fixture::new();
        let (store, session) = fixture.profile_live(&owner("active"), &id("active"));
        let mut registry = registry();
        registry
            .start(owner("active"), id("active"), || Ok(session))
            .unwrap();
        let namespace = retained_history(
            &store,
            &owner("completed"),
            &id("completed"),
            TerminalLifecycle::Closed,
            1,
            64,
        );
        let orphan = fixture
            .path
            .join("terminal-v1")
            .join(&namespace)
            .join("sessions/completed/tj-checkpoint-00000000000000000099");
        std::fs::write(&orphan, vec![b'x'; 32 * 1024]).unwrap();
        std::fs::set_permissions(&orphan, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .unwrap();
        let transaction = store.transaction().unwrap();
        let budget = capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
        drop(transaction);
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"progress".to_vec());
        let mut steps = registry.pump_with_profile(&store, &budget, 3, 1).unwrap();
        assert_eq!(steps.remove(0).result.unwrap().output, b"progress");
        assert_eq!(
            retained_usage(&store, &namespace, &id("completed")).raw_bytes,
            64
        );
        assert!(!orphan.exists());
        let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        registry
            .shutdown_with_profile(&store, &normal, 3, TerminalClosePolicy::Force)
            .unwrap();
    }

    #[test]
    fn profile_retention_never_recovers_unselected_payloads() {
        for (unused_raw, checkpoint_only) in [(0, false), (0, true), (64, false)] {
            let fixture = Fixture::new();
            let (store, session) = fixture.profile_live(&owner("active"), &id("active"));
            let mut registry = registry();
            registry
                .start(owner("active"), id("active"), || Ok(session))
                .unwrap();
            let useful = retained_history(
                &store,
                &owner("useful"),
                &id("useful"),
                TerminalLifecycle::Closed,
                1,
                32 * 1024,
            );
            let unused = retained_history(
                &store,
                &owner("unused"),
                &id("unused"),
                TerminalLifecycle::Closed,
                if checkpoint_only { 0 } else { 2 },
                unused_raw,
            );
            let transaction = store.transaction().unwrap();
            let mut journal = TerminalJournal::open_for_retention(
                transaction.open_session(&unused, &id("unused")).unwrap(),
                &id("unused"),
            )
            .unwrap();
            let event = journal.append_event(b"retained").unwrap();
            if !checkpoint_only {
                journal
                    .evict(&crate::terminal_journal::TerminalJournalEviction::CompletedCheckpoint)
                    .unwrap();
            }
            drop(journal);
            let event_path = fixture
                .path
                .join("terminal-v1")
                .join(&unused)
                .join("sessions")
                .join("unused")
                .join(format!("tj-event-{event:020}"));
            std::fs::write(&event_path, b"corrupt!").unwrap();
            let budget =
                capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
            drop(transaction);
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"progress".to_vec());
            let mut steps = registry.pump_with_profile(&store, &budget, 3, 1).unwrap();
            assert_eq!(steps.remove(0).result.unwrap().output, b"progress");
            assert_eq!(retained_usage(&store, &useful, &id("useful")).raw_bytes, 0);
            assert_eq!(
                retained_usage(&store, &unused, &id("unused")).raw_bytes,
                unused_raw as u64
            );
            assert_eq!(std::fs::read(event_path).unwrap(), b"corrupt!");
            let transaction = store.transaction().unwrap();
            assert!(matches!(
                TerminalJournal::open_for_retention(
                    transaction.open_session(&unused, &id("unused")).unwrap(),
                    &id("unused")
                ),
                Err(TerminalJournalError::Corrupt)
            ));
            drop(transaction);
            let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            registry
                .shutdown_with_profile(&store, &normal, 3, TerminalClosePolicy::Force)
                .unwrap();
        }
    }

    #[test]
    fn profile_retention_skips_legacy_recovered_residents() {
        for lifecycle in [TerminalLifecycle::Closed, TerminalLifecycle::Lost] {
            let fixture = Fixture::new();
            let (store, session) = fixture.profile_live(&owner("active"), &id("active"));
            let mut registry = registry();
            registry
                .start(owner("active"), id("active"), || Ok(session))
                .unwrap();
            let legacy =
                retained_history(&store, &owner("legacy"), &id("legacy"), lifecycle, 0, 64);
            let useful = retained_history(
                &store,
                &owner("useful"),
                &id("useful"),
                TerminalLifecycle::Closed,
                1,
                32 * 1024,
            );
            let transaction = store.transaction().unwrap();
            let mut journal = TerminalJournal::open_for_retention(
                transaction.open_session(&legacy, &id("legacy")).unwrap(),
                &id("legacy"),
            )
            .unwrap();
            let stored = journal.load_state().unwrap().unwrap();
            let (mut facts, monitors) =
                TerminalSessionFacts::decode(&stored.bytes, &id("legacy"), &stored.source).unwrap();
            let monitors = facts.restore_monitors(monitors).unwrap();
            facts.metadata = None;
            journal
                .publish_state(journal.latest(), &facts.encode(&monitors).unwrap())
                .unwrap();
            registry
                .recover(owner("legacy"), id("legacy"), || {
                    TerminalRecoveredSession::recover(
                        TerminalHistory::recover(journal)?,
                        &owner("legacy"),
                        1,
                    )
                })
                .unwrap();
            let before = registry
                .physical_usage(&owner("legacy"), &id("legacy"))
                .unwrap();
            let budget =
                capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
            drop(transaction);
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"progress".to_vec());
            let mut steps = registry.pump_with_profile(&store, &budget, 2, 1).unwrap();
            assert_eq!(steps.remove(0).result.unwrap().output, b"progress");
            assert_eq!(
                registry
                    .physical_usage(&owner("legacy"), &id("legacy"))
                    .unwrap(),
                before
            );
            assert_eq!(retained_usage(&store, &useful, &id("useful")).raw_bytes, 0);
            let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            registry
                .shutdown_with_profile(&store, &normal, 2, TerminalClosePolicy::Force)
                .unwrap();
        }
    }

    #[test]
    fn profile_retention_never_treats_lost_or_stale_completed_facts_as_completion() {
        for stale in [false, true] {
            let fixture = Fixture::new();
            let active_owner = owner("active");
            let active_id = id("active");
            let (store, session) = fixture.profile_live(&active_owner, &active_id);
            let mut registry = registry();
            registry
                .start(active_owner, active_id, || Ok(session))
                .unwrap();
            let victim_id = id("victim");
            let lifecycle = if stale {
                TerminalLifecycle::Closed
            } else {
                TerminalLifecycle::Lost
            };
            let victim = retained_history(
                &store,
                &owner("foreign"),
                &victim_id,
                lifecycle,
                0,
                32 * 1024,
            );
            let mut transaction = store.transaction().unwrap();
            if stale {
                let mut journal = TerminalJournal::open_for_retention(
                    transaction.open_session(&victim, &victim_id).unwrap(),
                    &victim_id,
                )
                .unwrap();
                journal.append(b"unobserved").unwrap();
            }
            let budget =
                capacity_budget(TerminalProfileBudget::output_charge(&transaction).unwrap());
            let before = transaction.inventory().unwrap().usage;
            assert_eq!(
                reclaim_profile_capacity(
                    &mut registry.entries,
                    "/workspace",
                    0,
                    &mut transaction,
                    &budget,
                    1
                ),
                Err(profile_error(TerminalProfileError::ResourceLimit))
            );
            assert_eq!(transaction.inventory().unwrap().usage, before);
            drop(transaction);
            let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            registry
                .shutdown_with_profile(&store, &normal, 1, TerminalClosePolicy::Force)
                .unwrap();
        }
    }

    #[test]
    fn profile_retention_preserves_victims_when_active_charge_cannot_fit_alone() {
        let fixture = Fixture::new();
        let active_owner = owner("active");
        let active_id = id("active");
        let (store, session) = fixture.profile_live(&active_owner, &active_id);
        let budget = capacity_budget(
            TerminalProfileBudget::output_charge(&store.transaction().unwrap()).unwrap(),
        );
        let mut registry = registry();
        registry
            .start(active_owner, active_id, || Ok(session))
            .unwrap();
        let victim_id = id("victim");
        let victim = retained_history(
            &store,
            &owner("foreign"),
            &victim_id,
            TerminalLifecycle::Closed,
            0,
            32 * 1024,
        );
        let before = retained_usage(&store, &victim, &victim_id);
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"pending".to_vec());
        let steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
        assert_eq!(
            steps[0].result.as_ref().err(),
            Some(&profile_error(TerminalProfileError::ResourceLimit))
        );
        assert_eq!(fixture.state.lock().unwrap().reads, 0);
        assert_eq!(retained_usage(&store, &victim, &victim_id), before);
        let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        registry
            .shutdown_with_profile(&store, &normal, 1, TerminalClosePolicy::Force)
            .unwrap();
    }

    #[test]
    fn profile_retention_preserves_victims_for_non_output_admission_refusals() {
        for ledger in 0..3 {
            let fixture = Fixture::new();
            let active_owner = owner("active");
            let active_id = id("active");
            let (store, session) = fixture.profile_live(&active_owner, &active_id);
            let mut registry = registry();
            registry
                .start(active_owner, active_id, || Ok(session))
                .unwrap();
            let victim_id = id("victim");
            let victim = retained_history(
                &store,
                &owner("foreign"),
                &victim_id,
                TerminalLifecycle::Closed,
                0,
                32 * 1024,
            );
            let before = retained_usage(&store, &victim, &victim_id);
            let mut limits = TerminalProfileLimits::default();
            limits.retained.output_bytes =
                TerminalProfileBudget::output_charge(&store.transaction().unwrap()).unwrap();
            match ledger {
                0 => limits.temporary_bytes = 1,
                1 => limits.retained.protected_bytes = 1,
                2 => limits.retained.metadata_bytes = 1,
                _ => unreachable!(),
            }
            let budget = TerminalProfileBudget::new(limits).unwrap();
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"pending".to_vec());
            let steps = registry.pump_with_profile(&store, &budget, 1, 1).unwrap();
            assert_eq!(
                steps[0].result.as_ref().err(),
                Some(&profile_error(TerminalProfileError::ResourceLimit))
            );
            assert_eq!(fixture.state.lock().unwrap().reads, 0);
            assert_eq!(retained_usage(&store, &victim, &victim_id), before);
            let normal = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            registry
                .shutdown_with_profile(&store, &normal, 1, TerminalClosePolicy::Force)
                .unwrap();
        }
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
        let existing = TerminalProfileBudget::output_charge(&store.transaction().unwrap()).unwrap();
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
    fn profile_owner_passes_exact_authority_for_durable_mutation() {
        use crate::terminal_owner::TerminalOwnerLoop;
        use machine_god_core::CancellationToken;
        let fixture = Fixture::new();
        let owner = owner("profile");
        let id = id("profile-mutation");
        let (store, session) = fixture.profile_live(&owner, &id);
        let store = Arc::new(store);
        let budget = Arc::new(
            TerminalProfileBudget::new(TerminalProfileLimits {
                temporary_bytes: 32 * 1024 * 1024,
                ..TerminalProfileLimits::default()
            })
            .unwrap(),
        );
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        let (worker, handle) = TerminalOwnerLoop::new();
        let expected_store = Arc::clone(&store);
        let expected_budget = Arc::clone(&budget);
        let request_owner = owner.clone();
        let request_id = id.clone();
        let stop = handle.clone();
        let mut request = handle.request_with_profile(
            CancellationToken::new(),
            move |registry, store, budget, now_ms, cancellation| {
                assert!(std::ptr::eq(store, Arc::as_ptr(&expected_store)));
                assert!(std::ptr::eq(budget, Arc::as_ptr(&expected_budget)));
                assert_eq!(now_ms, 17);
                assert!(!cancellation.is_cancelled());
                let mut transaction = store.transaction().unwrap();
                let namespace = owner_name("/workspace", &request_owner);
                let mut context =
                    TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                registry
                    .live_mut(&request_owner, &request_id)
                    .unwrap()
                    .close_with(
                        &mut context,
                        &request_owner,
                        TerminalClosePolicy::Force,
                        now_ms,
                    )
                    .unwrap();
                let facts = registry.inspect(&request_owner, &request_id).unwrap();
                assert_eq!(facts.context.lifecycle, TerminalLifecycle::Closed);
                stop.shutdown();
                facts.context.lifecycle
            },
        );
        assert!(poll_owner(&mut request).is_pending());
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        let exit = worker.run_with_profile(&mut registry, &store, &budget, || 17, |_| {});
        assert!(exit.error.is_none());
        assert!(exit.shutdown.unwrap().is_empty());
        assert_eq!(
            poll_owner(&mut request),
            std::task::Poll::Ready(Ok(TerminalLifecycle::Closed))
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert!(registry.live_mut(&owner, &id).is_err());
        drop(registry);
        let transaction = store.transaction().unwrap();
        let namespace = owner_name("/workspace", &owner);
        let journal = TerminalJournal::open_existing(
            transaction.open_session(&namespace, &id).unwrap(),
            &id,
            TerminalJournalLimits::default(),
        )
        .unwrap();
        let recovered = TerminalRecoveredSession::recover(
            TerminalHistory::recover(journal).unwrap(),
            &owner,
            17,
        )
        .unwrap();
        assert_eq!(
            recovered.facts(&owner).unwrap().context.lifecycle,
            TerminalLifecycle::Closed
        );
    }

    #[test]
    fn profile_owner_refuses_unmetered_dispatch_without_invoking_callback() {
        use crate::terminal_owner::{TerminalOwnerError, TerminalOwnerLoop};
        use machine_god_core::CancellationToken;
        let (worker, handle) = TerminalOwnerLoop::<Backend>::new();
        let mut request = handle.request_with_profile(CancellationToken::new(), |_, _, _, _, _| {
            panic!("profile-free worker must not invoke a profile mutation");
        });
        assert!(poll_owner(&mut request).is_pending());
        let stop = handle.clone();
        let mut stopping = handle.request(CancellationToken::new(), move |_, _, _| stop.shutdown());
        assert!(poll_owner(&mut stopping).is_pending());
        let exit = worker.run(&mut registry(), || 0, |_| {});
        assert!(exit.error.is_none());
        assert!(exit.shutdown.unwrap().is_empty());
        assert_eq!(
            poll_owner(&mut request),
            std::task::Poll::Ready(Err(TerminalOwnerError::ProfileRequired))
        );
        assert_eq!(poll_owner(&mut stopping), std::task::Poll::Ready(Ok(())));
    }

    #[test]
    fn profile_owner_cancellation_skips_effects_but_preserves_committed_receipt() {
        use crate::terminal_owner::{TerminalOwnerError, TerminalOwnerLoop};
        use machine_god_core::CancellationToken;
        let fixture = Fixture::new();
        let owner = owner("profile");
        let id = id("profile-cancel");
        let (store, session) = fixture.profile_live(&owner, &id);
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let mut registry = registry();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        let (worker, handle) = TerminalOwnerLoop::new();
        drop(
            handle.request_with_profile(CancellationToken::new(), |_, _, _, _, _| {
                panic!("unpolled mutation");
            }),
        );
        let precancelled = CancellationToken::new();
        precancelled.cancel();
        let mut precancelled = handle.request_with_profile(precancelled, |_, _, _, _, _| {
            panic!("cancelled before submission");
        });
        assert_eq!(
            poll_owner(&mut precancelled),
            std::task::Poll::Ready(Err(TerminalOwnerError::Cancelled))
        );
        let queued_cancel = CancellationToken::new();
        let mut queued = handle.request_with_profile(queued_cancel.clone(), |_, _, _, _, _| {
            panic!("cancelled before mutation");
        });
        assert!(poll_owner(&mut queued).is_pending());
        queued_cancel.cancel();
        let mut abandoned =
            handle.request_with_profile(CancellationToken::new(), |_, _, _, _, _| {
                panic!("abandoned queued mutation");
            });
        assert!(poll_owner(&mut abandoned).is_pending());
        drop(abandoned);
        let cancellation = CancellationToken::new();
        let cancel_after_commit = cancellation.clone();
        let stop = handle.clone();
        let state = Arc::clone(&fixture.state);
        let mut committed =
            handle.request_with_profile(cancellation, move |registry, store, budget, now_ms, _| {
                assert_eq!(state.lock().unwrap().closes, 0);
                let mut transaction = store.transaction().unwrap();
                let namespace = owner_name("/workspace", &owner);
                let mut context =
                    TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                let receipt = registry.live_mut(&owner, &id).unwrap().close_with(
                    &mut context,
                    &owner,
                    TerminalClosePolicy::Force,
                    now_ms,
                );
                assert_eq!(state.lock().unwrap().closes, 1);
                cancel_after_commit.cancel();
                stop.shutdown();
                receipt
            });
        assert!(poll_owner(&mut committed).is_pending());
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        let exit = worker.run_with_profile(&mut registry, &store, &budget, || 1, |_| {});
        assert!(exit.error.is_none());
        assert!(exit.shutdown.unwrap().is_empty());
        assert_eq!(
            poll_owner(&mut queued),
            std::task::Poll::Ready(Err(TerminalOwnerError::Cancelled))
        );
        assert_eq!(
            poll_owner(&mut committed),
            std::task::Poll::Ready(Ok(Ok(())))
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
    }

    #[test]
    fn profile_owner_reply_wake_can_reenter_profile_after_return_or_panic() {
        use crate::terminal_owner::{TerminalOwnerError, TerminalOwnerLoop};
        use machine_god_core::CancellationToken;
        use std::future::Future;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::task::{Context, Wake, Waker};
        struct ProfileWake {
            store: Arc<TerminalProfileStore>,
            state: Arc<Mutex<State>>,
            acquired: AtomicUsize,
        }
        impl Wake for ProfileWake {
            fn wake(self: Arc<Self>) {
                let transaction = self.store.transaction().unwrap();
                assert!(matches!(
                    self.store.transaction(),
                    Err(TerminalProfileStoreError::Busy)
                ));
                assert_eq!(self.state.lock().unwrap().closes, 0);
                self.acquired.fetch_add(1, Ordering::SeqCst);
                drop(transaction);
            }
        }
        for panic_in_operation in [false, true] {
            let fixture = Fixture::new();
            let owner = owner("profile");
            let id = id("profile-wake");
            let (store, session) = fixture.profile_live(&owner, &id);
            let store = Arc::new(store);
            let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
            let mut registry = registry();
            registry
                .start(owner.clone(), id.clone(), || Ok(session))
                .unwrap();
            let (worker, handle) = TerminalOwnerLoop::new();
            let stop = handle.clone();
            let mut request = handle.request_with_profile(
                CancellationToken::new(),
                move |registry, store, budget, now_ms, _| {
                    let mut transaction = store.transaction().unwrap();
                    let namespace = owner_name("/workspace", &owner);
                    let mut context =
                        TerminalProfileMutationContext::new(&mut transaction, *budget, &namespace);
                    let session = registry.live_mut(&owner, &id).unwrap();
                    session.shell_ready_with(&mut context, now_ms).unwrap();
                    session
                        .resize_with(
                            &mut context,
                            &owner,
                            &TerminalDimensions::new(4, 24).unwrap(),
                            now_ms,
                        )
                        .unwrap();
                    assert!(
                        !panic_in_operation,
                        "panic while holding profile transaction"
                    );
                    stop.shutdown();
                },
            );
            let wake = Arc::new(ProfileWake {
                store: Arc::clone(&store),
                state: Arc::clone(&fixture.state),
                acquired: AtomicUsize::new(0),
            });
            let waker = Waker::from(Arc::clone(&wake));
            assert!(
                std::pin::Pin::new(&mut request)
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            let exit = worker.run_with_profile(&mut registry, &store, &budget, || 1, |_| {});
            assert_eq!(
                exit.error,
                panic_in_operation.then_some(TerminalOwnerError::Panicked)
            );
            assert!(exit.shutdown.unwrap().is_empty());
            assert_eq!(wake.acquired.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
            assert_eq!(
                poll_owner(&mut request),
                std::task::Poll::Ready(if panic_in_operation {
                    Err(TerminalOwnerError::Panicked)
                } else {
                    Ok(())
                })
            );
            assert!(store.transaction().is_ok());
        }
    }

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

    #[test]
    fn profile_shutdown_retries_recovered_publication_after_storage_becomes_available() {
        let fixture = Fixture::new();
        let owner = owner("retry");
        let id = id("retry");
        let (store, session) = fixture.profile_live(&owner, &id);
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let mut live_registry = registry();
        live_registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"kept".to_vec());
        assert!(
            live_registry
                .pump_with_profile(&store, &budget, 1, 1)
                .unwrap()[0]
                .result
                .is_ok()
        );
        assert!(
            live_registry
                .shutdown_with_profile(&store, &budget, 2, TerminalClosePolicy::Force)
                .unwrap()
                .is_empty()
        );
        drop(live_registry);
        let mut transaction = store.transaction().unwrap();
        let namespace = owner_name("/workspace", &owner);
        let journal = TerminalJournal::open_existing(
            transaction.open_session(&namespace, &id).unwrap(),
            &id,
            TerminalJournalLimits::default(),
        )
        .unwrap();
        let mut context = TerminalProfileMutationContext::new(&mut transaction, budget, &namespace);
        let recovered = TerminalRecoveredSession::recover_with(
            &mut context,
            TerminalHistory::recover(journal).unwrap(),
            &owner,
            2,
        )
        .unwrap();
        drop(transaction);
        let mut registry = registry();
        registry
            .recover(owner.clone(), id.clone(), || Ok(recovered))
            .unwrap();
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
        let held = store.transaction().unwrap();
        assert_eq!(
            registry
                .shutdown_with_profile(&store, &budget, 2, TerminalClosePolicy::Force)
                .unwrap()
                .len(),
            1
        );
        drop(held);
        assert!(
            registry
                .shutdown_with_profile(&store, &budget, 2, TerminalClosePolicy::Force)
                .unwrap()
                .is_empty()
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert_eq!(registry.physical_usage(&owner, &id).unwrap().raw_bytes, 4);
        registry.release(&owner, &id).unwrap();
        assert!(store.transaction().is_ok());
    }

    #[test]
    fn recovered_publication_failure_requires_explicit_transfer() {
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
    fn reserved_start_is_exact_registry_owner_and_session_authority() {
        let mut first = registry();
        let mut second = registry();
        let who = owner("one");
        let token = first.reserve_start(who.clone(), id("pending")).unwrap();
        assert_eq!(token.owner(), &who);
        assert_eq!(token.session_id(), &id("pending"));
        assert_eq!(format!("{token:?}"), "TerminalStartReservation { .. }");
        assert_eq!(
            second.commit_reserved_start(&token, || panic!("foreign factory")),
            Err(TerminalRegistryError::Invalid)
        );
        assert_eq!(
            second.withdraw_reserved_start(&token),
            Err(TerminalRegistryError::Invalid)
        );
        // Fields are private to this module; forged-token tests show all three
        // identity dimensions are checked, not only a numeric sequence.
        for (owner, id) in [
            (owner("foreign"), token.id.clone()),
            (who.clone(), id("other")),
        ] {
            let forged = TerminalStartReservation {
                registry: Arc::clone(&first.identity),
                serial: token.serial,
                owner,
                id,
            };
            assert_eq!(
                first.commit_reserved_start(&forged, || panic!("forged factory")),
                Err(TerminalRegistryError::NotFound)
            );
            assert_eq!(
                first.withdraw_reserved_start(&forged),
                Err(TerminalRegistryError::NotFound)
            );
        }
        first.withdraw_reserved_start(&token).unwrap();
        assert_eq!(
            first.commit_reserved_start(&token, || panic!("withdrawn factory")),
            Err(TerminalRegistryError::NotFound)
        );
        assert_eq!(
            first.withdraw_reserved_start(&token),
            Err(TerminalRegistryError::NotFound)
        );
        let fresh = first.reserve_start(who, id("pending")).unwrap();
        assert_ne!(fresh.serial, token.serial);
    }

    #[test]
    fn reserved_start_capacity_is_shared_with_live_recovered_and_direct_admission() {
        let fixture = Fixture::new();
        let who = owner("one");
        let live_id = id("resident");
        let mut registry = registry();
        registry
            .start(who.clone(), live_id.clone(), || {
                fixture.live(&who, &live_id, 0)
            })
            .unwrap();
        let mut pending = Vec::new();
        for number in 0..MAX_RESIDENT_TERMINALS - 1 {
            pending.push(
                registry
                    .reserve_start(who.clone(), id(&format!("pending-{number}")))
                    .unwrap(),
            );
        }
        assert!(matches!(
            registry.reserve_start(who.clone(), live_id.clone()),
            Err(TerminalRegistryError::Conflict)
        ));
        assert!(matches!(
            registry.reserve_start(who.clone(), pending[0].id.clone()),
            Err(TerminalRegistryError::Conflict)
        ));
        assert!(matches!(
            registry.reserve_start(who.clone(), id("overflow")),
            Err(TerminalRegistryError::Capacity)
        ));
        assert_eq!(
            registry.start(who.clone(), id("overflow"), || panic!("full direct start")),
            Err(TerminalRegistryError::Capacity)
        );
        assert_eq!(
            registry.recover(who.clone(), id("overflow"), || panic!("full recovery")),
            Err(TerminalRegistryError::Capacity)
        );
        assert_eq!(
            registry.start(who.clone(), pending[0].id.clone(), || panic!(
                "duplicate direct start"
            )),
            Err(TerminalRegistryError::Conflict)
        );
        assert_eq!(
            registry.recover(who.clone(), pending[0].id.clone(), || panic!(
                "duplicate recovery"
            )),
            Err(TerminalRegistryError::Conflict)
        );
        registry
            .live_mut(&who, &live_id)
            .unwrap()
            .close(&who, TerminalClosePolicy::Force, 0)
            .unwrap();
        registry.release(&who, &live_id).unwrap();
        registry
            .recover(who.clone(), live_id.clone(), || {
                fixture.recovered(&who, &live_id, 0)
            })
            .unwrap();
        assert!(matches!(
            registry.reserve_start(who.clone(), id("overflow")),
            Err(TerminalRegistryError::Capacity)
        ));
        registry
            .withdraw_reserved_start(&pending.pop().unwrap())
            .unwrap();
        assert!(registry.reserve_start(who, id("released-slot")).is_ok());
    }

    #[test]
    fn reserved_start_is_invisible_and_does_not_block_resident_pumping() {
        let fixture = Fixture::new();
        let who = owner("one");
        let live_id = id("resident");
        let mut registry = registry();
        registry
            .start(who.clone(), live_id.clone(), || {
                fixture.live(&who, &live_id, 0)
            })
            .unwrap();
        let token = registry
            .reserve_start(who.clone(), id("preparing"))
            .unwrap();
        assert_eq!(registry.owner_ids(&who), vec![live_id.clone()]);
        assert!(matches!(
            registry.inspect(&who, token.session_id()),
            Err(TerminalRegistryError::NotFound)
        ));
        assert!(matches!(
            registry.live_mut(&who, token.session_id()),
            Err(TerminalRegistryError::NotFound)
        ));
        assert_eq!(
            registry
                .list(&who, None, 16, &TerminalRegistryFilter::default())
                .unwrap()
                .len(),
            1
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"still responsive".to_vec());
        let mut steps = registry.pump(1, 16).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps.remove(0).result.unwrap().output, b"still responsive");
        registry.withdraw_reserved_start(&token).unwrap();
    }

    #[test]
    fn reserved_start_commits_once_and_failures_allow_a_fresh_reservation() {
        let fixture = Fixture::new();
        let who = owner("one");
        let terminal = id("reserved");
        let mut registry = registry();
        let failed = registry
            .reserve_start(who.clone(), terminal.clone())
            .unwrap();
        assert_eq!(
            registry.commit_reserved_start(&failed, || Err(TerminalSessionError::Native)),
            Err(TerminalRegistryError::Session(TerminalSessionError::Native))
        );
        assert_eq!(
            registry.commit_reserved_start(&failed, || panic!("failed token replay")),
            Err(TerminalRegistryError::NotFound)
        );
        let panicked = registry
            .reserve_start(who.clone(), terminal.clone())
            .unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || registry.commit_reserved_start(&panicked, || panic!("factory panic"))
            ))
            .is_err()
        );
        assert_eq!(
            registry.commit_reserved_start(&panicked, || panic!("panic token replay")),
            Err(TerminalRegistryError::NotFound)
        );
        let committed = registry
            .reserve_start(who.clone(), terminal.clone())
            .unwrap();
        registry
            .commit_reserved_start(&committed, || fixture.live(&who, &terminal, 0))
            .unwrap();
        assert_eq!(
            registry.commit_reserved_start(&committed, || panic!("committed token replay")),
            Err(TerminalRegistryError::NotFound)
        );
        assert_eq!(
            registry.withdraw_reserved_start(&committed),
            Err(TerminalRegistryError::NotFound)
        );
        assert_eq!(registry.owner_ids(&who), vec![terminal]);
        assert!(registry.pending_starts.is_empty());
    }

    #[test]
    fn reserved_start_factory_facts_remain_bound_and_shutdown_invalidates_pending() {
        for foreign in [false, true] {
            let fixture = Fixture::new();
            let who = owner("one");
            let terminal = id("reserved");
            let mut registry = registry();
            let token = registry
                .reserve_start(who.clone(), terminal.clone())
                .unwrap();
            let actual_owner = if foreign {
                owner("foreign")
            } else {
                who.clone()
            };
            let actual_id = if foreign {
                terminal.clone()
            } else {
                id("other")
            };
            assert!(
                registry
                    .commit_reserved_start(&token, || fixture.live(&actual_owner, &actual_id, 0))
                    .is_err()
            );
            assert!(registry.entries.is_empty());
            assert!(registry.pending_starts.is_empty());
            let token = registry
                .reserve_start(who.clone(), terminal.clone())
                .unwrap();
            registry.shutdown(0, TerminalClosePolicy::Force).unwrap();
            assert!(registry.pending_starts.is_empty());
            assert_eq!(
                registry.commit_reserved_start(&token, || panic!("shutdown factory")),
                Err(TerminalRegistryError::Closed)
            );
            assert_eq!(
                registry.withdraw_reserved_start(&token),
                Err(TerminalRegistryError::Closed)
            );
            assert!(matches!(
                registry.reserve_start(who, terminal),
                Err(TerminalRegistryError::Closed)
            ));
        }
    }

    #[test]
    fn reserved_start_counter_exhaustion_never_recycles_token_identity() {
        let mut registry = registry();
        let who = owner("one");
        registry.next_start_serial = NonZeroU64::new(u64::MAX);
        let last = registry.reserve_start(who.clone(), id("last")).unwrap();
        assert_eq!(last.serial.get(), u64::MAX);
        registry.withdraw_reserved_start(&last).unwrap();
        assert!(registry.pending_starts.is_empty());
        assert!(matches!(
            registry.reserve_start(who, id("fresh")),
            Err(TerminalRegistryError::Capacity)
        ));
        assert_eq!(
            registry.commit_reserved_start(&last, || panic!("exhausted replay")),
            Err(TerminalRegistryError::NotFound)
        );
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
