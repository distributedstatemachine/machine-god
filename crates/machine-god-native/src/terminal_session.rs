//! Single-owner terminal driver. A persistent native host owns this value;
//! individual tool calls borrow it and do not own the process lifetime.

use crate::background_input::BackgroundInputReceipt;
use crate::background_process::BackgroundProcessSignal;
use crate::terminal_history::{TerminalHistory, TerminalHistoryError, TerminalHistoryEviction};
use crate::terminal_input::{
    TerminalInput, TerminalInputError, TerminalInputReceipt, TerminalWriterId,
};
use crate::terminal_journal::{TerminalJournalPage, TerminalJournalPhysicalUsage};
use crate::terminal_monitor::{
    MAX_MONITOR_FEED_BYTES, TerminalMonitorActivation, TerminalMonitorContext,
    TerminalMonitorError, TerminalMonitorMutation, TerminalMonitorSet, TerminalProbeEvidence,
    TerminalProbeRequest, TerminalProcessOutcome,
};
use crate::terminal_profile::{
    TerminalJournalPersistence, TerminalProfileBudget, TerminalProfileReadPermit,
};
use crate::terminal_profile_store::TerminalProfileTransaction;
use crate::terminal_pty::{
    TerminalPty, TerminalPtyClose, TerminalPtyDimensions, TerminalPtyRead, TerminalPtyStatus,
};
use crate::terminal_session_record::{
    TerminalSessionFacts, TerminalSessionMetadata, TerminalSessionRecordError, TerminalStartupStage,
};
use crate::terminal_startup::{TerminalStartupControl, TerminalStartupEvent};
use machine_god_core::{
    BackgroundOutputOwner, CancellationToken, TerminalActionResult, TerminalActorRole,
    TerminalAllowedControls, TerminalAttention, TerminalAttentionState, TerminalClosePolicy,
    TerminalCursor, TerminalDimensions, TerminalEventQuery, TerminalGap, TerminalLifecycle,
    TerminalMonitorEvent, TerminalMonitorOperation, TerminalScreen, TerminalSessionId,
    TerminalSignal, TerminalWriteLease, TerminalWriteLeaseIntent, TerminalWriteRequest,
};
use std::fmt;
use std::num::NonZeroU64;
use std::time::Instant;

/// Implementations retain native authority, never reconstruct it from a PID.
/// Read/write are nonblocking and bounded; Drop must release native ownership.
pub(crate) trait TerminalSessionBackend {
    /// Startup transports may disable echo until the trusted readiness marker.
    fn restore_startup_echo(&mut self) -> std::result::Result<(), ()> {
        Ok(())
    }
    fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()>;
    fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()>;
    fn write_with_paste(
        &mut self,
        bytes: &[u8],
        _paste: bool,
    ) -> std::result::Result<BackgroundInputReceipt, ()> {
        self.write(bytes)
    }
    fn input_write_limit(&self) -> usize {
        crate::background_input::MAX_BACKGROUND_INPUT_BYTES
    }
    fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()>;
    fn resize(&mut self, dimensions: &TerminalDimensions) -> std::result::Result<(), ()>;
    fn signal(&mut self, signal: TerminalSignal) -> std::result::Result<(), ()>;
    fn signal_may_discard_output(&self) -> bool;
    fn close(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
    ) -> std::result::Result<TerminalPtyClose, ()>;
}
impl TerminalSessionBackend for TerminalPty {
    fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
        self.read(buffer).map_err(|_| ())
    }
    fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
        self.write(bytes).map_err(|_| ())
    }
    fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
        self.status().map_err(|_| ())
    }
    fn resize(&mut self, dimensions: &TerminalDimensions) -> std::result::Result<(), ()> {
        self.resize(TerminalPtyDimensions {
            rows: dimensions.rows(),
            columns: dimensions.columns(),
        })
        .map_err(|_| ())
    }
    fn signal(&mut self, signal: TerminalSignal) -> std::result::Result<(), ()> {
        let signal = match signal {
            TerminalSignal::Hangup => BackgroundProcessSignal::Hangup,
            TerminalSignal::Interrupt => BackgroundProcessSignal::Interrupt,
            TerminalSignal::Quit => BackgroundProcessSignal::Quit,
            TerminalSignal::Terminate => BackgroundProcessSignal::Terminate,
            TerminalSignal::Kill => BackgroundProcessSignal::Kill,
        };
        self.signal(signal).map_err(|_| ())
    }
    fn signal_may_discard_output(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn close(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
    ) -> std::result::Result<TerminalPtyClose, ()> {
        self.close_with_output(force, output).map_err(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalSessionError {
    NotFound,
    InvalidState,
    Clock,
    Native,
    Input(TerminalInputError),
    History(TerminalHistoryError),
    Monitor(TerminalMonitorError),
    Record(TerminalSessionRecordError),
}
type Result<T> = std::result::Result<T, TerminalSessionError>;
impl From<TerminalHistoryError> for TerminalSessionError {
    fn from(error: TerminalHistoryError) -> Self {
        Self::History(error)
    }
}
impl From<TerminalInputError> for TerminalSessionError {
    fn from(error: TerminalInputError) -> Self {
        Self::Input(error)
    }
}
impl From<TerminalMonitorError> for TerminalSessionError {
    fn from(error: TerminalMonitorError) -> Self {
        Self::Monitor(error)
    }
}
impl From<TerminalSessionRecordError> for TerminalSessionError {
    fn from(error: TerminalSessionRecordError) -> Self {
        Self::Record(error)
    }
}

/// Output is bounded for active wait matchers; probe requests are descriptions,
/// not authorization for a host to perform their effects.
pub(crate) struct TerminalSessionStep {
    pub(crate) output: Vec<u8>,
    pub(crate) cursor: TerminalCursor,
    pub(crate) probes: Vec<TerminalProbeRequest>,
    pub(crate) lifecycle: TerminalLifecycle,
    /// The read permit must be released before an ordinary-context exit drain.
    pub(crate) cleanup_needed: bool,
}
#[derive(Clone, Copy)]
enum TerminalPumpMode {
    RunningRead,
    WithExitDrain,
}
struct SessionStartup {
    control: TerminalStartupControl,
    cancellation: CancellationToken,
    /// True means the durable transition succeeded; only its ACK may be retried.
    pending: Option<(TerminalStartupEvent, bool)>,
}
pub(crate) struct TerminalSession<B: TerminalSessionBackend> {
    backend: Option<B>,
    history: TerminalHistory,
    owner: BackgroundOutputOwner,
    metadata: TerminalSessionMetadata,
    input: TerminalInput,
    attention: TerminalAttentionState,
    attention_writer: Option<(TerminalActorRole, TerminalWriterId)>,
    monitors: TerminalMonitorSet,
    lifecycle: TerminalLifecycle,
    outcome: Option<TerminalProcessOutcome>,
    created_at_ms: i64,
    monitor_notifications_incomplete: bool,
    publication_error: Option<TerminalSessionError>,
    pending_output_gap: bool,
    now_ms: i64,
    last_output_ms: i64,
    startup: Option<SessionStartup>,
    startup_stage: TerminalStartupStage,
    command_start_cursor: Option<TerminalCursor>,
}
impl<B: TerminalSessionBackend> fmt::Debug for TerminalSession<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalSession").finish_non_exhaustive()
    }
}
impl<B: TerminalSessionBackend> TerminalSession<B> {
    /// Takes already-owned transport and fresh history. Only a trusted readiness
    /// channel calls `shell_ready`; raw output is never readiness authority.
    #[cfg(test)]
    pub(crate) fn new(
        backend: B,
        history: TerminalHistory,
        owner: BackgroundOutputOwner,
        id: TerminalSessionId,
        metadata: TerminalSessionMetadata,
        now_ms: i64,
    ) -> Result<Self> {
        Self::new_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            backend,
            history,
            owner,
            id,
            metadata,
            now_ms,
        )
    }

    pub(crate) fn new_with(
        persistence: &mut dyn TerminalJournalPersistence,
        backend: B,
        history: TerminalHistory,
        owner: BackgroundOutputOwner,
        id: TerminalSessionId,
        metadata: TerminalSessionMetadata,
        now_ms: i64,
    ) -> Result<Self> {
        metadata.validate()?;
        history.require_live()?;
        if history.session_id() != &id {
            return Err(TerminalSessionError::InvalidState);
        }
        let monitors = TerminalMonitorSet::new(
            id,
            TerminalMonitorContext {
                now_ms,
                cursor: history.latest(),
                lifecycle: TerminalLifecycle::Starting,
            },
        )?;
        let mut session = Self {
            backend: Some(backend),
            history,
            owner,
            metadata,
            input: TerminalInput::new(),
            attention: TerminalAttentionState::default(),
            attention_writer: None,
            monitors,
            lifecycle: TerminalLifecycle::Starting,
            outcome: None,
            created_at_ms: now_ms,
            monitor_notifications_incomplete: false,
            publication_error: None,
            pending_output_gap: false,
            now_ms,
            last_output_ms: now_ms,
            startup: None,
            startup_stage: TerminalStartupStage::Prepared,
            command_start_cursor: None,
        };
        session.persist_with(persistence)?;
        Ok(session)
    }
    #[cfg(test)]
    pub(crate) fn shell_ready(&mut self, now_ms: i64) -> Result<()> {
        self.shell_ready_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            now_ms,
        )
    }

    pub(crate) fn shell_ready_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        now_ms: i64,
    ) -> Result<()> {
        self.check_time(now_ms)?;
        if self.lifecycle != TerminalLifecycle::Starting
            || self.startup_stage != TerminalStartupStage::Prepared
        {
            return Err(TerminalSessionError::InvalidState);
        }
        self.now_ms = now_ms;
        if self
            .backend
            .as_mut()
            .ok_or(TerminalSessionError::InvalidState)?
            .restore_startup_echo()
            .is_err()
        {
            return Err(self.failed_observation_with(persistence, TerminalSessionError::Native));
        }
        self.startup_stage = TerminalStartupStage::ShellReady;
        if self.metadata.command.is_none() {
            self.lifecycle = TerminalLifecycle::Running;
        }
        self.persist_with(persistence)
    }

    /// The trusted owner has drained profile bytes before this boundary. This
    /// durable observation alone never grants recovered process authority.
    pub(crate) fn command_started_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        now_ms: i64,
    ) -> Result<()> {
        self.check_time(now_ms)?;
        if self.lifecycle != TerminalLifecycle::Starting
            || self.startup_stage != TerminalStartupStage::ShellReady
            || self.metadata.command.is_none()
        {
            return Err(TerminalSessionError::InvalidState);
        }
        self.now_ms = now_ms;
        self.command_start_cursor = Some(self.history.latest());
        self.startup_stage = TerminalStartupStage::CommandStarted;
        self.lifecycle = TerminalLifecycle::Running;
        self.persist_with(persistence)
    }

    /// Trusted host assembly transfers the matching control half on the owner
    /// worker. The operation token governs startup only, never later input.
    pub(crate) fn attach_startup_control(
        &mut self,
        control: TerminalStartupControl,
        cancellation: CancellationToken,
    ) -> Result<()> {
        if self.startup.is_some()
            || self.lifecycle != TerminalLifecycle::Starting
            || self.startup_stage != TerminalStartupStage::Prepared
            || control.has_command() != self.metadata.command.is_some()
        {
            return Err(TerminalSessionError::InvalidState);
        }
        self.startup = Some(SessionStartup {
            control,
            cancellation,
            pending: None,
        });
        Ok(())
    }

    fn poll_startup_control(&mut self) -> Result<()> {
        if let Some(startup) = &mut self.startup
            && let Some(event) = startup
                .control
                .poll(Instant::now(), &startup.cancellation)
                .map_err(|_| TerminalSessionError::Native)?
        {
            if startup.pending.is_some() {
                return Err(TerminalSessionError::InvalidState);
            }
            startup.pending = Some((event, false));
        }
        Ok(())
    }

    /// Called after an admitted read observes no more startup output, with its
    /// read permit released. Ordinary persistence precedes every ACK; the
    /// command cannot run while profile output is still ahead of its cursor.
    pub(crate) fn advance_startup_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        now_ms: i64,
    ) -> Result<()> {
        self.check_time(now_ms)?;
        self.now_ms = now_ms;
        let result = (|| {
            // The event was observed BEFORE the admitted read. Polling for a
            // new event here would race profile bytes arriving between an
            // empty read and marker receipt, misplacing the command cursor.
            let Some((event, published)) =
                self.startup.as_ref().and_then(|startup| startup.pending)
            else {
                return Ok(());
            };
            if !published {
                match event {
                    TerminalStartupEvent::ShellReady => {
                        self.shell_ready_with(persistence, now_ms)?;
                    }
                    TerminalStartupEvent::CommandStarted => {
                        self.command_started_with(persistence, now_ms)?;
                    }
                }
                self.startup.as_mut().expect("attached startup").pending = Some((event, true));
            }
            let startup = self.startup.as_mut().expect("attached startup");
            let acknowledged = match event {
                TerminalStartupEvent::ShellReady => startup
                    .control
                    .acknowledge_shell_ready(Instant::now(), &startup.cancellation),
                TerminalStartupEvent::CommandStarted => startup
                    .control
                    .release_command(Instant::now(), &startup.cancellation),
            }
            .map_err(|_| TerminalSessionError::Native)?;
            if acknowledged {
                startup.pending = None;
            }
            Ok(())
        })();
        if let Err(error) = result {
            return Err(self.abort_startup_with(Some(persistence), now_ms, error));
        }
        if let Some(startup) = &mut self.startup
            && startup.control.is_complete()
            && startup.control.retry_cleanup().is_ok()
        {
            self.startup.take();
        }
        Ok(())
    }

    fn abort_startup_with(
        &mut self,
        persistence: Option<&mut dyn TerminalJournalPersistence>,
        now_ms: i64,
        error: TerminalSessionError,
    ) -> TerminalSessionError {
        self.startup.take();
        self.now_ms = self.now_ms.max(now_ms);
        if let Some(persistence) = persistence {
            let native = self.finish_native_with(persistence, true, TerminalLifecycle::Lost);
            self.lose();
            let _ = self.persist_with(persistence);
            if let Err(failure) = native {
                self.publication_error.get_or_insert(failure);
            }
        } else {
            let _ = self.teardown_without_persistence(true, now_ms, error);
            self.lose();
        }
        error
    }

    /// A failed startup read cannot leave a blocked shell alive. The caller
    /// must release any restricted read permit before supplying this context.
    pub(crate) fn fail_pending_startup_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        now_ms: i64,
        error: TerminalSessionError,
    ) -> TerminalSessionError {
        if self.startup.is_some() {
            self.abort_startup_with(Some(persistence), now_ms, error)
        } else {
            error
        }
    }
    pub(crate) fn context(&self) -> TerminalMonitorContext {
        TerminalMonitorContext {
            now_ms: self.now_ms,
            cursor: self.history.latest(),
            lifecycle: self.lifecycle,
        }
    }
    pub(crate) fn outcome(&self) -> Option<TerminalProcessOutcome> {
        self.outcome
    }
    pub(crate) fn last_output_ms(&self) -> i64 {
        self.last_output_ms
    }
    pub(crate) fn owns_backend(&self) -> bool {
        self.backend.is_some()
    }
    /// Observe whether the owner needs a cleanup transaction instead of one
    /// bounded-read permit. Does not consume output or publish session facts;
    /// `pump_with` still rechecks status to handle a later process exit.
    pub(crate) fn needs_native_cleanup(&mut self) -> Result<bool> {
        self.poll_startup_control()?;
        let Some(backend) = self.backend.as_mut() else {
            return Ok(false);
        };
        if !matches!(
            self.lifecycle,
            TerminalLifecycle::Starting | TerminalLifecycle::Running
        ) {
            return Ok(true);
        }
        backend
            .status()
            .map(|status| status != TerminalPtyStatus::Running)
            .map_err(|()| TerminalSessionError::Native)
    }
    /// Native cleanup and durable state publication are independent obligations.
    pub(crate) fn publication_error(&self) -> Option<TerminalSessionError> {
        self.publication_error
    }
    /// A failed native status observation is not a harmless admission deferral.
    /// Quiesce the lost session even without persistence authority; in that case
    /// retain a publication obligation without performing any journal writes.
    pub(crate) fn native_status_failed_with(
        &mut self,
        persistence: Option<&mut dyn TerminalJournalPersistence>,
        now_ms: i64,
    ) -> Result<TerminalSessionStep> {
        self.check_time(now_ms)?;
        self.now_ms = now_ms;
        let error = TerminalSessionError::Native;
        if self.startup.is_some() {
            return Err(self.abort_startup_with(persistence, now_ms, error));
        }
        if let Some(persistence) = persistence {
            return Err(self.failed_observation_with(persistence, error));
        }
        self.failed_publication();
        self.publication_error = Some(error);
        Err(error)
    }
    /// Register persistent capacity using ordinary authority before borrowing
    /// that same transaction for the restricted one-read permit.
    pub(crate) fn ensure_checkpoint_reserve_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
    ) -> Result<()> {
        Ok(self.history.ensure_checkpoint_reserve_with(persistence)?)
    }

    pub(crate) fn required_profile_read_growth(&self) -> Result<u64> {
        Ok(self.history.required_profile_read_growth()?)
    }

    pub(crate) fn checkpoint_reserve_bytes(&self) -> usize {
        self.history.checkpoint_reserve_bytes()
    }

    pub(crate) fn preflight_profile_read(
        &mut self,
        transaction: &mut TerminalProfileTransaction<'_>,
        budget: &TerminalProfileBudget,
        owner_namespace: &str,
    ) -> Result<()> {
        Ok(self
            .history
            .preflight_profile_read(transaction, budget, owner_namespace)?)
    }

    /// Reserve the whole bounded read before the owner invokes `pump_with`.
    /// The permit borrows the held profile transaction, not this session.
    pub(crate) fn reserve_profile_read<'a, 'store>(
        &mut self,
        transaction: &'a mut TerminalProfileTransaction<'store>,
        budget: &TerminalProfileBudget,
        owner_namespace: &str,
    ) -> Result<TerminalProfileReadPermit<'a, 'store>> {
        Ok(self
            .history
            .reserve_read(transaction, budget, owner_namespace)?)
    }
    pub(crate) fn inspect(&self, owner: &BackgroundOutputOwner) -> Result<TerminalSessionFacts> {
        self.authorize(owner)?;
        self.facts()
    }

    pub(crate) fn public_facts(
        &self,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        controls: &TerminalAllowedControls,
    ) -> Result<machine_god_core::TerminalSessionFacts> {
        self.authorize(owner)?;
        let mut facts = project_facts(
            &self.facts()?,
            &self.history,
            &self.monitors,
            actor,
            controls,
        )?;
        facts.next_actions.write &= !self.input.is_quiesced();
        Ok(facts)
    }

    pub(crate) fn prepare_public_facts_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
    ) -> Result<()> {
        self.authorize(owner)?;
        self.history
            .prepare_public_facts_with(persistence)
            .map_err(|error| self.failed_observation_with(persistence, error.into()))
    }

    pub(crate) fn inspect_result_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        query: &TerminalEventQuery,
        controls: &TerminalAllowedControls,
    ) -> Result<TerminalActionResult> {
        // Validate the bounded projection before an acknowledgement mutation.
        self.authorize(owner)?;
        query
            .validate()
            .map_err(|_| TerminalMonitorError::Invalid)?;
        self.prepare_public_facts_with(persistence, owner)?;
        self.public_facts(owner, actor, controls)?;
        let events = self.events_with(persistence, owner, query)?;
        project_inspect(
            self.public_facts(owner, actor, controls)?,
            &self.metadata,
            &self.monitors,
            events,
        )
    }
    pub(crate) fn physical_usage(
        &self,
        owner: &BackgroundOutputOwner,
    ) -> Result<TerminalJournalPhysicalUsage> {
        self.authorize(owner)?;
        Ok(self.history.physical_usage()?)
    }
    fn authorize_retention(
        &self,
        owner: &BackgroundOutputOwner,
        kind: TerminalHistoryEviction,
    ) -> Result<()> {
        self.authorize(owner)?;
        if let Some(error) = self.publication_error {
            return Err(error);
        }
        let eligible = match kind {
            TerminalHistoryEviction::LiveCoveredOutput => {
                self.owns_backend()
                    && matches!(
                        self.lifecycle,
                        TerminalLifecycle::Starting | TerminalLifecycle::Running
                    )
            }
            TerminalHistoryEviction::CompletedOutput
            | TerminalHistoryEviction::CompletedCheckpoint => {
                !self.owns_backend()
                    && matches!(
                        self.lifecycle,
                        TerminalLifecycle::Closed | TerminalLifecycle::Exited
                    )
            }
        };
        if eligible {
            Ok(())
        } else {
            Err(TerminalSessionError::InvalidState)
        }
    }
    pub(crate) fn eviction_bytes(
        &self,
        owner: &BackgroundOutputOwner,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        self.authorize_retention(owner, kind)?;
        Ok(self.history.eviction_bytes(kind)?)
    }
    #[cfg(test)]
    pub(crate) fn evict(
        &mut self,
        owner: &BackgroundOutputOwner,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        self.evict_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            kind,
        )
    }

    pub(crate) fn evict_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        self.authorize_retention(owner, kind)?;
        self.history
            .evict_with(persistence, kind)
            .map_err(|error| self.failed_observation_with(persistence, error.into()))
    }
    pub(crate) fn read(
        &self,
        owner: &BackgroundOutputOwner,
        cursor: &TerminalCursor,
        maximum: usize,
    ) -> Result<TerminalJournalPage> {
        self.authorize(owner)?;
        Ok(self.history.read(cursor, maximum)?)
    }
    pub(crate) fn screen(&self, owner: &BackgroundOutputOwner) -> Result<TerminalScreen> {
        self.authorize(owner)?;
        Ok(self.history.screen()?)
    }
    #[cfg(test)]
    pub(crate) fn write(
        &mut self,
        owner: &BackgroundOutputOwner,
        writer: TerminalWriterId,
        request: &TerminalWriteRequest,
        cancelled: bool,
    ) -> Result<TerminalInputReceipt> {
        self.write_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            TerminalActorRole::Agent,
            writer,
            request,
            cancelled,
        )
    }

    pub(crate) fn write_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        request: &TerminalWriteRequest,
        cancelled: bool,
    ) -> Result<TerminalInputReceipt> {
        let (receipt, error) =
            self.write_completion_with(persistence, owner, actor, writer, request, cancelled)?;
        error.map_or(Ok(receipt), Err)
    }

    /// Admission failures have no accepted receipt. Once input is submitted,
    /// preserve its exact accepted-byte receipt independently of a subsequent
    /// durable quiescence-publication error; callers must never retry payload
    /// bytes merely because that separate publication failed.
    pub(crate) fn write_completion_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        request: &TerminalWriteRequest,
        cancelled: bool,
    ) -> Result<(TerminalInputReceipt, Option<TerminalSessionError>)> {
        self.authorize(owner)?;
        if self.lifecycle != TerminalLifecycle::Running {
            return Err(TerminalSessionError::InvalidState);
        }
        self.input.check_submit(actor, writer, request, cancelled)?;
        if request.lease != TerminalWriteLeaseIntent::Revoke {
            self.check_attention_writer(actor, writer)?;
        }
        let status = self
            .backend
            .as_mut()
            .ok_or(TerminalSessionError::InvalidState)?
            .status();
        let status = status.map_err(|()| {
            self.failed_observation_with(persistence, TerminalSessionError::Native)
        })?;
        if status != TerminalPtyStatus::Running {
            self.clear_input_authority();
            self.persist_with(persistence)?;
            return Err(TerminalSessionError::InvalidState);
        }
        match request.lease {
            TerminalWriteLeaseIntent::Acquire => {
                let attention = if actor == TerminalActorRole::Human {
                    TerminalAttention::UserTakeover
                } else {
                    self.attention.attention()
                };
                let state = TerminalAttentionState::new(attention, actor_lease(actor))
                    .map_err(|_| TerminalInputError::LeaseConflict)?;
                let claimant = if actor == TerminalActorRole::Human {
                    Some((actor, writer))
                } else {
                    self.attention_writer
                };
                self.publish_attention_with(persistence, state, claimant)?;
            }
            TerminalWriteLeaseIntent::Release => {
                let state = self.attention.cancel(actor);
                let claimant = self.remaining_attention_writer(actor);
                self.publish_attention_with(persistence, state, claimant)?;
            }
            TerminalWriteLeaseIntent::Revoke => {
                self.publish_attention_with(persistence, TerminalAttentionState::default(), None)?;
            }
            TerminalWriteLeaseIntent::Use => {}
        }
        let receipt = self
            .input
            .submit_with_actor(actor, writer, request, cancelled)?;
        let mut publication_error = None;
        if receipt.operation_id.is_some() {
            self.flush_input();
            if self.input.is_quiesced() {
                publication_error = self.persist_with(persistence).err();
            }
        }
        let receipt = receipt.operation_id.map_or(Ok(receipt), |operation| {
            self.input.receipt_with_actor(actor, writer, operation)
        })?;
        Ok((receipt, publication_error))
    }
    #[cfg(test)]
    pub(crate) fn write_receipt(
        &self,
        owner: &BackgroundOutputOwner,
        writer: TerminalWriterId,
        operation: NonZeroU64,
    ) -> Result<TerminalInputReceipt> {
        self.write_receipt_with_actor(owner, TerminalActorRole::Agent, writer, operation)
    }

    pub(crate) fn write_receipt_with_actor(
        &self,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        operation: NonZeroU64,
    ) -> Result<TerminalInputReceipt> {
        self.authorize(owner)?;
        Ok(self.input.receipt_with_actor(actor, writer, operation)?)
    }

    pub(crate) fn begin_attention_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        now_ms: i64,
    ) -> Result<TerminalAttentionState> {
        self.authorize(owner)?;
        self.check_time(now_ms)?;
        if !matches!(
            self.lifecycle,
            TerminalLifecycle::Starting | TerminalLifecycle::Running
        ) {
            return Err(TerminalSessionError::InvalidState);
        }
        self.check_attention_writer(actor, writer)?;
        self.input.check_cancel(actor, writer)?;
        if actor == TerminalActorRole::Agent {
            let state = TerminalAttentionState::new(
                TerminalAttention::AgentWait,
                self.attention.write_lease(),
            )
            .map_err(|_| TerminalInputError::LeaseConflict)?;
            self.now_ms = now_ms;
            self.publish_attention_with(persistence, state, Some((actor, writer)))?;
        }
        Ok(self.attention.clone())
    }

    pub(crate) fn cancel_attention_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        now_ms: i64,
    ) -> Result<TerminalAttentionState> {
        self.finish_attention_with(persistence, owner, actor, writer, now_ms, true)
    }

    pub(crate) fn finish_attention_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
        now_ms: i64,
        cancelled: bool,
    ) -> Result<TerminalAttentionState> {
        self.authorize(owner)?;
        if let Some(error) = self.publication_error {
            return Err(error);
        }
        self.check_time(now_ms)?;
        self.check_attention_writer(actor, writer)?;
        self.input.check_cancel(actor, writer)?;
        let mut state = self.attention.clone();
        if actor == TerminalActorRole::Agent && state.attention() == TerminalAttention::AgentWait {
            state = TerminalAttentionState::new(TerminalAttention::Background, state.write_lease())
                .map_err(|_| TerminalSessionError::InvalidState)?;
        }
        if cancelled {
            state = state.cancel(actor);
        }
        self.now_ms = now_ms;
        let claimant = if cancelled || actor == TerminalActorRole::Agent {
            self.remaining_attention_writer(actor)
        } else {
            self.attention_writer
        };
        self.publish_attention_with(persistence, state, claimant)?;
        if cancelled {
            self.input.cancel_claim(actor, writer);
        }
        Ok(self.attention.clone())
    }

    fn check_attention_writer(
        &self,
        actor: TerminalActorRole,
        writer: TerminalWriterId,
    ) -> Result<()> {
        if self
            .attention_writer
            .is_some_and(|(role, holder)| role == actor && holder != writer)
        {
            Err(TerminalInputError::LeaseConflict.into())
        } else {
            Ok(())
        }
    }

    fn remaining_attention_writer(
        &self,
        actor: TerminalActorRole,
    ) -> Option<(TerminalActorRole, TerminalWriterId)> {
        self.attention_writer.filter(|(role, _)| *role != actor)
    }

    fn publish_attention_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        state: TerminalAttentionState,
        writer: Option<(TerminalActorRole, TerminalWriterId)>,
    ) -> Result<()> {
        if self.attention == state && self.attention_writer == writer {
            return Ok(());
        }
        self.attention = state;
        self.attention_writer = writer;
        // Publication failure invokes the common loss path, clearing authority
        // and retaining the backend solely for native cleanup. Never grant a
        // lease after either precommit or committed-accounting failure.
        self.persist_with(persistence)
    }

    fn clear_input_authority(&mut self) {
        self.input.quiesce();
        self.attention = TerminalAttentionState::default();
        self.attention_writer = None;
    }

    /// One nonblocking write, one <=16 KiB read, then bounded monitor work.
    /// Cancelling an attention future does not drop a committed input suffix.
    #[cfg(test)]
    pub(crate) fn pump(&mut self, now_ms: i64) -> Result<TerminalSessionStep> {
        self.pump_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            now_ms,
        )
    }

    pub(crate) fn pump_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        now_ms: i64,
    ) -> Result<TerminalSessionStep> {
        if let Err(error) = self.poll_startup_control() {
            return Err(self.abort_startup_with(Some(persistence), now_ms, error));
        }
        let mut step = self
            .pump_mode_with(persistence, now_ms, TerminalPumpMode::WithExitDrain)
            .map_err(|error| self.fail_pending_startup_with(persistence, now_ms, error))?;
        if step.output.is_empty()
            && self.backend.is_some()
            && matches!(
                self.lifecycle,
                TerminalLifecycle::Starting | TerminalLifecycle::Running
            )
        {
            self.advance_startup_with(persistence, now_ms)?;
            step.lifecycle = self.lifecycle;
        }
        Ok(step)
    }

    /// Consume at most one admitted read, never an exit drain. The owner drops
    /// the read permit before handling `cleanup_needed` with ordinary held
    /// persistence authority. Output already returned here is not replayed.
    pub(crate) fn pump_read_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        now_ms: i64,
    ) -> Result<TerminalSessionStep> {
        self.pump_mode_with(persistence, now_ms, TerminalPumpMode::RunningRead)
    }

    fn pump_mode_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        now_ms: i64,
        mode: TerminalPumpMode,
    ) -> Result<TerminalSessionStep> {
        self.check_time(now_ms)?;
        let before_lifecycle = self.lifecycle;
        let timer_due = self
            .monitors
            .next_deadline()
            .is_some_and(|deadline| deadline <= now_ms);
        self.now_ms = now_ms;
        if !matches!(
            self.lifecycle,
            TerminalLifecycle::Starting | TerminalLifecycle::Running
        ) {
            return Ok(self.step(Vec::new(), Vec::new()));
        }
        match self.pump_inner_with(persistence, mode) {
            Ok(step) => {
                if timer_due
                    || !step.output.is_empty()
                    || !step.probes.is_empty()
                    || self.lifecycle != before_lifecycle
                {
                    self.persist_with(persistence)?;
                    self.release_completed_reserve_with(persistence)?;
                }
                Ok(step)
            }
            Err(error) => Err(self.failed_observation_with(persistence, error)),
        }
    }
    fn pump_inner_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        mode: TerminalPumpMode,
    ) -> Result<TerminalSessionStep> {
        let status = self
            .backend
            .as_mut()
            .ok_or(TerminalSessionError::InvalidState)?
            .status()
            .map_err(|()| TerminalSessionError::Native)?;
        if status != TerminalPtyStatus::Running {
            if matches!(mode, TerminalPumpMode::RunningRead) {
                return Ok(self.cleanup_step(Vec::new()));
            }
            // Retain final output and close before reaping the shell, including
            // owned jobs left behind by an exited interactive shell.
            self.finish_native_with(persistence, false, TerminalLifecycle::Exited)?;
            return Ok(self.step(Vec::new(), Vec::new()));
        }
        self.flush_input();
        let mut buffer = [0; MAX_MONITOR_FEED_BYTES];
        let read = self
            .backend
            .as_mut()
            .ok_or(TerminalSessionError::InvalidState)?
            .read(&mut buffer)
            .map_err(|()| TerminalSessionError::Native)?;
        if read.bytes_read > buffer.len() {
            return Err(TerminalSessionError::Native);
        }
        let output = buffer[..read.bytes_read].to_vec();
        if !output.is_empty() {
            self.last_output_ms = self.now_ms;
            let receipt = match self.history.append_with(persistence, &output) {
                Ok(receipt) => receipt,
                Err(error) => {
                    self.pending_output_gap = true;
                    self.history.invalidate_output_without_persistence();
                    self.monitor_notifications_incomplete = true;
                    return Err(error.into());
                }
            };
            if let Some(error) = receipt.accounting_error {
                // The bytes are committed. Do not retry their append or enqueue
                // their protocol replies after admission accounting failed.
                self.monitor_notifications_incomplete = true;
                return Err(TerminalHistoryError::Accounting(error).into());
            }
            self.monitors.output(&output, self.context())?;
            if receipt.screen_unavailable.is_some() {
                self.monitors.raw_gap(self.context())?;
            } else if self.monitors.needs_screen() {
                self.monitors
                    .screen(&self.history.screen()?, self.context())?;
            }
            if !receipt.replies.is_empty() && !self.input.is_quiesced() {
                self.input.replies(receipt.replies)?;
            }
        }
        if read.closed {
            let status = self
                .backend
                .as_mut()
                .ok_or(TerminalSessionError::InvalidState)?
                .status()
                .map_err(|()| TerminalSessionError::Native)?;
            if status == TerminalPtyStatus::Running {
                return Err(TerminalSessionError::Native);
            }
            if matches!(mode, TerminalPumpMode::RunningRead) {
                return Ok(self.cleanup_step(output));
            }
            self.finish_native_with(persistence, false, TerminalLifecycle::Exited)?;
            return Ok(self.step(output, Vec::new()));
        }
        let probes = self.monitors.tick(self.context())?;
        Ok(self.step(output, probes))
    }
    #[cfg(test)]
    pub(crate) fn resize(
        &mut self,
        owner: &BackgroundOutputOwner,
        dimensions: &TerminalDimensions,
        now_ms: i64,
    ) -> Result<()> {
        self.resize_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            dimensions,
            now_ms,
        )
    }

    pub(crate) fn resize_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        dimensions: &TerminalDimensions,
        now_ms: i64,
    ) -> Result<()> {
        self.authorize_running(owner, now_ms)?;
        self.history.validate_resize(dimensions)?;
        let backend = self
            .backend
            .as_mut()
            .ok_or(TerminalSessionError::InvalidState)?;
        let mut native_attempted = false;
        let resized = self
            .history
            .resize_with(persistence, dimensions, |dimensions| {
                native_attempted = true;
                backend.resize(dimensions)
            });
        if let Err(error) = resized {
            // Only initial barrier admission is effect-free. The same quota
            // error can reject the final checkpoint after native resize.
            if !native_attempted && matches!(error, TerminalHistoryError::Profile(_)) {
                return Err(error.into());
            }
            return Err(self.failed_observation_with(persistence, error.into()));
        }
        self.now_ms = now_ms;
        if self.monitors.needs_screen() {
            let observed = self
                .history
                .screen()
                .map_err(TerminalSessionError::from)
                .and_then(|screen| {
                    self.monitors
                        .screen(&screen, self.context())
                        .map_err(TerminalSessionError::from)
                });
            if let Err(error) = observed {
                return Err(self.failed_observation_with(persistence, error));
            }
        }
        self.persist_with(persistence)
    }
    #[cfg(test)]
    pub(crate) fn signal(
        &mut self,
        owner: &BackgroundOutputOwner,
        signal: TerminalSignal,
        now_ms: i64,
    ) -> Result<()> {
        self.signal_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            signal,
            now_ms,
        )
    }

    pub(crate) fn signal_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        signal: TerminalSignal,
        now_ms: i64,
    ) -> Result<()> {
        self.authorize_running(owner, now_ms)?;
        if self
            .backend
            .as_ref()
            .ok_or(TerminalSessionError::InvalidState)?
            .signal_may_discard_output()
        {
            if let Err(error) = self.history.mark_output_gap_with(persistence) {
                if matches!(error, TerminalHistoryError::Profile(_)) {
                    return Err(error.into());
                }
                return Err(self.failed_observation_with(persistence, error.into()));
            }
            self.now_ms = now_ms;
            if let Err(error) = self.monitors.raw_gap(self.context()) {
                return Err(self.failed_observation_with(persistence, error.into()));
            }
            self.persist_with(persistence)?;
        }
        // A failed signal is not retried and never escalates into close/kill.
        self.backend
            .as_mut()
            .ok_or(TerminalSessionError::InvalidState)?
            .signal(signal)
            .map_err(|()| TerminalSessionError::Native)?;
        self.now_ms = now_ms;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn close(
        &mut self,
        owner: &BackgroundOutputOwner,
        policy: TerminalClosePolicy,
        now_ms: i64,
    ) -> Result<()> {
        self.close_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            policy,
            now_ms,
        )
    }

    pub(crate) fn close_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        policy: TerminalClosePolicy,
        now_ms: i64,
    ) -> Result<()> {
        self.authorize(owner)?;
        self.check_time(now_ms)?;
        self.now_ms = now_ms;
        let native = self.finish_native_with(
            persistence,
            policy == TerminalClosePolicy::Force,
            TerminalLifecycle::Closed,
        );
        // Persist the observed result after cleanup, even when cleanup failed.
        // Persistence failure must never prevent the first native cleanup attempt.
        let persisted = self.persist_with(persistence);
        if self.publication_error.is_none()
            && let Err(error @ TerminalSessionError::History(_)) = native
        {
            self.publication_error = Some(error);
        }
        native.and(persisted)?;
        self.release_completed_reserve_with(persistence)
    }

    pub(crate) fn release_completed_reserve_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
    ) -> Result<()> {
        if self.backend.is_none()
            && self.publication_error.is_none()
            && !self.pending_output_gap
            && matches!(
                self.lifecycle,
                TerminalLifecycle::Exited | TerminalLifecycle::Closed
            )
            && let Err(error) = self.history.release_checkpoint_reserve_with(persistence)
        {
            let error = TerminalSessionError::from(error);
            self.publication_error = Some(error);
            return Err(error);
        }
        Ok(())
    }

    /// Last-resort owner cleanup when no profile authority can be obtained.
    /// This path never publishes a journal mutation and never returns durable
    /// success. Owned native authority survives failed cleanup for retry.
    pub(crate) fn teardown_without_persistence(
        &mut self,
        force: bool,
        now_ms: i64,
        error: TerminalSessionError,
    ) -> Result<()> {
        self.now_ms = self.now_ms.max(now_ms);
        self.clear_input_authority();
        self.monitors.quiesce();
        self.monitor_notifications_incomplete = true;
        self.publication_error.get_or_insert(error);
        let Some(backend) = self.backend.as_mut() else {
            return Err(error);
        };
        self.pending_output_gap = true;
        self.history.invalidate_output_without_persistence();
        let mut observed_output = false;
        let closed = backend
            .close(force, &mut |bytes| {
                observed_output |= !bytes.is_empty();
            })
            .and_then(require_closed_status);
        if observed_output {
            self.last_output_ms = self.now_ms;
        }
        if let Ok(closed) = closed {
            self.outcome = outcome(closed.status);
            self.lifecycle = TerminalLifecycle::Closed;
            self.backend.take();
            Err(error)
        } else {
            self.lose();
            Err(TerminalSessionError::Native)
        }
    }

    fn finish_native_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        force: bool,
        final_lifecycle: TerminalLifecycle,
    ) -> Result<()> {
        self.clear_input_authority();
        let Some(backend) = self.backend.as_mut() else {
            if final_lifecycle == TerminalLifecycle::Closed {
                self.lifecycle = TerminalLifecycle::Closed;
            }
            return Ok(());
        };
        let mut history_error = None;
        let monitors = &mut self.monitors;
        let now_ms = self.now_ms;
        let lifecycle = self.lifecycle;
        let mut observed_output = false;
        let mut monitor_error = None;
        let result = match self.history.begin_close_with(persistence) {
            Ok(mut capture) => {
                self.pending_output_gap = false;
                let result = backend
                    .close(force, &mut |bytes| {
                        if bytes.is_empty() {
                            return;
                        }
                        observed_output = true;
                        match capture.append(bytes) {
                            Ok(cursor) if monitor_error.is_none() => {
                                for chunk in bytes.chunks(MAX_MONITOR_FEED_BYTES) {
                                    if let Err(error) = monitors.output(
                                        chunk,
                                        TerminalMonitorContext {
                                            now_ms,
                                            cursor: cursor.clone(),
                                            lifecycle,
                                        },
                                    ) {
                                        monitor_error = Some(error);
                                        break;
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(error) => {
                                history_error.get_or_insert(error);
                            }
                        }
                    })
                    .and_then(require_closed_status);
                if let Err(error) = capture.finish(
                    result
                        .as_ref()
                        .is_ok_and(|closed| !closed.output_incomplete),
                ) {
                    history_error.get_or_insert(error);
                }
                result
            }
            Err(error) => {
                history_error = Some(error);
                self.pending_output_gap = !matches!(error, TerminalHistoryError::Accounting(_));
                backend
                    .close(force, &mut |bytes| observed_output |= !bytes.is_empty())
                    .and_then(require_closed_status)
            }
        };
        if self.pending_output_gap {
            self.history.invalidate_output_without_persistence();
        }
        if observed_output {
            self.last_output_ms = now_ms;
        }
        if let Ok(closed) = result {
            self.outcome = outcome(closed.status);
            self.lifecycle = final_lifecycle;
            self.backend.take();
            if closed.output_incomplete {
                if let Err(error) = self.monitors.raw_gap(self.context()) {
                    monitor_error.get_or_insert(error);
                }
            } else if self.monitors.needs_screen()
                && let Ok(screen) = self.history.screen()
                && let Err(error) = self.monitors.screen(&screen, self.context())
            {
                monitor_error.get_or_insert(error);
            }
            if let Err(error) = self.monitors.end_session(self.outcome, self.context()) {
                self.monitor_notifications_incomplete = true;
                self.monitors.quiesce();
                monitor_error.get_or_insert(error);
            }
        } else {
            self.lose();
        }
        self.monitor_notifications_incomplete |= monitor_error.is_some() || history_error.is_some();
        if let Some(error) = history_error {
            return Err(error.into());
        }
        if let Some(error) = monitor_error {
            return Err(error.into());
        }
        result
            .map(|_| ())
            .map_err(|()| TerminalSessionError::Native)
    }
    #[cfg(test)]
    pub(crate) fn monitor(
        &mut self,
        owner: &BackgroundOutputOwner,
        operation: TerminalMonitorOperation,
        activation: TerminalMonitorActivation,
        now_ms: i64,
    ) -> Result<TerminalMonitorMutation> {
        self.monitor_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            operation,
            activation,
            now_ms,
        )
    }

    pub(crate) fn monitor_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        operation: TerminalMonitorOperation,
        activation: TerminalMonitorActivation,
        now_ms: i64,
    ) -> Result<TerminalMonitorMutation> {
        self.authorize(owner)?;
        self.check_time(now_ms)?;
        if !matches!(
            self.lifecycle,
            TerminalLifecycle::Starting | TerminalLifecycle::Running
        ) {
            return Err(TerminalSessionError::InvalidState);
        }
        let mut context = self.context();
        context.now_ms = now_ms;
        let mutation = self
            .monitors
            .apply_with_activation(operation, activation, context)?;
        self.now_ms = now_ms;
        self.persist_with(persistence)?;
        Ok(mutation)
    }
    #[cfg(test)]
    pub(crate) fn complete_probe(
        &mut self,
        evidence: TerminalProbeEvidence,
        now_ms: i64,
    ) -> Result<bool> {
        self.complete_probe_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            evidence,
            now_ms,
        )
    }

    pub(crate) fn complete_probe_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        evidence: TerminalProbeEvidence,
        now_ms: i64,
    ) -> Result<bool> {
        if !matches!(
            self.lifecycle,
            TerminalLifecycle::Starting | TerminalLifecycle::Running
        ) {
            return Ok(false);
        }
        let mut context = self.context();
        context.now_ms = now_ms;
        let mut candidate = self.monitors.clone();
        let accepted = candidate
            .complete_probe(evidence, context)
            .map_err(|error| {
                if error == TerminalMonitorError::Clock {
                    TerminalSessionError::Clock
                } else {
                    TerminalSessionError::Monitor(error)
                }
            })?;
        if accepted {
            self.check_time(now_ms)?;
            self.monitors = candidate;
            self.now_ms = now_ms;
            self.persist_with(persistence)?;
        }
        Ok(accepted)
    }
    #[cfg(test)]
    pub(crate) fn events(
        &mut self,
        owner: &BackgroundOutputOwner,
        query: &TerminalEventQuery,
    ) -> Result<Vec<TerminalMonitorEvent>> {
        self.events_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            query,
        )
    }

    pub(crate) fn events_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        query: &TerminalEventQuery,
    ) -> Result<Vec<TerminalMonitorEvent>> {
        self.authorize(owner)?;
        if query
            .acknowledge_event_id
            .is_none_or(|ack| ack <= self.monitors.acknowledged_event_id())
        {
            return Ok(self.monitors.events(query)?);
        }
        let mut candidate = self.monitors.clone();
        let events = candidate.events(query)?;
        if candidate.acknowledged_event_id() != self.monitors.acknowledged_event_id() {
            if let Err(error) = self.persist_pending_output_gap(persistence) {
                self.publication_error = Some(error);
                self.failed_publication();
                return Err(error);
            }
            let publication = (|| -> Result<()> {
                candidate.checkpoint_context(self.context())?;
                let bytes = self.facts()?.encode(&candidate)?;
                self.history.publish_state_with(persistence, &bytes)?;
                Ok(())
            })();
            if let Err(error) = publication {
                if matches!(
                    error,
                    TerminalSessionError::History(TerminalHistoryError::Accounting(_))
                ) {
                    // State publication committed this acknowledgement even
                    // though its accounting failed. Keep that receipt in memory.
                    self.monitors = candidate;
                }
                self.publication_error = Some(error);
                self.failed_publication();
                return Err(error);
            }
            self.publication_error = None;
            self.monitors = candidate;
        }
        Ok(events)
    }
    fn facts(&self) -> Result<TerminalSessionFacts> {
        let mut facts = TerminalSessionFacts::new(
            self.history.session_id().clone(),
            &self.owner,
            self.context(),
            self.created_at_ms,
            self.last_output_ms,
            self.outcome,
        )?;
        facts.monitor_notifications_incomplete = self.monitor_notifications_incomplete;
        facts.metadata = Some(self.metadata.clone());
        facts.startup_stage = Some(self.startup_stage);
        facts
            .command_start_cursor
            .clone_from(&self.command_start_cursor);
        facts.attention = if matches!(
            self.lifecycle,
            TerminalLifecycle::Starting | TerminalLifecycle::Running
        ) {
            self.attention.clone()
        } else {
            TerminalAttentionState::default()
        };
        Ok(facts)
    }
    #[cfg(test)]
    fn persist(&mut self) -> Result<()> {
        self.persist_with(&mut crate::terminal_profile::TerminalTestPersistence)
    }

    fn persist_with(&mut self, persistence: &mut dyn TerminalJournalPersistence) -> Result<()> {
        let result = (|| {
            self.persist_pending_output_gap(persistence)?;
            self.monitors.checkpoint_context(self.context())?;
            let bytes = self.facts()?.encode(&self.monitors)?;
            self.history.publish_state_with(persistence, &bytes)?;
            Ok(())
        })();
        self.publication_error = result.err();
        if self.publication_error.is_some() {
            self.failed_publication();
        }
        result
    }
    fn persist_pending_output_gap(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
    ) -> Result<()> {
        if self.pending_output_gap {
            let result = self.history.mark_output_gap_with(persistence);
            if result.is_ok() || matches!(result, Err(TerminalHistoryError::Accounting(_))) {
                // Accounting failure does not revoke the committed barrier.
                self.pending_output_gap = false;
            }
            result?;
        }
        Ok(())
    }
    fn failed_observation_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        error: TerminalSessionError,
    ) -> TerminalSessionError {
        if matches!(error, TerminalSessionError::Monitor(_)) {
            self.monitor_notifications_incomplete = true;
        }
        self.failed_publication();
        let _ = self.persist_with(persistence);
        // A successful best-effort state write cannot erase a failed output
        // accounting obligation or a rejected consumed-output publication.
        if self.publication_error.is_none()
            && matches!(
                error,
                TerminalSessionError::History(
                    TerminalHistoryError::Accounting(_) | TerminalHistoryError::Profile(_)
                )
            )
        {
            self.publication_error = Some(error);
        }
        error
    }
    fn failed_publication(&mut self) {
        if self.backend.is_some() {
            self.lose();
        } else {
            self.clear_input_authority();
            self.monitors.quiesce();
        }
    }
    fn authorize(&self, owner: &BackgroundOutputOwner) -> Result<()> {
        if owner == &self.owner {
            Ok(())
        } else {
            Err(TerminalSessionError::NotFound)
        }
    }
    fn authorize_running(&self, owner: &BackgroundOutputOwner, now_ms: i64) -> Result<()> {
        self.authorize(owner)?;
        self.check_time(now_ms)?;
        // A control action cannot overtake an admitted partial payload or a
        // cursor reply. The host keeps pumping and retries this bounded Busy.
        if self.input.has_pending_bytes() {
            return Err(TerminalInputError::Busy.into());
        }
        if self.lifecycle == TerminalLifecycle::Running {
            Ok(())
        } else {
            Err(TerminalSessionError::InvalidState)
        }
    }
    fn check_time(&self, now_ms: i64) -> Result<()> {
        if now_ms < self.now_ms || now_ms < 0 {
            Err(TerminalSessionError::Clock)
        } else {
            Ok(())
        }
    }
    fn flush_input(&mut self) {
        if let Some(backend) = &mut self.backend {
            self.input
                .flush_with_limit(backend.input_write_limit(), |bytes, paste| {
                    backend.write_with_paste(bytes, paste)
                });
        }
        if self.input.is_quiesced() {
            self.attention = TerminalAttentionState::default();
            self.attention_writer = None;
        }
    }
    fn step(&self, output: Vec<u8>, probes: Vec<TerminalProbeRequest>) -> TerminalSessionStep {
        TerminalSessionStep {
            output,
            cursor: self.history.latest(),
            probes,
            lifecycle: self.lifecycle,
            cleanup_needed: false,
        }
    }
    fn cleanup_step(&mut self, output: Vec<u8>) -> TerminalSessionStep {
        self.clear_input_authority();
        let mut step = self.step(output, Vec::new());
        step.cleanup_needed = true;
        step
    }
    fn lose(&mut self) {
        self.clear_input_authority();
        self.lifecycle = TerminalLifecycle::Lost;
        self.outcome = None;
        if self.monitors.end_session(None, self.context()).is_err() {
            self.monitor_notifications_incomplete = true;
        }
        self.monitors.quiesce();
    }
}
fn actor_lease(actor: TerminalActorRole) -> TerminalWriteLease {
    match actor {
        TerminalActorRole::Human => TerminalWriteLease::Human,
        TerminalActorRole::Agent => TerminalWriteLease::Agent,
    }
}
fn require_closed_status(closed: TerminalPtyClose) -> std::result::Result<TerminalPtyClose, ()> {
    if closed.status == TerminalPtyStatus::Running {
        Err(())
    } else {
        Ok(closed)
    }
}
fn outcome(status: TerminalPtyStatus) -> Option<TerminalProcessOutcome> {
    match status {
        TerminalPtyStatus::Running => None,
        TerminalPtyStatus::Exited(code) => Some(TerminalProcessOutcome::Exited(code)),
        TerminalPtyStatus::Signalled(signal) => Some(TerminalProcessOutcome::Signaled(signal)),
    }
}

/// Owner-authorized durable observations with no native backend or input path.
/// Native recovery marks a formerly live host record lost and revokes pending
/// monitor probes. It does not probe, signal, or reconnect a persisted PID.
pub(crate) struct TerminalRecoveredSession {
    history: TerminalHistory,
    facts: TerminalSessionFacts,
    monitors: TerminalMonitorSet,
    publication_error: Option<TerminalSessionError>,
}
impl fmt::Debug for TerminalRecoveredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalRecoveredSession")
            .finish_non_exhaustive()
    }
}
impl TerminalRecoveredSession {
    /// Reconcile a failed recovered-history publication without replaying an
    /// uncommitted request or acquiring live backend authority.
    pub(crate) fn retry_publication_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
    ) -> Result<()> {
        self.authorize(owner)?;
        if self.publication_error.is_none() {
            return Ok(());
        }
        let result = (|| {
            self.history.prepare_public_facts_with(persistence)?;
            self.history
                .publish_state_with(persistence, &self.facts.encode(&self.monitors)?)?;
            Ok(())
        })();
        self.publication_error = result.as_ref().err().copied();
        result
    }

    #[cfg(test)]
    pub(crate) fn recover(
        history: TerminalHistory,
        owner: &BackgroundOutputOwner,
        now_ms: i64,
    ) -> Result<Self> {
        Self::recover_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            history,
            owner,
            now_ms,
        )
    }

    pub(crate) fn recover_with(
        persistence: &mut dyn TerminalJournalPersistence,
        mut history: TerminalHistory,
        owner: &BackgroundOutputOwner,
        now_ms: i64,
    ) -> Result<Self> {
        if history.require_live().is_ok() {
            return Err(TerminalSessionError::InvalidState);
        }
        let state = history
            .load_state()?
            .ok_or(TerminalSessionError::NotFound)?;
        let (mut facts, monitor_bytes) =
            TerminalSessionFacts::decode(&state.bytes, history.session_id(), &state.source)?;
        if !facts.owned_by(owner) {
            return Err(TerminalSessionError::NotFound);
        }
        if now_ms < facts.context.now_ms {
            return Err(TerminalSessionError::Clock);
        }
        let mut monitors = facts.restore_monitors(monitor_bytes)?;
        let latest = history.latest();
        if state.source > latest {
            return Err(TerminalSessionRecordError::Invalid.into());
        }
        let incomplete = state.source < latest;
        if incomplete {
            facts.observation_gap = Some(
                TerminalGap::new(state.source, latest.clone())
                    .map_err(|_| TerminalSessionRecordError::Invalid)?,
            );
            facts.context.cursor = latest;
        }
        let was_live = matches!(
            facts.context.lifecycle,
            TerminalLifecycle::Starting | TerminalLifecycle::Running,
        );
        if was_live || incomplete {
            facts.context.now_ms = now_ms;
        }
        if was_live {
            facts.context.lifecycle = TerminalLifecycle::Lost;
            facts.outcome = None;
            facts.attention = machine_god_core::TerminalAttentionState::default();
            match monitors.end_session(None, facts.context.clone()) {
                Ok(()) => {}
                Err(TerminalMonitorError::Counter) => {
                    // Exhausted notification identities cannot retain effect
                    // authority or make otherwise intact history unreadable.
                    monitors.quiesce();
                    monitors.checkpoint_context(facts.context.clone())?;
                    facts.monitor_notifications_incomplete = true;
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            monitors.checkpoint_context(facts.context.clone())?;
        }
        if was_live || incomplete {
            history.publish_state_with(persistence, &facts.encode(&monitors)?)?;
        }
        Ok(Self {
            history,
            facts,
            monitors,
            publication_error: None,
        })
    }

    pub(crate) fn publication_error(&self) -> Option<TerminalSessionError> {
        self.publication_error
    }

    pub(crate) fn facts(&self, owner: &BackgroundOutputOwner) -> Result<&TerminalSessionFacts> {
        self.authorize(owner)?;
        Ok(&self.facts)
    }

    pub(crate) fn public_facts(
        &self,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        controls: &TerminalAllowedControls,
    ) -> Result<machine_god_core::TerminalSessionFacts> {
        self.authorize(owner)?;
        project_facts(&self.facts, &self.history, &self.monitors, actor, controls)
    }

    pub(crate) fn prepare_public_facts_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
    ) -> Result<()> {
        self.authorize(owner)?;
        self.facts
            .metadata
            .as_ref()
            .ok_or(TerminalSessionError::InvalidState)?;
        let result = self
            .history
            .prepare_public_facts_with(persistence)
            .map_err(TerminalSessionError::from);
        if let Err(error) = result {
            self.publication_error = Some(error);
        }
        result
    }

    pub(crate) fn inspect_result_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        actor: TerminalActorRole,
        query: &TerminalEventQuery,
        controls: &TerminalAllowedControls,
    ) -> Result<TerminalActionResult> {
        self.authorize(owner)?;
        query
            .validate()
            .map_err(|_| TerminalMonitorError::Invalid)?;
        self.facts
            .metadata
            .as_ref()
            .ok_or(TerminalSessionError::InvalidState)?;
        self.prepare_public_facts_with(persistence, owner)?;
        self.public_facts(owner, actor, controls)?;
        let events = self.events_with(persistence, owner, query)?;
        project_inspect(
            self.public_facts(owner, actor, controls)?,
            self.facts
                .metadata
                .as_ref()
                .ok_or(TerminalSessionError::InvalidState)?,
            &self.monitors,
            events,
        )
    }

    pub(crate) fn checkpoint_reserve_bytes(&self) -> usize {
        self.history.checkpoint_reserve_bytes()
    }

    pub(crate) fn retire_completed_checkpoint_reserve_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
    ) -> Result<()> {
        self.authorize_retention(owner, TerminalHistoryEviction::CompletedOutput)?;
        if self.facts.context.cursor != self.history.latest() {
            return Err(TerminalSessionError::InvalidState);
        }
        let result = self
            .history
            .retire_completed_checkpoint_reserve_with(persistence)
            .map_err(TerminalSessionError::from);
        self.publication_error = result.as_ref().err().copied();
        result
    }

    pub(crate) fn physical_usage(
        &self,
        owner: &BackgroundOutputOwner,
    ) -> Result<TerminalJournalPhysicalUsage> {
        self.authorize(owner)?;
        Ok(self.history.physical_usage()?)
    }
    fn authorize_retention(
        &self,
        owner: &BackgroundOutputOwner,
        kind: TerminalHistoryEviction,
    ) -> Result<()> {
        self.authorize(owner)?;
        if let Some(error) = self.publication_error {
            return Err(error);
        }
        if kind == TerminalHistoryEviction::LiveCoveredOutput
            || !matches!(
                self.facts.context.lifecycle,
                TerminalLifecycle::Closed | TerminalLifecycle::Exited
            )
        {
            return Err(TerminalSessionError::InvalidState);
        }
        Ok(())
    }
    pub(crate) fn eviction_bytes(
        &self,
        owner: &BackgroundOutputOwner,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        self.authorize_retention(owner, kind)?;
        Ok(self.history.eviction_bytes(kind)?)
    }
    #[cfg(test)]
    pub(crate) fn evict(
        &mut self,
        owner: &BackgroundOutputOwner,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        self.evict_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            kind,
        )
    }

    pub(crate) fn evict_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        kind: TerminalHistoryEviction,
    ) -> Result<usize> {
        self.authorize_retention(owner, kind)?;
        let result = self
            .history
            .evict_with(persistence, kind)
            .map_err(TerminalSessionError::from);
        self.publication_error = result.as_ref().err().copied();
        result
    }

    pub(crate) fn read(
        &self,
        owner: &BackgroundOutputOwner,
        cursor: &TerminalCursor,
        maximum: usize,
    ) -> Result<TerminalJournalPage> {
        self.authorize(owner)?;
        Ok(self.history.read(cursor, maximum)?)
    }

    pub(crate) fn screen(&self, owner: &BackgroundOutputOwner) -> Result<TerminalScreen> {
        self.authorize(owner)?;
        Ok(self.history.screen()?)
    }

    #[cfg(test)]
    pub(crate) fn events(
        &mut self,
        owner: &BackgroundOutputOwner,
        query: &TerminalEventQuery,
    ) -> Result<Vec<TerminalMonitorEvent>> {
        self.events_with(
            &mut crate::terminal_profile::TerminalTestPersistence,
            owner,
            query,
        )
    }

    pub(crate) fn events_with(
        &mut self,
        persistence: &mut dyn TerminalJournalPersistence,
        owner: &BackgroundOutputOwner,
        query: &TerminalEventQuery,
    ) -> Result<Vec<TerminalMonitorEvent>> {
        self.authorize(owner)?;
        if query
            .acknowledge_event_id
            .is_none_or(|ack| ack <= self.monitors.acknowledged_event_id())
        {
            return Ok(self.monitors.events(query)?);
        }
        let mut candidate = self.monitors.clone();
        let events = candidate.events(query)?;
        if candidate.acknowledged_event_id() != self.monitors.acknowledged_event_id() {
            let published = self
                .history
                .publish_state_with(persistence, &self.facts.encode(&candidate)?);
            if let Err(error) = published {
                if matches!(error, TerminalHistoryError::Accounting(_)) {
                    self.monitors = candidate;
                }
                self.publication_error = Some(error.into());
                return Err(error.into());
            }
            self.publication_error = None;
            self.monitors = candidate;
        }
        Ok(events)
    }

    fn authorize(&self, owner: &BackgroundOutputOwner) -> Result<()> {
        if self.facts.owned_by(owner) {
            Ok(())
        } else {
            Err(TerminalSessionError::NotFound)
        }
    }
}

fn project_facts(
    facts: &TerminalSessionFacts,
    history: &TerminalHistory,
    monitors: &TerminalMonitorSet,
    actor: TerminalActorRole,
    controls: &TerminalAllowedControls,
) -> Result<machine_god_core::TerminalSessionFacts> {
    let metadata = facts
        .metadata
        .as_ref()
        .ok_or(TerminalSessionError::InvalidState)?;
    let latest = history.latest();
    let earliest = history.earliest();
    let origin = TerminalCursor::new(1, 0).map_err(|_| TerminalSessionError::InvalidState)?;
    let unread_range = if earliest < latest && earliest.segment() == latest.segment() {
        Some(machine_god_core::TerminalRawRange {
            start: earliest.clone(),
            end: latest.clone(),
        })
    } else {
        None
    };
    // Retained-prefix absence is an output fact, unlike observation_gap (which
    // only describes monitor observation). No read is issued merely to inspect.
    let raw_gap = if origin < earliest {
        Some(TerminalGap::new(origin, earliest).map_err(|_| TerminalSessionError::InvalidState)?)
    } else {
        None
    };
    let live = matches!(
        facts.context.lifecycle,
        TerminalLifecycle::Starting | TerminalLifecycle::Running
    );
    let mut next_actions = controls.clone();
    next_actions.write &= live
        && (facts.attention.write_lease() == TerminalWriteLease::None
            || facts.attention.write_lease() == actor_lease(actor));
    next_actions.resize &= live;
    next_actions.signal &= live;
    next_actions.wait &= facts.context.lifecycle != TerminalLifecycle::Closed;
    next_actions.close &= facts.context.lifecycle != TerminalLifecycle::Closed;
    // Matches pinned lifecycle_controls; monitor operation authorization is
    // separate from this descriptive next-action mask.
    next_actions.monitor = false;
    let projected = machine_god_core::TerminalSessionFacts {
        session_id: facts.session_id.clone(),
        lifecycle: facts.context.lifecycle,
        attention: facts.attention.clone(),
        backend: metadata.backend,
        persistence: machine_god_core::TerminalPersistenceLevel::Durable,
        output_cursor: latest,
        unread_range,
        raw_gap,
        screen_recovery: history.screen_recovery()?,
        active_monitor_count: u16::try_from(monitors.len())
            .map_err(|_| TerminalSessionError::InvalidState)?,
        next_actions,
    };
    projected
        .validate()
        .map_err(|_| TerminalSessionError::InvalidState)?;
    Ok(projected)
}

fn project_inspect(
    session: machine_god_core::TerminalSessionFacts,
    metadata: &TerminalSessionMetadata,
    monitors: &TerminalMonitorSet,
    events: Vec<TerminalMonitorEvent>,
) -> Result<TerminalActionResult> {
    let result = TerminalActionResult::Inspect {
        session,
        shell: metadata.shell.clone(),
        cwd: metadata.cwd.clone(),
        command: metadata.command.clone(),
        monitors: monitors.summaries(),
        events,
        event_gap_through: monitors.dropped_through_event_id(),
        next_event_id: monitors.next_event_id(),
    };
    result
        .validate()
        .map_err(|_| TerminalSessionError::InvalidState)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background_input::BackgroundInputStatus;
    use crate::terminal_input::TerminalInputProgress;
    use crate::terminal_journal::{TerminalJournal, TerminalJournalLimits};
    use crate::terminal_journal::{
        TerminalJournalError, TerminalJournalMutation, TerminalJournalReceipt,
    };
    use crate::terminal_monitor::{
        TerminalProbeObservation, TerminalWaitOutcome, TerminalWaitState,
    };
    use crate::terminal_profile::{
        TerminalProfileCompletion, TerminalProfileError, TerminalTestPersistence,
    };
    use machine_god_core::{
        SessionId, SessionIncarnationId, TerminalMonitorCondition as Condition,
        TerminalMonitorDefinition, TerminalMonitorLifetime, TerminalMonitorState,
        TerminalNotifySchedule, TerminalReturnCondition, TerminalSchedule, TerminalWaitRequest,
        TerminalWriteLeaseIntent, TerminalWritePayload,
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
        reason = "independent backend fault injections"
    )]
    struct State {
        output: VecDeque<Vec<u8>>,
        tail: Vec<u8>,
        writes: Vec<u8>,
        paste_modes: Vec<bool>,
        write_limit: usize,
        write_closes: bool,
        status: TerminalPtyStatus,
        exit_after_read: Option<TerminalPtyStatus>,
        closes: usize,
        signals: Vec<TerminalSignal>,
        resizes: Vec<TerminalDimensions>,
        status_calls: usize,
        incomplete: bool,
        signal_flushes: bool,
        signal_fails: bool,
        close_fails: bool,
        echo_restore_fails: bool,
        echo_restore_calls: usize,
    }
    impl Default for State {
        fn default() -> Self {
            Self {
                output: VecDeque::new(),
                tail: Vec::new(),
                writes: Vec::new(),
                paste_modes: Vec::new(),
                write_limit: usize::MAX,
                write_closes: false,
                status: TerminalPtyStatus::Running,
                exit_after_read: None,
                closes: 0,
                signals: Vec::new(),
                resizes: Vec::new(),
                status_calls: 0,
                incomplete: false,
                signal_flushes: false,
                signal_fails: false,
                close_fails: false,
                echo_restore_fails: false,
                echo_restore_calls: 0,
            }
        }
    }
    struct Backend(Arc<Mutex<State>>);

    #[test]
    fn submitted_write_receipt_survives_failed_quiescence_publication() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let writer = TerminalWriterId::new(NonZeroU64::new(1).unwrap());
        session
            .write(
                &owner("owner"),
                writer,
                &TerminalWriteRequest {
                    lease: TerminalWriteLeaseIntent::Acquire,
                    payload: None,
                },
                false,
            )
            .unwrap();
        {
            let mut state = fixture.state.lock().unwrap();
            state.write_limit = 2;
            state.write_closes = true;
        }
        let mut persistence = Persistence {
            denied: true,
            ..Persistence::default()
        };
        let (receipt, error) = session
            .write_completion_with(
                &mut persistence,
                &owner("owner"),
                TerminalActorRole::Agent,
                writer,
                &TerminalWriteRequest {
                    lease: TerminalWriteLeaseIntent::Use,
                    payload: Some(TerminalWritePayload::Text {
                        text: "abcd".into(),
                    }),
                },
                false,
            )
            .unwrap();
        assert_eq!(receipt.accepted_bytes, 2);
        assert_eq!(receipt.progress, TerminalInputProgress::Closed);
        assert!(error.is_some());
        assert_eq!(fixture.state.lock().unwrap().writes, b"ab");
        assert_eq!(
            session
                .write_receipt(&owner("owner"), writer, receipt.operation_id.unwrap())
                .unwrap(),
            receipt
        );
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 1)
            .unwrap();
    }

    #[test]
    fn session_dispatch_preserves_paste_kind_through_backpressure() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        fixture.state.lock().unwrap().write_limit = 1;
        let writer = TerminalWriterId::new(NonZeroU64::new(1).unwrap());
        session
            .write(
                &owner("owner"),
                writer,
                &TerminalWriteRequest {
                    lease: TerminalWriteLeaseIntent::Acquire,
                    payload: None,
                },
                false,
            )
            .unwrap();
        session
            .write(
                &owner("owner"),
                writer,
                &TerminalWriteRequest {
                    lease: TerminalWriteLeaseIntent::Use,
                    payload: Some(TerminalWritePayload::Paste { text: "abc".into() }),
                },
                false,
            )
            .unwrap();
        session.pump(1).unwrap();
        session.pump(2).unwrap();
        assert_eq!(fixture.state.lock().unwrap().writes, b"abc");
        assert_eq!(
            fixture.state.lock().unwrap().paste_modes,
            [true, true, true]
        );
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 3)
            .unwrap();
    }

    #[test]
    fn command_start_is_distinct_durable_and_never_recovered_as_authority() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.metadata.command = Some("printf command".into());
        let mut persistence = TerminalTestPersistence;
        assert_eq!(
            session.command_started_with(&mut persistence, 0),
            Err(TerminalSessionError::InvalidState)
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"profile bytes".to_vec());
        session.pump(1).unwrap();
        session.shell_ready_with(&mut persistence, 2).unwrap();
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Starting);
        let state = session.history.load_state().unwrap().unwrap();
        let (facts, _) = TerminalSessionFacts::decode(&state.bytes, &id(), &state.source).unwrap();
        assert_eq!(facts.startup_stage, Some(TerminalStartupStage::ShellReady));
        assert_eq!(facts.command_start_cursor, None);
        assert_eq!(
            session.shell_ready_with(&mut persistence, 2),
            Err(TerminalSessionError::InvalidState)
        );
        session.command_started_with(&mut persistence, 3).unwrap();
        let state = session.history.load_state().unwrap().unwrap();
        let (facts, _) = TerminalSessionFacts::decode(&state.bytes, &id(), &state.source).unwrap();
        assert_eq!(
            facts.startup_stage,
            Some(TerminalStartupStage::CommandStarted)
        );
        assert_eq!(facts.command_start_cursor, Some(session.context().cursor));
        assert_eq!(facts.context.lifecycle, TerminalLifecycle::Running);
        assert_eq!(
            session.command_started_with(&mut persistence, 3),
            Err(TerminalSessionError::InvalidState)
        );
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 4)
            .unwrap();
        drop(session);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 4).unwrap();
        assert_eq!(
            recovered.facts(&owner("owner")).unwrap().startup_stage,
            Some(TerminalStartupStage::CommandStarted)
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
    }

    impl TerminalSessionBackend for Backend {
        fn write_with_paste(
            &mut self,
            bytes: &[u8],
            paste: bool,
        ) -> std::result::Result<BackgroundInputReceipt, ()> {
            self.0.lock().unwrap().paste_modes.push(paste);
            self.write(bytes)
        }
        fn restore_startup_echo(&mut self) -> std::result::Result<(), ()> {
            let mut state = self.0.lock().unwrap();
            state.echo_restore_calls += 1;
            if state.echo_restore_fails {
                Err(())
            } else {
                Ok(())
            }
        }
        fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            let mut state = self.0.lock().unwrap();
            let Some(mut bytes) = state.output.pop_front() else {
                return Ok(TerminalPtyRead {
                    bytes_read: 0,
                    closed: false,
                });
            };
            let count = buffer.len().min(bytes.len());
            buffer[..count].copy_from_slice(&bytes[..count]);
            if count < bytes.len() {
                bytes.drain(..count);
                state.output.push_front(bytes);
            }
            let exit_after_read = state.exit_after_read.take();
            if let Some(status) = exit_after_read {
                state.status = status;
            }
            Ok(TerminalPtyRead {
                bytes_read: count,
                closed: exit_after_read.is_some(),
            })
        }
        fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            let mut state = self.0.lock().unwrap();
            let count = bytes.len().min(state.write_limit);
            state.writes.extend_from_slice(&bytes[..count]);
            Ok(BackgroundInputReceipt::new(
                count,
                state.write_closes,
                if state.write_closes {
                    BackgroundInputStatus::Closed
                } else if count == bytes.len() {
                    BackgroundInputStatus::Written
                } else {
                    BackgroundInputStatus::Backpressure
                },
            ))
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            let mut state = self.0.lock().unwrap();
            state.status_calls += 1;
            Ok(state.status)
        }
        fn resize(&mut self, dimensions: &TerminalDimensions) -> std::result::Result<(), ()> {
            self.0.lock().unwrap().resizes.push(dimensions.clone());
            Ok(())
        }
        fn signal(&mut self, signal: TerminalSignal) -> std::result::Result<(), ()> {
            let mut state = self.0.lock().unwrap();
            state.signals.push(signal);
            if state.signal_fails { Err(()) } else { Ok(()) }
        }
        fn signal_may_discard_output(&self) -> bool {
            self.0.lock().unwrap().signal_flushes
        }
        fn close(
            &mut self,
            _: bool,
            output: &mut dyn FnMut(&[u8]),
        ) -> std::result::Result<TerminalPtyClose, ()> {
            let (tail, result) = {
                let mut state = self.0.lock().unwrap();
                state.closes += 1;
                let tail = std::mem::take(&mut state.tail);
                let result = if state.close_fails {
                    Err(())
                } else {
                    if state.status == TerminalPtyStatus::Running {
                        state.status = TerminalPtyStatus::Exited(0);
                    }
                    Ok(TerminalPtyClose {
                        status: state.status,
                        output_incomplete: state.incomplete,
                    })
                };
                (tail, result)
            };
            for chunk in tail.chunks(4096) {
                output(chunk);
            }
            result
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
                "machine-god-session-{:032x}",
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
        fn session(&self) -> TerminalSession<Backend> {
            self.session_with(&mut TerminalTestPersistence)
        }
        fn session_with(
            &self,
            persistence: &mut dyn TerminalJournalPersistence,
        ) -> TerminalSession<Backend> {
            let journal =
                TerminalJournal::create(self.fd(), id(), TerminalJournalLimits::default()).unwrap();
            let history = TerminalHistory::create_with(
                persistence,
                journal,
                &TerminalDimensions::new(3, 20).unwrap(),
            )
            .unwrap();
            TerminalSession::new_with(
                persistence,
                Backend(Arc::clone(&self.state)),
                history,
                owner("owner"),
                id(),
                crate::terminal_session_record::test_metadata(),
                0,
            )
            .unwrap()
        }
        fn block_publication(&self) -> OwnedFd {
            rustix::fs::openat(
                self.fd(),
                "tj-meta.tmp",
                OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::from_bits_retain(0o600),
            )
            .unwrap()
        }
        fn recover(&self) -> TerminalHistory {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                match TerminalJournal::open_existing(
                    self.fd(),
                    &id(),
                    TerminalJournalLimits::default(),
                ) {
                    Err(crate::terminal_journal::TerminalJournalError::Busy)
                        if std::time::Instant::now() < deadline =>
                    {
                        // A parallel fork/exec may transiently retain the
                        // just-dropped flock; production acquisition stays try-only.
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    result => return TerminalHistory::recover(result.unwrap()).unwrap(),
                }
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn id() -> TerminalSessionId {
        TerminalSessionId::new("session-test").unwrap()
    }
    fn owner(incarnation: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new("same-session").unwrap(),
            SessionIncarnationId::new(incarnation).unwrap(),
        )
    }
    fn writer() -> TerminalWriterId {
        TerminalWriterId::new(NonZeroU64::new(1).unwrap())
    }
    fn text(value: &str) -> TerminalWriteRequest {
        TerminalWriteRequest {
            lease: TerminalWriteLeaseIntent::Use,
            payload: Some(TerminalWritePayload::Text { text: value.into() }),
        }
    }
    fn acquire(session: &mut TerminalSession<Backend>) {
        session
            .write(
                &owner("owner"),
                writer(),
                &TerminalWriteRequest {
                    lease: TerminalWriteLeaseIntent::Acquire,
                    payload: None,
                },
                false,
            )
            .unwrap();
    }
    fn query() -> TerminalEventQuery {
        TerminalEventQuery {
            after_event_id: 0,
            acknowledge_event_id: None,
            max_events: 256,
        }
    }
    fn add(
        session: &mut TerminalSession<Backend>,
        condition: Condition,
    ) -> machine_god_core::TerminalMonitorId {
        let check_schedule = condition
            .requires_polling()
            .then_some(TerminalSchedule { interval_ms: 10 });
        session
            .monitor(
                &owner("owner"),
                TerminalMonitorOperation::Add {
                    definition: TerminalMonitorDefinition {
                        condition,
                        check_schedule,
                        notify: TerminalNotifySchedule::OnMatch,
                        lifetime: TerminalMonitorLifetime::UntilSessionEnd,
                    },
                },
                TerminalMonitorActivation::default(),
                session.now_ms,
            )
            .unwrap()
            .monitor_id
    }

    #[derive(Default)]
    struct Persistence {
        calls: Vec<&'static str>,
        denied: bool,
        denied_call: Option<usize>,
        fail_accounting: Option<&'static str>,
        remaining_appends: Option<usize>,
    }
    impl TerminalJournalPersistence for Persistence {
        fn mutate(
            &mut self,
            journal: &mut TerminalJournal,
            mutation: TerminalJournalMutation<'_>,
        ) -> std::result::Result<
            TerminalProfileCompletion<TerminalJournalReceipt, TerminalJournalError>,
            TerminalProfileError,
        > {
            let kind = match &mutation {
                TerminalJournalMutation::Append(_) => "append",
                TerminalJournalMutation::Checkpoint { .. } => "checkpoint",
                TerminalJournalMutation::CheckpointReserve(_) => "reserve",
                TerminalJournalMutation::State { .. } => "state",
                TerminalJournalMutation::Event(_) => "event",
                TerminalJournalMutation::Acknowledge(_) => "ack",
                TerminalJournalMutation::Evict(_) => "evict",
            };
            self.calls.push(kind);
            if self.denied || self.denied_call == Some(self.calls.len()) {
                return Err(TerminalProfileError::ResourceLimit);
            }
            if kind == "append"
                && let Some(remaining) = &mut self.remaining_appends
            {
                *remaining = remaining
                    .checked_sub(1)
                    .ok_or(TerminalProfileError::ResourceLimit)?;
            }
            let mut completion = TerminalTestPersistence.mutate(journal, mutation)?;
            if self.fail_accounting == Some(kind) && completion.operation.is_ok() {
                self.fail_accounting = None;
                completion.accounting = Err(TerminalProfileError::AccountingMismatch);
            }
            Ok(completion)
        }
    }

    fn denied_error() -> TerminalSessionError {
        TerminalHistoryError::Profile(TerminalProfileError::ResourceLimit).into()
    }

    fn lease_request(lease: TerminalWriteLeaseIntent) -> TerminalWriteRequest {
        TerminalWriteRequest {
            lease,
            payload: None,
        }
    }

    #[test]
    fn startup_echo_restoration_is_session_owned_and_failure_clears_authority() {
        for fails in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            fixture.state.lock().unwrap().echo_restore_fails = fails;
            let result = session.shell_ready(0);
            assert_eq!(result.is_err(), fails);
            assert_eq!(fixture.state.lock().unwrap().echo_restore_calls, 1);
            assert_eq!(
                session.lifecycle,
                if fails {
                    TerminalLifecycle::Lost
                } else {
                    TerminalLifecycle::Running
                }
            );
            assert_eq!(session.attention, TerminalAttentionState::default());
            assert!(session.owns_backend());
            assert_eq!(session.input.is_quiesced(), fails);
        }
    }

    #[test]
    fn actor_bound_attention_cancellation_preserves_partial_payload_and_receipt() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let mut persistence = Persistence::default();
        let agent = TerminalActorRole::Agent;
        let human = TerminalActorRole::Human;
        let other = TerminalWriterId::new(NonZeroU64::new(2).unwrap());
        acquire(&mut session);
        fixture.state.lock().unwrap().write_limit = 2;
        let receipt = session
            .write_with(
                &mut persistence,
                &owner("owner"),
                agent,
                writer(),
                &text("abcdef"),
                false,
            )
            .unwrap();
        let operation = receipt.operation_id.unwrap();
        assert_eq!(receipt.accepted_bytes, 2);
        session
            .begin_attention_with(&mut persistence, &owner("owner"), agent, writer(), 1)
            .unwrap();
        assert!(matches!(
            session.cancel_attention_with(&mut persistence, &owner("owner"), agent, other, 1),
            Err(TerminalSessionError::Input(
                TerminalInputError::LeaseConflict
            ))
        ));
        assert!(matches!(
            session.write_receipt_with_actor(&owner("owner"), human, writer(), operation),
            Err(TerminalSessionError::Input(TerminalInputError::NotFound))
        ));
        session
            .cancel_attention_with(&mut persistence, &owner("owner"), human, writer(), 1)
            .unwrap();
        assert_eq!(
            session
                .inspect(&owner("owner"))
                .unwrap()
                .attention
                .attention(),
            TerminalAttention::AgentWait
        );
        session
            .cancel_attention_with(&mut persistence, &owner("owner"), agent, writer(), 2)
            .unwrap();
        assert_eq!(session.attention, TerminalAttentionState::default());
        assert_eq!(session.lifecycle, TerminalLifecycle::Running);
        session
            .write_with(
                &mut persistence,
                &owner("owner"),
                human,
                other,
                &lease_request(TerminalWriteLeaseIntent::Acquire),
                false,
            )
            .unwrap();
        assert!(matches!(
            session.write_with(
                &mut persistence,
                &owner("owner"),
                human,
                other,
                &text("X"),
                false
            ),
            Err(TerminalSessionError::Input(TerminalInputError::Busy))
        ));
        fixture.state.lock().unwrap().write_limit = usize::MAX;
        session.pump(3).unwrap();
        assert_eq!(fixture.state.lock().unwrap().writes, b"abcdef");
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        let finished = session
            .write_receipt_with_actor(&owner("owner"), agent, writer(), operation)
            .unwrap();
        assert_eq!(finished.accepted_bytes, 6);
        assert_eq!(finished.progress, TerminalInputProgress::Complete);
        assert_eq!(
            session
                .inspect(&owner("owner"))
                .unwrap()
                .attention
                .attention(),
            TerminalAttention::UserTakeover
        );
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 4)
            .unwrap();
        assert_eq!(session.attention, TerminalAttentionState::default());
    }

    #[test]
    fn attention_is_durable_role_scoped_and_never_recovers_authority() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let mut persistence = Persistence::default();
        let agent = TerminalActorRole::Agent;
        let human = TerminalActorRole::Human;
        session
            .write_with(
                &mut persistence,
                &owner("owner"),
                human,
                writer(),
                &lease_request(TerminalWriteLeaseIntent::Acquire),
                false,
            )
            .unwrap();
        assert!(matches!(
            session.write_with(
                &mut persistence,
                &owner("owner"),
                agent,
                writer(),
                &lease_request(TerminalWriteLeaseIntent::Acquire),
                false
            ),
            Err(TerminalSessionError::Input(
                TerminalInputError::LeaseConflict
            ))
        ));
        assert!(matches!(
            session.begin_attention_with(&mut persistence, &owner("owner"), agent, writer(), 0),
            Err(TerminalSessionError::Input(
                TerminalInputError::LeaseConflict
            ))
        ));
        session
            .finish_attention_with(&mut persistence, &owner("owner"), human, writer(), 0, false)
            .unwrap();
        assert_eq!(session.attention_writer, Some((human, writer())));
        session.pump(1).unwrap();
        let state = session.history.load_state().unwrap().unwrap();
        let (decoded, _) =
            TerminalSessionFacts::decode(&state.bytes, &id(), &state.source).unwrap();
        assert_eq!(
            decoded.attention.attention(),
            TerminalAttention::UserTakeover
        );
        drop(session);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 2).unwrap();
        assert_eq!(
            recovered.facts(&owner("owner")).unwrap().attention,
            TerminalAttentionState::default()
        );
        assert_eq!(
            recovered.facts(&owner("owner")).unwrap().context.lifecycle,
            TerminalLifecycle::Lost
        );
    }

    #[test]
    fn normal_wait_finish_keeps_lease_but_revoke_quiesces_pending_input() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        acquire(&mut session);
        let mut persistence = Persistence::default();
        session
            .begin_attention_with(
                &mut persistence,
                &owner("owner"),
                TerminalActorRole::Agent,
                writer(),
                1,
            )
            .unwrap();
        session
            .finish_attention_with(
                &mut persistence,
                &owner("owner"),
                TerminalActorRole::Agent,
                writer(),
                2,
                false,
            )
            .unwrap();
        assert_eq!(session.attention.attention(), TerminalAttention::Background);
        assert_eq!(session.attention.write_lease(), TerminalWriteLease::Agent);
        fixture.state.lock().unwrap().write_limit = 1;
        let receipt = session
            .write(&owner("owner"), writer(), &text("abc"), false)
            .unwrap();
        session
            .write_with(
                &mut persistence,
                &owner("owner"),
                TerminalActorRole::Human,
                writer(),
                &lease_request(TerminalWriteLeaseIntent::Revoke),
                false,
            )
            .unwrap();
        assert_eq!(session.attention, TerminalAttentionState::default());
        assert!(session.input.is_quiesced());
        assert_eq!(session.lifecycle, TerminalLifecycle::Running);
        let receipt = session
            .write_receipt(&owner("owner"), writer(), receipt.operation_id.unwrap())
            .unwrap();
        assert_eq!(receipt.accepted_bytes, 1);
        assert_eq!(receipt.progress, TerminalInputProgress::Closed);
    }

    #[test]
    fn attention_publication_failure_never_grants_input_or_drops_accepted_count() {
        for accounting in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            acquire(&mut session);
            fixture.state.lock().unwrap().write_limit = 1;
            let receipt = session
                .write(&owner("owner"), writer(), &text("abc"), false)
                .unwrap();
            let mut persistence = Persistence {
                denied: !accounting,
                fail_accounting: accounting.then_some("state"),
                ..Persistence::default()
            };
            assert!(
                session
                    .cancel_attention_with(
                        &mut persistence,
                        &owner("owner"),
                        TerminalActorRole::Agent,
                        writer(),
                        1
                    )
                    .is_err()
            );
            assert_eq!(session.attention, TerminalAttentionState::default());
            assert_eq!(session.lifecycle, TerminalLifecycle::Lost);
            assert!(session.input.is_quiesced());
            assert!(session.owns_backend());
            let receipt = session
                .write_receipt(&owner("owner"), writer(), receipt.operation_id.unwrap())
                .unwrap();
            assert_eq!(receipt.accepted_bytes, 1);
            assert_eq!(fixture.state.lock().unwrap().closes, 0);
        }
    }

    #[test]
    fn wrong_incarnation_cannot_publish_attention_or_inspect_events() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let mut persistence = Persistence::default();
        assert!(matches!(
            session.begin_attention_with(
                &mut persistence,
                &owner("replacement"),
                TerminalActorRole::Agent,
                writer(),
                0
            ),
            Err(TerminalSessionError::NotFound)
        ));
        assert!(matches!(
            session.inspect_result_with(
                &mut persistence,
                &owner("replacement"),
                TerminalActorRole::Agent,
                &query(),
                &TerminalAllowedControls::default()
            ),
            Err(TerminalSessionError::NotFound)
        ));
        assert!(persistence.calls.is_empty());
        assert_eq!(fixture.state.lock().unwrap().writes, b"");
    }

    #[test]
    fn bounded_public_projection_preserves_metadata_monitors_and_event_continuation() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let monitor_id = add(
            &mut session,
            Condition::OutputContains {
                pattern: "ready".into(),
            },
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"ready".to_vec());
        session.pump(1).unwrap();
        let mut persistence = Persistence::default();
        let controls = TerminalAllowedControls {
            read: true,
            write: true,
            inspect: true,
            wait: true,
            ..TerminalAllowedControls::default()
        };
        let TerminalActionResult::Inspect {
            session: facts,
            monitors,
            events,
            shell,
            cwd,
            next_event_id,
            ..
        } = session
            .inspect_result_with(
                &mut persistence,
                &owner("owner"),
                TerminalActorRole::Agent,
                &query(),
                &controls,
            )
            .unwrap()
        else {
            panic!("inspect variant")
        };
        assert_eq!(facts.active_monitor_count, 1);
        assert_eq!(monitors[0].monitor_id, monitor_id);
        assert_eq!(shell, session.metadata.shell);
        assert_eq!(cwd, session.metadata.cwd);
        assert_eq!(next_event_id, events.last().unwrap().event_id + 1);
        assert!(facts.raw_gap.is_none());
        assert!(facts.unread_range.is_some());
        assert!(facts.next_actions.write);
        assert!(matches!(
            facts.screen_recovery,
            machine_god_core::TerminalScreenRecovery::Available { .. }
        ));
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 2)
            .unwrap();
        drop(session);
        let mut recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 2).unwrap();
        let result = recovered
            .inspect_result_with(
                &mut persistence,
                &owner("owner"),
                TerminalActorRole::Human,
                &query(),
                &controls,
            )
            .unwrap();
        let TerminalActionResult::Inspect { session: facts, .. } = result else {
            panic!("inspect variant")
        };
        assert!(!facts.next_actions.write);
        assert!(!facts.next_actions.wait);
        assert!(facts.next_actions.read);
        assert_eq!(facts.attention, TerminalAttentionState::default());
    }

    #[test]
    fn resize_registers_growth_before_effects_and_shrinks_only_after_new_checkpoint() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let initial = session.history.checkpoint_reserve_bytes();
        let mut persistence = Persistence::default();
        let larger = TerminalDimensions::new(24, 80).unwrap();
        session
            .resize_with(&mut persistence, &owner("owner"), &larger, 1)
            .unwrap();
        assert_eq!(
            persistence.calls,
            ["reserve", "checkpoint", "checkpoint", "state"]
        );
        let bound =
            crate::terminal_screen::TerminalScreenEngine::checkpoint_bound(&larger).unwrap() + 9;
        assert_eq!(session.history.checkpoint_reserve_bytes(), bound);
        assert!(bound > initial);
        persistence.calls.clear();
        session
            .resize_with(
                &mut persistence,
                &owner("owner"),
                &TerminalDimensions::new(3, 20).unwrap(),
                2,
            )
            .unwrap();
        assert_eq!(
            persistence.calls,
            ["checkpoint", "checkpoint", "reserve", "state"]
        );
        assert_eq!(session.history.checkpoint_reserve_bytes(), initial);
        assert_eq!(fixture.state.lock().unwrap().resizes.len(), 2);
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 3)
            .unwrap();
        assert_eq!(session.history.checkpoint_reserve_bytes(), 0);
    }

    #[test]
    fn final_facts_and_native_cleanup_both_precede_reserve_release() {
        for failure in ["native", "state", "reserve"] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            let reserved = session.history.checkpoint_reserve_bytes();
            assert!(reserved > 0);
            fixture.state.lock().unwrap().close_fails = failure == "native";
            let mut persistence = Persistence {
                // Close publishes barrier, final checkpoint, final facts, release.
                denied_call: match failure {
                    "state" => Some(3),
                    "reserve" => Some(4),
                    _ => None,
                },
                ..Persistence::default()
            };
            assert!(
                session
                    .close_with(
                        &mut persistence,
                        &owner("owner"),
                        TerminalClosePolicy::Force,
                        1
                    )
                    .is_err()
            );
            assert_eq!(session.history.checkpoint_reserve_bytes(), reserved);
            assert_eq!(session.owns_backend(), failure == "native");
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
            if failure == "reserve" {
                let stored = session.history.load_state().unwrap().unwrap();
                let (facts, _) =
                    TerminalSessionFacts::decode(&stored.bytes, &id(), &stored.source).unwrap();
                assert_eq!(facts.context.lifecycle, TerminalLifecycle::Closed);
            } else {
                assert!(!persistence.calls.contains(&"reserve"));
            }
            fixture.state.lock().unwrap().close_fails = false;
            persistence = Persistence::default();
            session
                .close_with(
                    &mut persistence,
                    &owner("owner"),
                    TerminalClosePolicy::Force,
                    2,
                )
                .unwrap();
            assert_eq!(session.history.checkpoint_reserve_bytes(), 0);
            assert_eq!(persistence.calls.last(), Some(&"reserve"));
            assert!(session.publication_error.is_none());
        }
    }

    #[test]
    fn contextless_cleanup_and_observation_only_recovery_retain_live_reserve() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let reserved = session.history.checkpoint_reserve_bytes();
        assert_eq!(
            session.teardown_without_persistence(true, 1, denied_error()),
            Err(denied_error())
        );
        assert!(!session.owns_backend());
        assert_eq!(session.history.checkpoint_reserve_bytes(), reserved);
        drop(session);
        let mut recovered = fixture.recover();
        assert_eq!(recovered.checkpoint_reserve_bytes(), reserved);
        assert_eq!(
            recovered.release_checkpoint_reserve_with(&mut TerminalTestPersistence),
            Err(TerminalHistoryError::ReadOnly)
        );
        assert_eq!(recovered.checkpoint_reserve_bytes(), reserved);
    }

    #[test]
    fn resize_admission_refusal_distinguishes_before_and_after_native_effects() {
        for denied_call in [1, 2, 3] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            let old_reserve = session.history.checkpoint_reserve_bytes();
            add(&mut session, Condition::ProcessExit);
            let metadata = std::fs::read(fixture.path.join("tj-meta")).unwrap();
            let monitors = session.monitors.snapshot().unwrap();
            let mut persistence = Persistence {
                denied_call: Some(denied_call),
                ..Persistence::default()
            };
            assert_eq!(
                session.resize_with(
                    &mut persistence,
                    &owner("owner"),
                    &TerminalDimensions::new(24, 80).unwrap(),
                    1,
                ),
                Err(denied_error())
            );
            assert!(session.owns_backend());
            if denied_call <= 2 {
                assert_eq!(persistence.calls, ["reserve", "checkpoint"][..denied_call]);
                assert!(fixture.state.lock().unwrap().resizes.is_empty());
                assert_eq!(session.lifecycle, TerminalLifecycle::Running);
                assert!(!session.input.is_quiesced());
                assert!(session.publication_error.is_none());
                assert_eq!(session.now_ms, 0);
                assert_eq!(session.monitors.snapshot().unwrap(), monitors);
                if denied_call == 1 {
                    assert_eq!(
                        std::fs::read(fixture.path.join("tj-meta")).unwrap(),
                        metadata
                    );
                    assert_eq!(session.history.checkpoint_reserve_bytes(), old_reserve);
                } else {
                    assert!(session.history.checkpoint_reserve_bytes() > old_reserve);
                }
                assert!(session.history.screen().is_ok());
            } else {
                assert_eq!(
                    persistence.calls,
                    ["reserve", "checkpoint", "checkpoint", "state"]
                );
                assert!(session.history.checkpoint_reserve_bytes() > old_reserve);
                assert_eq!(fixture.state.lock().unwrap().resizes.len(), 1);
                assert_eq!(session.lifecycle, TerminalLifecycle::Lost);
                assert!(session.input.is_quiesced());
                assert_eq!(session.monitors.len(), 0);
                // Even a successful lost-state write cannot erase the refused
                // post-resize checkpoint publication.
                assert_eq!(session.publication_error, Some(denied_error()));
                assert!(session.history.screen().is_err());
                let stored = session.history.load_state().unwrap().unwrap();
                let (facts, monitors) =
                    TerminalSessionFacts::decode(&stored.bytes, &id(), &stored.source).unwrap();
                assert_eq!(facts.context.lifecycle, TerminalLifecycle::Lost);
                assert_eq!(TerminalMonitorSet::restore(monitors).unwrap().len(), 0);
            }
            session
                .close(&owner("owner"), TerminalClosePolicy::Force, 2)
                .unwrap();
        }
    }

    #[test]
    fn resize_after_signal_gap_rejects_without_losing_the_running_session() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        add(&mut session, Condition::ProcessExit);
        fixture.state.lock().unwrap().signal_flushes = true;
        let mut persistence = Persistence::default();
        session
            .signal_with(
                &mut persistence,
                &owner("owner"),
                TerminalSignal::Interrupt,
                1,
            )
            .unwrap();
        let metadata = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        let monitors = session.monitors.snapshot().unwrap();
        persistence.calls.clear();
        assert!(matches!(
            session.resize_with(
                &mut persistence,
                &owner("owner"),
                &TerminalDimensions::new(4, 30).unwrap(),
                2,
            ),
            Err(TerminalSessionError::History(TerminalHistoryError::Screen(
                crate::terminal_screen::TerminalScreenError::Unavailable(_)
            )))
        ));
        assert!(persistence.calls.is_empty());
        assert_eq!(
            std::fs::read(fixture.path.join("tj-meta")).unwrap(),
            metadata
        );
        assert_eq!(session.monitors.snapshot().unwrap(), monitors);
        assert_eq!(session.lifecycle, TerminalLifecycle::Running);
        assert_eq!(session.now_ms, 1);
        assert!(!session.input.is_quiesced());
        assert!(session.publication_error.is_none());
        assert!(fixture.state.lock().unwrap().resizes.is_empty());
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"still running".to_vec());
        assert_eq!(
            session.pump_with(&mut persistence, 2).unwrap().output,
            b"still running"
        );
        assert_eq!(session.lifecycle, TerminalLifecycle::Running);
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 3)
            .unwrap();
    }

    #[test]
    fn failed_native_status_quiesces_and_only_publishes_with_explicit_authority() {
        for available in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            add(&mut session, Condition::ProcessExit);
            let before = session.history.load_state().unwrap().unwrap().bytes;
            let mut persistence = Persistence::default();
            let context =
                available.then_some(&mut persistence as &mut dyn TerminalJournalPersistence);
            assert!(matches!(
                session.native_status_failed_with(context, 1),
                Err(TerminalSessionError::Native)
            ));
            assert_eq!(session.lifecycle, TerminalLifecycle::Lost);
            assert!(session.input.is_quiesced());
            assert_eq!(session.monitors.len(), 0);
            assert!(session.owns_backend());
            assert_eq!(session.publication_error.is_some(), !available);
            assert_eq!(fixture.state.lock().unwrap().closes, 0);
            let stored = session.history.load_state().unwrap().unwrap();
            if available {
                assert_eq!(persistence.calls, ["state"]);
                let (facts, monitors) =
                    TerminalSessionFacts::decode(&stored.bytes, &id(), &stored.source).unwrap();
                assert_eq!(facts.context.lifecycle, TerminalLifecycle::Lost);
                assert_eq!(TerminalMonitorSet::restore(monitors).unwrap().len(), 0);
            } else {
                assert!(persistence.calls.is_empty());
                assert_eq!(stored.bytes, before);
            }
            session
                .close(&owner("owner"), TerminalClosePolicy::Force, 2)
                .unwrap();
        }
    }

    #[test]
    fn exit_racing_read_preflight_drains_large_tail_only_after_read_permit_release() {
        for exit_after_read in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            let tail = vec![b't'; 2 * MAX_MONITOR_FEED_BYTES + 7];
            fixture.state.lock().unwrap().tail = tail.clone();
            assert!(!session.needs_native_cleanup().unwrap());
            let prefix = if exit_after_read {
                let prefix = vec![b'p'; MAX_MONITOR_FEED_BYTES];
                let mut state = fixture.state.lock().unwrap();
                state.output.push_back(prefix.clone());
                state.exit_after_read = Some(TerminalPtyStatus::Exited(7));
                prefix
            } else {
                fixture.state.lock().unwrap().status = TerminalPtyStatus::Exited(7);
                Vec::new()
            };
            // A one-read permit would reject a second append; native close
            // delivers the >16 KiB tail in many chunks, outside that permit.
            let mut read_permit = Persistence {
                remaining_appends: Some(1),
                ..Persistence::default()
            };
            let step = session.pump_read_with(&mut read_permit, 1).unwrap();
            assert!(step.cleanup_needed);
            assert_eq!(step.output, prefix);
            assert_eq!(step.cursor.offset(), prefix.len() as u64);
            assert_eq!(fixture.state.lock().unwrap().closes, 0);
            assert_eq!(fixture.state.lock().unwrap().tail, tail);
            assert!(session.owns_backend());
            assert!(session.input.is_quiesced());
            assert_eq!(
                read_permit
                    .calls
                    .iter()
                    .filter(|kind| **kind == "append")
                    .count(),
                usize::from(exit_after_read)
            );
            assert!(!read_permit.calls.contains(&"checkpoint"));
            drop(read_permit);
            let mut ordinary = Persistence::default();
            let completed = session.pump_with(&mut ordinary, 1).unwrap();
            assert!(!completed.cleanup_needed);
            assert!(completed.output.is_empty());
            assert_eq!(completed.lifecycle, TerminalLifecycle::Exited);
            assert_eq!(session.outcome(), Some(TerminalProcessOutcome::Exited(7)));
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
            assert!(!session.owns_backend());
            assert!(!session.monitor_notifications_incomplete);
            assert!(session.publication_error().is_none());
            let mut expected = prefix;
            expected.extend_from_slice(&tail);
            assert_eq!(completed.cursor.offset(), expected.len() as u64);
            assert_eq!(
                ordinary
                    .calls
                    .iter()
                    .filter(|kind| **kind == "append")
                    .count(),
                tail.len().div_ceil(4096)
            );
            let page = session
                .read(
                    &owner("owner"),
                    &TerminalCursor::new(1, 0).unwrap(),
                    expected.len(),
                )
                .unwrap();
            assert_eq!(page.bytes, expected);
            drop(session);
            let recovered = TerminalRecoveredSession::recover_with(
                &mut ordinary,
                fixture.recover(),
                &owner("owner"),
                1,
            )
            .unwrap();
            assert_eq!(recovered.facts.context.lifecycle, TerminalLifecycle::Exited);
            assert_eq!(
                recovered.facts.context.cursor.offset(),
                expected.len() as u64
            );
            assert!(!recovered.facts.monitor_notifications_incomplete);
        }
    }

    #[test]
    fn cleanup_preflight_observes_status_without_consuming_output_or_publishing() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"pending".to_vec());
        let before = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        assert!(!session.needs_native_cleanup().unwrap());
        fixture.state.lock().unwrap().status = TerminalPtyStatus::Exited(7);
        assert!(session.needs_native_cleanup().unwrap());
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Starting);
        assert_eq!(session.outcome(), None);
        assert_eq!(fixture.state.lock().unwrap().output.len(), 1);
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        assert_eq!(std::fs::read(fixture.path.join("tj-meta")).unwrap(), before);
        session.lose();
        let calls = fixture.state.lock().unwrap().status_calls;
        assert!(session.needs_native_cleanup().unwrap());
        assert_eq!(fixture.state.lock().unwrap().status_calls, calls);
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 1)
            .unwrap();
        assert!(!session.needs_native_cleanup().unwrap());
        assert_eq!(session.outcome(), Some(TerminalProcessOutcome::Exited(7)));
    }

    #[test]
    fn explicit_context_routes_initialization_controls_output_and_close() {
        let fixture = Fixture::new();
        let mut persistence = Persistence::default();
        let mut session = fixture.session_with(&mut persistence);
        assert_eq!(persistence.calls, ["reserve", "checkpoint", "state"]);
        persistence.calls.clear();
        session.shell_ready_with(&mut persistence, 0).unwrap();
        session
            .resize_with(
                &mut persistence,
                &owner("owner"),
                &TerminalDimensions::new(4, 20).unwrap(),
                1,
            )
            .unwrap();
        fixture.state.lock().unwrap().signal_flushes = true;
        session
            .signal_with(
                &mut persistence,
                &owner("owner"),
                TerminalSignal::Interrupt,
                2,
            )
            .unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"output".to_vec());
        session.pump_with(&mut persistence, 3).unwrap();
        fixture.state.lock().unwrap().tail = b"tail".to_vec();
        session
            .close_with(
                &mut persistence,
                &owner("owner"),
                TerminalClosePolicy::Force,
                4,
            )
            .unwrap();
        assert_eq!(
            persistence.calls,
            [
                "state",
                "reserve",
                "checkpoint",
                "checkpoint",
                "state",
                "checkpoint",
                "state",
                "append",
                "state",
                "checkpoint",
                "append",
                "state",
                "reserve"
            ]
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert!(!session.owns_backend());
        assert!(session.publication_error().is_none());
    }

    #[test]
    fn denied_context_never_dispatches_native_resize_or_flushing_signal() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        fixture.state.lock().unwrap().signal_flushes = true;
        let before = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        let mut denied = Persistence {
            denied: true,
            ..Persistence::default()
        };
        assert_eq!(
            session.resize_with(
                &mut denied,
                &owner("owner"),
                &TerminalDimensions::new(4, 20).unwrap(),
                1
            ),
            Err(denied_error())
        );
        assert_eq!(
            session.signal_with(&mut denied, &owner("owner"), TerminalSignal::Interrupt, 1),
            Err(denied_error())
        );
        let state = fixture.state.lock().unwrap();
        assert!(state.resizes.is_empty());
        assert!(state.signals.is_empty());
        assert_eq!(state.closes, 0);
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Running);
        assert_eq!(std::fs::read(fixture.path.join("tj-meta")).unwrap(), before);
    }

    #[test]
    fn committed_output_accounting_error_keeps_cursor_and_never_replays_replies() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let output = b"committed\x1b[6n";
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(output.to_vec());
        let mut persistence = Persistence {
            fail_accounting: Some("append"),
            ..Persistence::default()
        };
        let expected = TerminalSessionError::History(TerminalHistoryError::Accounting(
            TerminalProfileError::AccountingMismatch,
        ));
        assert!(matches!(session.pump_with(&mut persistence, 1), Err(error) if error == expected));
        assert_eq!(session.context().cursor.offset(), output.len() as u64);
        assert_eq!(
            session
                .read(&owner("owner"), &TerminalCursor::new(1, 0).unwrap(), 64)
                .unwrap()
                .bytes,
            output
        );
        assert_eq!(session.publication_error(), Some(expected));
        assert!(session.input.is_quiesced());
        assert!(session.monitor_notifications_incomplete);
        assert!(
            session
                .pump_with(&mut persistence, 2)
                .unwrap()
                .output
                .is_empty()
        );
        assert_eq!(
            persistence
                .calls
                .iter()
                .filter(|kind| **kind == "append")
                .count(),
            1
        );
        assert!(fixture.state.lock().unwrap().writes.is_empty());
        drop(session);
        let recovered = TerminalRecoveredSession::recover_with(
            &mut persistence,
            fixture.recover(),
            &owner("owner"),
            2,
        )
        .unwrap();
        assert_eq!(recovered.facts.context.cursor.offset(), output.len() as u64);
        assert!(recovered.facts.monitor_notifications_incomplete);
    }

    #[test]
    fn denied_close_still_drains_and_cleans_native_authority() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        fixture.state.lock().unwrap().tail = b"discarded tail".to_vec();
        let before = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        let mut denied = Persistence {
            denied: true,
            ..Persistence::default()
        };
        assert_eq!(
            session.close_with(&mut denied, &owner("owner"), TerminalClosePolicy::Force, 1),
            Err(denied_error())
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert!(fixture.state.lock().unwrap().tail.is_empty());
        assert!(!session.owns_backend());
        assert!(session.input.is_quiesced());
        assert_eq!(session.publication_error(), Some(denied_error()));
        assert_eq!(std::fs::read(fixture.path.join("tj-meta")).unwrap(), before);
        assert!(session.history.screen().is_err());
        assert!(session.pending_output_gap);
        let mut allowed = Persistence::default();
        session
            .close_with(&mut allowed, &owner("owner"), TerminalClosePolicy::Force, 2)
            .unwrap();
        assert_eq!(allowed.calls, ["checkpoint", "state", "reserve"]);
        assert!(!session.pending_output_gap);
        assert!(session.publication_error().is_none());
        drop(session);
        assert!(fixture.recover().screen().is_err());
    }

    #[test]
    fn contextless_teardown_never_writes_and_retains_failed_cleanup_for_retry() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        fixture.state.lock().unwrap().tail = b"discarded tail".to_vec();
        fixture.state.lock().unwrap().close_fails = true;
        let before = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        assert_eq!(
            session.teardown_without_persistence(true, 1, denied_error()),
            Err(TerminalSessionError::Native)
        );
        assert!(session.owns_backend());
        fixture.state.lock().unwrap().close_fails = false;
        assert_eq!(
            session.teardown_without_persistence(true, 0, denied_error()),
            Err(denied_error())
        );
        assert!(!session.owns_backend());
        assert_eq!(fixture.state.lock().unwrap().closes, 2);
        assert_eq!(session.context().now_ms, 1);
        assert_eq!(session.outcome(), Some(TerminalProcessOutcome::Exited(0)));
        assert_eq!(session.publication_error(), Some(denied_error()));
        assert!(session.monitor_notifications_incomplete);
        assert_eq!(std::fs::read(fixture.path.join("tj-meta")).unwrap(), before);
        assert!(session.history.screen().is_err());
        let mut denied = Persistence {
            denied: true,
            ..Persistence::default()
        };
        assert_eq!(session.persist_with(&mut denied), Err(denied_error()));
        assert_eq!(denied.calls, ["checkpoint"]);
        assert!(session.pending_output_gap);
        let mut allowed = Persistence::default();
        session.persist_with(&mut allowed).unwrap();
        assert_eq!(allowed.calls, ["checkpoint", "state"]);
        assert!(!session.pending_output_gap);
        drop(session);
        assert!(fixture.recover().screen().is_err());
    }

    #[test]
    fn rejected_consumed_output_requires_gap_barrier_before_state_retry() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"discarded".to_vec());
        let before = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        let mut denied = Persistence {
            denied: true,
            ..Persistence::default()
        };
        assert!(matches!(session.pump_with(&mut denied, 1), Err(error) if error == denied_error()));
        assert_eq!(denied.calls, ["append", "checkpoint"]);
        assert_eq!(session.context().cursor.offset(), 0);
        assert!(session.pending_output_gap);
        assert!(session.history.screen().is_err());
        assert!(session.input.is_quiesced());
        assert_eq!(std::fs::read(fixture.path.join("tj-meta")).unwrap(), before);
        let mut allowed = Persistence::default();
        session.persist_with(&mut allowed).unwrap();
        assert_eq!(allowed.calls, ["checkpoint", "state"]);
        assert!(!session.pending_output_gap);
        drop(session);
        let recovered = TerminalRecoveredSession::recover_with(
            &mut allowed,
            fixture.recover(),
            &owner("owner"),
            1,
        )
        .unwrap();
        assert!(recovered.history.screen().is_err());
        assert!(recovered.facts.monitor_notifications_incomplete);
    }

    #[test]
    fn close_tail_accounting_error_survives_successful_final_state_publication() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        fixture.state.lock().unwrap().tail = b"committed tail".to_vec();
        let mut persistence = Persistence {
            fail_accounting: Some("append"),
            ..Persistence::default()
        };
        let expected = TerminalSessionError::History(TerminalHistoryError::Accounting(
            TerminalProfileError::AccountingMismatch,
        ));
        assert_eq!(
            session.close_with(
                &mut persistence,
                &owner("owner"),
                TerminalClosePolicy::Force,
                1
            ),
            Err(expected)
        );
        assert_eq!(session.publication_error(), Some(expected));
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert!(!session.owns_backend());
        assert_eq!(session.context().cursor.offset(), 14);
        assert!(session.monitor_notifications_incomplete);
        assert!(fixture.state.lock().unwrap().writes.is_empty());
        drop(session);
        let recovered = TerminalRecoveredSession::recover_with(
            &mut persistence,
            fixture.recover(),
            &owner("owner"),
            1,
        )
        .unwrap();
        assert_eq!(recovered.facts.context.lifecycle, TerminalLifecycle::Closed);
        assert_eq!(recovered.facts.context.cursor.offset(), 14);
        assert!(recovered.facts.monitor_notifications_incomplete);
    }

    #[test]
    fn live_ack_accounting_failure_tracks_only_the_publication_that_committed() {
        for gap_pending in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            add(
                &mut session,
                Condition::OutputContains {
                    pattern: "event".into(),
                },
            );
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"event".to_vec());
            session.pump(1).unwrap();
            let acknowledged = session
                .events(&owner("owner"), &query())
                .unwrap()
                .last()
                .unwrap()
                .event_id;
            if gap_pending {
                assert_eq!(
                    session.teardown_without_persistence(true, 2, denied_error()),
                    Err(denied_error())
                );
            }
            let mut persistence = Persistence {
                fail_accounting: Some(if gap_pending { "checkpoint" } else { "state" }),
                ..Persistence::default()
            };
            assert!(matches!(
                session.events_with(
                    &mut persistence,
                    &owner("owner"),
                    &TerminalEventQuery {
                        acknowledge_event_id: Some(acknowledged),
                        ..query()
                    }
                ),
                Err(TerminalSessionError::History(
                    TerminalHistoryError::Accounting(_)
                ))
            ));
            let expected = if gap_pending { 0 } else { acknowledged };
            assert_eq!(session.monitors.acknowledged_event_id(), expected);
            assert!(!session.pending_output_gap);
            assert!(session.publication_error().is_some());
            assert_eq!(
                persistence.calls,
                [if gap_pending { "checkpoint" } else { "state" }]
            );
            drop(session);
            let recovered = TerminalRecoveredSession::recover_with(
                &mut persistence,
                fixture.recover(),
                &owner("owner"),
                2,
            )
            .unwrap();
            assert_eq!(recovered.monitors.acknowledged_event_id(), expected);
        }
    }

    #[test]
    fn recovered_contextual_ack_preserves_committed_receipt_after_accounting_error() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        add(
            &mut session,
            Condition::OutputContains {
                pattern: "event".into(),
            },
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"event".to_vec());
        session.pump(1).unwrap();
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 2)
            .unwrap();
        drop(session);
        let mut persistence = Persistence::default();
        let mut recovered = TerminalRecoveredSession::recover_with(
            &mut persistence,
            fixture.recover(),
            &owner("owner"),
            2,
        )
        .unwrap();
        let events = recovered
            .events_with(&mut persistence, &owner("owner"), &query())
            .unwrap();
        let acknowledged = events.last().unwrap().event_id;
        persistence.fail_accounting = Some("state");
        assert!(matches!(
            recovered.events_with(
                &mut persistence,
                &owner("owner"),
                &TerminalEventQuery {
                    acknowledge_event_id: Some(acknowledged),
                    ..query()
                }
            ),
            Err(TerminalSessionError::History(
                TerminalHistoryError::Accounting(_)
            ))
        ));
        assert_eq!(recovered.monitors.acknowledged_event_id(), acknowledged);
        assert!(recovered.publication_error().is_some());
        assert_eq!(persistence.calls, ["state"]);
        drop(recovered);
        let reopened = TerminalRecoveredSession::recover_with(
            &mut persistence,
            fixture.recover(),
            &owner("owner"),
            2,
        )
        .unwrap();
        assert_eq!(reopened.monitors.acknowledged_event_id(), acknowledged);
    }

    #[test]
    fn owner_scoped_launch_facts_survive_recovery_without_stale_attention() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        assert!(matches!(
            session.inspect(&owner("wrong")),
            Err(TerminalSessionError::NotFound)
        ));
        let mut facts = session.inspect(&owner("owner")).unwrap();
        let metadata = serde_json::to_value(facts.metadata.as_ref().unwrap()).unwrap();
        facts.attention = machine_god_core::TerminalAttentionState::new(
            machine_god_core::TerminalAttention::UserTakeover,
            machine_god_core::TerminalWriteLease::Human,
        )
        .unwrap();
        session
            .history
            .publish_state(&facts.encode(&session.monitors).unwrap())
            .unwrap();
        drop(session);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        let facts = recovered.facts(&owner("owner")).unwrap();
        assert_eq!(
            serde_json::to_value(facts.metadata.as_ref().unwrap()).unwrap(),
            metadata
        );
        assert_eq!(
            facts.attention,
            machine_god_core::TerminalAttentionState::default()
        );
        assert_eq!(facts.context.lifecycle, TerminalLifecycle::Lost);
        drop(recovered);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(
            recovered.facts(&owner("owner")).unwrap().attention,
            machine_god_core::TerminalAttentionState::default()
        );
        assert!(fixture.state.lock().unwrap().writes.is_empty());
        assert!(fixture.state.lock().unwrap().signals.is_empty());
    }

    #[test]
    fn idle_pumps_do_not_republish_unchanged_durable_state() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let metadata = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        for time in 1..=20 {
            session.pump(time).unwrap();
        }
        assert_eq!(
            std::fs::read(fixture.path.join("tj-meta")).unwrap(),
            metadata
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"changed".to_vec());
        session.pump(21).unwrap();
        assert_ne!(
            std::fs::read(fixture.path.join("tj-meta")).unwrap(),
            metadata
        );
        let state = session.history.load_state().unwrap().unwrap();
        let (facts, _) = TerminalSessionFacts::decode(&state.bytes, &id(), &state.source).unwrap();
        assert_eq!(facts.context.cursor, session.context().cursor);
        assert_eq!(facts.last_output_ms, 21);
    }

    #[test]
    fn closed_facts_monitor_events_and_acknowledgements_survive_reopen() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(5).unwrap();
        add(
            &mut session,
            Condition::OutputContains {
                pattern: "ready".into(),
            },
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"ready".to_vec());
        session.pump(10).unwrap();
        let events = session.events(&owner("owner"), &query()).unwrap();
        let acknowledged = events.last().unwrap().event_id;
        session
            .events(
                &owner("owner"),
                &TerminalEventQuery {
                    acknowledge_event_id: Some(acknowledged),
                    ..query()
                },
            )
            .unwrap();
        session
            .close(&owner("owner"), TerminalClosePolicy::Graceful, 11)
            .unwrap();
        let screen = session.screen(&owner("owner")).unwrap();
        drop(session);
        let mut recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(
            recovered.facts(&owner("owner")).unwrap().context.lifecycle,
            TerminalLifecycle::Closed
        );
        assert_eq!(
            recovered.facts.outcome,
            Some(TerminalProcessOutcome::Exited(0))
        );
        assert_eq!(recovered.monitors.acknowledged_event_id(), acknowledged);
        assert_eq!(recovered.events(&owner("owner"), &query()).unwrap(), events);
        assert_eq!(recovered.screen(&owner("owner")).unwrap(), screen);
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
    }

    #[test]
    fn host_loss_revokes_probes_without_replaying_input_or_accepting_other_owners() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        acquire(&mut session);
        fixture.state.lock().unwrap().write_limit = 1;
        session
            .write(&owner("owner"), writer(), &text("secret input"), false)
            .unwrap();
        add(
            &mut session,
            Condition::TcpReady {
                host: "example.invalid".into(),
                port: 80,
            },
        );
        assert_eq!(session.pump(10).unwrap().probes.len(), 1);
        drop(session);
        let metadata = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        assert!(matches!(
            TerminalRecoveredSession::recover(fixture.recover(), &owner("wrong"), 100),
            Err(TerminalSessionError::NotFound),
        ));
        assert_eq!(
            std::fs::read(fixture.path.join("tj-meta")).unwrap(),
            metadata
        );
        let (writes, calls) = {
            let state = fixture.state.lock().unwrap();
            (state.writes.clone(), state.status_calls)
        };
        assert!(matches!(
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 9),
            Err(TerminalSessionError::Clock),
        ));
        assert_eq!(
            std::fs::read(fixture.path.join("tj-meta")).unwrap(),
            metadata
        );
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(recovered.facts.context.lifecycle, TerminalLifecycle::Lost);
        assert_eq!(recovered.facts.context.now_ms, 100);
        assert_eq!(recovered.monitors.len(), 0);
        assert!(recovered.monitors.next_deadline().is_none());
        assert!(recovered.facts.outcome.is_none());
        assert!(matches!(
            recovered.facts(&owner("wrong")),
            Err(TerminalSessionError::NotFound)
        ));
        let origin = TerminalCursor::new(1, 0).unwrap();
        assert!(matches!(
            recovered.read(&owner("wrong"), &origin, 10),
            Err(TerminalSessionError::NotFound),
        ));
        assert_eq!(fixture.state.lock().unwrap().writes, writes);
        assert_eq!(fixture.state.lock().unwrap().status_calls, calls);
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        let saved = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        drop(recovered);
        let _recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(std::fs::read(fixture.path.join("tj-meta")).unwrap(), saved);
    }

    #[test]
    fn recovered_publication_retry_preserves_only_committed_acknowledgements() {
        for accounting in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            add(
                &mut session,
                Condition::OutputContains {
                    pattern: "kept".into(),
                },
            );
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"kept".to_vec());
            session.pump(1).unwrap();
            session
                .close(&owner("owner"), TerminalClosePolicy::Force, 2)
                .unwrap();
            drop(session);
            let mut recovered =
                TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 2).unwrap();
            let mut events = query();
            let last = recovered
                .events(&owner("owner"), &events)
                .unwrap()
                .last()
                .unwrap()
                .event_id;
            events.acknowledge_event_id = Some(last);
            let mut failure = Persistence {
                denied: !accounting,
                fail_accounting: accounting.then_some("state"),
                ..Persistence::default()
            };
            assert!(
                recovered
                    .events_with(&mut failure, &owner("owner"), &events)
                    .is_err()
            );
            let committed_ack = if accounting { last } else { 0 };
            assert_eq!(recovered.monitors.acknowledged_event_id(), committed_ack);
            let mut allowed = Persistence::default();
            assert!(matches!(
                recovered.retry_publication_with(&mut allowed, &owner("foreign")),
                Err(TerminalSessionError::NotFound)
            ));
            assert!(allowed.calls.is_empty());
            recovered
                .retry_publication_with(&mut allowed, &owner("owner"))
                .unwrap();
            assert!(recovered.publication_error().is_none());
            assert!(recovered.history.require_live().is_err());
            assert_eq!(recovered.monitors.acknowledged_event_id(), committed_ack);
            assert_eq!(allowed.calls, ["state"]);
            recovered
                .retry_publication_with(&mut allowed, &owner("owner"))
                .unwrap();
            assert_eq!(allowed.calls, ["state"]);
            drop(recovered);
            let recovered =
                TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 2).unwrap();
            assert_eq!(recovered.monitors.acknowledged_event_id(), committed_ack);
        }
    }

    #[test]
    fn crash_between_raw_and_state_commit_records_an_observation_gap() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        add(
            &mut session,
            Condition::OutputContains {
                pattern: "unobserved".into(),
            },
        );
        let observed = session.context().cursor;
        // Simulate a crash after raw fsync but before the monitor/state transaction.
        let latest = session.history.append(b"unobserved").unwrap().cursor;
        drop(session);
        let mut recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        let gap = recovered.facts.observation_gap.as_ref().unwrap();
        assert_eq!(gap.missing_from, observed);
        assert_eq!(gap.available_from, latest);
        assert_eq!(recovered.facts.context.cursor, latest);
        assert_eq!(recovered.facts.context.lifecycle, TerminalLifecycle::Lost);
        assert_eq!(
            recovered
                .read(&owner("owner"), &observed, 100)
                .unwrap()
                .bytes,
            b"unobserved"
        );
        assert!(
            recovered
                .events(&owner("owner"), &query())
                .unwrap()
                .iter()
                .all(|event| {
                    event.reason != machine_god_core::TerminalMonitorEventReason::Matched
                })
        );
        drop(recovered);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert!(recovered.facts.observation_gap.is_some());
    }

    #[test]
    fn recovered_event_acknowledgement_is_durable_and_wrong_owner_has_no_effects() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        add(
            &mut session,
            Condition::OutputContains {
                pattern: "event".into(),
            },
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"event".to_vec());
        session.pump(1).unwrap();
        drop(session);
        let mut recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        let events = recovered.events(&owner("owner"), &query()).unwrap();
        let ack = events.last().unwrap().event_id;
        let query = TerminalEventQuery {
            acknowledge_event_id: Some(ack),
            ..query()
        };
        let metadata = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        assert_eq!(
            recovered.events(&owner("wrong"), &query),
            Err(TerminalSessionError::NotFound)
        );
        assert_eq!(
            std::fs::read(fixture.path.join("tj-meta")).unwrap(),
            metadata
        );
        recovered.events(&owner("owner"), &query).unwrap();
        drop(recovered);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(recovered.monitors.acknowledged_event_id(), ack);
    }

    #[test]
    fn failed_live_acknowledgement_cannot_become_a_successful_memory_only_retry() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        add(
            &mut session,
            Condition::OutputContains {
                pattern: "event".into(),
            },
        );
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"event".to_vec());
        session.pump(1).unwrap();
        let events = session.events(&owner("owner"), &query()).unwrap();
        let ack = events.last().unwrap().event_id;
        let query = TerminalEventQuery {
            acknowledge_event_id: Some(ack),
            ..query()
        };
        let _temporary = fixture.block_publication();
        assert!(session.events(&owner("owner"), &query).is_err());
        assert_eq!(session.monitors.acknowledged_event_id(), 0);
        assert!(session.events(&owner("owner"), &query).is_err());
        assert_eq!(session.monitors.acknowledged_event_id(), 0);
        assert!(session.input.is_quiesced());
        drop(session);
        let mut recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(recovered.monitors.acknowledged_event_id(), 0);
        recovered.events(&owner("owner"), &query).unwrap();
        drop(recovered);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(recovered.monitors.acknowledged_event_id(), ack);
    }

    #[test]
    fn recovery_retains_history_when_terminal_notifications_exhaust_their_counter() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        session
            .monitor(
                &owner("owner"),
                TerminalMonitorOperation::Add {
                    definition: TerminalMonitorDefinition {
                        condition: Condition::ProcessExit,
                        check_schedule: None,
                        notify: TerminalNotifySchedule::OnExit,
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
            .push_back(b"retained".to_vec());
        session.pump(1).unwrap();
        let mut saved: serde_json::Value =
            serde_json::from_slice(&session.monitors.snapshot().unwrap()).unwrap();
        saved["next_event_id"] = serde_json::json!(u64::MAX);
        saved["dropped_through_event_id"] = serde_json::json!(u64::MAX - 1);
        saved["events"] = serde_json::json!([]);
        session.monitors =
            TerminalMonitorSet::restore(&serde_json::to_vec(&saved).unwrap()).unwrap();
        session.persist().unwrap();
        drop(session);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(recovered.facts.context.lifecycle, TerminalLifecycle::Lost);
        assert!(recovered.facts.monitor_notifications_incomplete);
        assert_eq!(recovered.monitors.len(), 0);
        assert!(recovered.monitors.next_deadline().is_none());
        assert_eq!(
            recovered
                .read(&owner("owner"), &TerminalCursor::new(1, 0).unwrap(), 64)
                .unwrap()
                .bytes,
            b"retained",
        );
        let metadata = std::fs::read(fixture.path.join("tj-meta")).unwrap();
        drop(recovered);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert!(recovered.facts.monitor_notifications_incomplete);
        assert_eq!(
            std::fs::read(fixture.path.join("tj-meta")).unwrap(),
            metadata
        );
    }

    #[test]
    fn consumed_output_notification_failure_survives_recovery() {
        for close in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            add(
                &mut session,
                Condition::OutputContains {
                    pattern: "event".into(),
                },
            );
            let mut saved: serde_json::Value =
                serde_json::from_slice(&session.monitors.snapshot().unwrap()).unwrap();
            saved["next_event_id"] = serde_json::json!(u64::MAX);
            saved["dropped_through_event_id"] = serde_json::json!(u64::MAX - 1);
            saved["events"] = serde_json::json!([]);
            session.monitors =
                TerminalMonitorSet::restore(&serde_json::to_vec(&saved).unwrap()).unwrap();
            if close {
                fixture.state.lock().unwrap().tail = b"event".to_vec();
            } else {
                fixture
                    .state
                    .lock()
                    .unwrap()
                    .output
                    .push_back(b"event".to_vec());
            }
            let result = if close {
                session.close(&owner("owner"), TerminalClosePolicy::Force, 1)
            } else {
                session.pump(1).map(|_| ())
            };
            assert!(matches!(
                result,
                Err(TerminalSessionError::Monitor(TerminalMonitorError::Counter)),
            ));
            assert!(session.monitor_notifications_incomplete);
            drop(session);
            let recovered =
                TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
            assert!(recovered.facts.monitor_notifications_incomplete);
            assert_eq!(recovered.monitors.len(), 0);
            assert_eq!(
                recovered
                    .read(&owner("owner"), &TerminalCursor::new(1, 0).unwrap(), 64)
                    .unwrap()
                    .bytes,
                b"event",
            );
            assert_eq!(
                recovered.facts.context.lifecycle,
                if close {
                    TerminalLifecycle::Closed
                } else {
                    TerminalLifecycle::Lost
                },
            );
        }
    }

    #[test]
    fn resize_and_signal_observation_failures_survive_recovery() {
        for signal in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            fixture
                .state
                .lock()
                .unwrap()
                .output
                .push_back(b"event".to_vec());
            session.pump(1).unwrap();
            add(
                &mut session,
                Condition::ScreenMatches {
                    pattern: "*event*".into(),
                },
            );
            let mut saved: serde_json::Value =
                serde_json::from_slice(&session.monitors.snapshot().unwrap()).unwrap();
            saved["next_event_id"] = serde_json::json!(u64::MAX);
            saved["dropped_through_event_id"] = serde_json::json!(u64::MAX - 1);
            saved["events"] = serde_json::json!([]);
            if signal {
                saved["monitors"][0]["definition"]["notify"] =
                    serde_json::to_value(TerminalNotifySchedule::OnStateChange).unwrap();
            }
            session.monitors =
                TerminalMonitorSet::restore(&serde_json::to_vec(&saved).unwrap()).unwrap();
            session.persist().unwrap();
            let result = if signal {
                fixture.state.lock().unwrap().signal_flushes = true;
                session.signal(&owner("owner"), TerminalSignal::Interrupt, 2)
            } else {
                session.resize(&owner("owner"), &TerminalDimensions::new(4, 20).unwrap(), 2)
            };
            assert!(
                matches!(
                    result,
                    Err(TerminalSessionError::Monitor(TerminalMonitorError::Counter))
                ),
                "signal={signal}: {result:?}"
            );
            assert!(session.monitor_notifications_incomplete);
            assert!(session.input.is_quiesced());
            assert!(fixture.state.lock().unwrap().signals.is_empty());
            drop(session);
            let recovered =
                TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
            assert!(recovered.facts.monitor_notifications_incomplete);
            assert_eq!(recovered.facts.context.lifecycle, TerminalLifecycle::Lost);
            assert_eq!(recovered.monitors.len(), 0);
            if !signal {
                assert!(recovered.facts.observation_gap.is_none());
                assert_eq!(
                    recovered.screen(&owner("owner")).unwrap().dimensions.rows(),
                    4
                );
            }
        }
    }

    #[test]
    fn known_process_exit_survives_monitor_notification_failure() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        add(&mut session, Condition::ProcessExit);
        let mut saved: serde_json::Value =
            serde_json::from_slice(&session.monitors.snapshot().unwrap()).unwrap();
        saved["next_event_id"] = serde_json::json!(u64::MAX);
        saved["dropped_through_event_id"] = serde_json::json!(u64::MAX - 1);
        saved["events"] = serde_json::json!([]);
        session.monitors =
            TerminalMonitorSet::restore(&serde_json::to_vec(&saved).unwrap()).unwrap();
        fixture.state.lock().unwrap().status = TerminalPtyStatus::Exited(23);
        assert!(matches!(
            session.pump(1),
            Err(TerminalSessionError::Monitor(TerminalMonitorError::Counter)),
        ));
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Exited);
        assert_eq!(session.outcome(), Some(TerminalProcessOutcome::Exited(23)));
        assert!(session.monitor_notifications_incomplete);
        drop(session);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(recovered.facts.context.lifecycle, TerminalLifecycle::Exited);
        assert_eq!(
            recovered.facts.outcome,
            Some(TerminalProcessOutcome::Exited(23))
        );
        assert!(recovered.facts.monitor_notifications_incomplete);
    }

    #[test]
    fn state_publication_failure_quiesces_input_but_cannot_skip_cleanup() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        let _temporary = rustix::fs::openat(
            fixture.fd(),
            "tj-meta.tmp",
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_bits_retain(0o600),
        )
        .unwrap();
        assert!(session.shell_ready(1).is_err());
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Lost);
        assert!(session.input.is_quiesced());
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        // Failed state publication makes history read-only, but cannot skip
        // the explicitly requested native cleanup.
        assert!(
            session
                .close(&owner("owner"), TerminalClosePolicy::Force, 2)
                .is_err()
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        assert!(session.input.is_quiesced());
        assert!(session.backend.is_none());
        drop(session);
        let recovered =
            TerminalRecoveredSession::recover(fixture.recover(), &owner("owner"), 100).unwrap();
        assert_eq!(recovered.facts.context.lifecycle, TerminalLifecycle::Lost);
        assert!(recovered.facts.outcome.is_none());
    }

    #[test]
    fn wrong_incarnation_and_clock_rejection_precede_every_native_effect() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(10).unwrap();
        let wrong = owner("other-incarnation");
        let origin = session.context().cursor;
        assert_eq!(
            session.write(&wrong, writer(), &text("secret"), false),
            Err(TerminalSessionError::NotFound)
        );
        assert!(matches!(
            session.read(&wrong, &origin, 64),
            Err(TerminalSessionError::NotFound)
        ));
        assert_eq!(session.screen(&wrong), Err(TerminalSessionError::NotFound));
        assert_eq!(
            session.resize(&wrong, &TerminalDimensions::new(4, 30).unwrap(), 11),
            Err(TerminalSessionError::NotFound)
        );
        assert_eq!(
            session.signal(&wrong, TerminalSignal::Kill, 11),
            Err(TerminalSessionError::NotFound)
        );
        assert_eq!(
            session.close(&wrong, TerminalClosePolicy::Force, 11),
            Err(TerminalSessionError::NotFound)
        );
        assert!(matches!(session.pump(9), Err(TerminalSessionError::Clock)));
        assert_eq!(fixture.state.lock().unwrap().status_calls, 0);
        assert_eq!(session.context().cursor, origin);
        assert_eq!(format!("{session:?}"), "TerminalSession { .. }");
    }

    #[test]
    fn driver_identity_must_match_its_history_and_recovered_history_stays_read_only() {
        let fixture = Fixture::new();
        let session = fixture.session();
        let TerminalSession {
            backend, history, ..
        } = session;
        assert!(matches!(
            TerminalSession::new(
                backend.unwrap(),
                history,
                owner("owner"),
                TerminalSessionId::new("other-terminal").unwrap(),
                crate::terminal_session_record::test_metadata(),
                0
            ),
            Err(TerminalSessionError::InvalidState)
        ));
        assert!(matches!(
            TerminalSession::new(
                Backend(Arc::clone(&fixture.state)),
                fixture.recover(),
                owner("owner"),
                id(),
                crate::terminal_session_record::test_metadata(),
                0
            ),
            Err(TerminalSessionError::History(
                TerminalHistoryError::ReadOnly
            ))
        ));
        assert_eq!(fixture.state.lock().unwrap().status_calls, 0);
    }

    #[test]
    fn output_never_impersonates_readiness_and_pending_input_outlives_attention() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"READY".to_vec());
        assert_eq!(
            session.pump(1).unwrap().lifecycle,
            TerminalLifecycle::Starting
        );
        assert_eq!(
            session.write(&owner("owner"), writer(), &text("early"), false),
            Err(TerminalSessionError::InvalidState)
        );
        session.shell_ready(2).unwrap();
        acquire(&mut session);
        fixture.state.lock().unwrap().write_limit = 1;
        let receipt = session
            .write(&owner("owner"), writer(), &text("a界"), false)
            .unwrap();
        let operation = receipt.operation_id.unwrap();
        assert_eq!(receipt.accepted_bytes, 1);
        assert_eq!(
            session.write(&owner("owner"), writer(), &text("cancelled"), true),
            Err(TerminalSessionError::Input(TerminalInputError::Cancelled))
        );
        for now in 3..7 {
            session.pump(now).unwrap();
        }
        assert_eq!(fixture.state.lock().unwrap().writes, "a界".as_bytes());
        assert_eq!(
            session
                .write_receipt(&owner("owner"), writer(), operation)
                .unwrap()
                .progress,
            TerminalInputProgress::Complete
        );
    }

    #[test]
    fn query_replies_are_once_only_and_do_not_require_a_writer_lease() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .push_back(b"\x1b[6n".to_vec());
        let step = session.pump(1).unwrap();
        assert_eq!(step.output, b"\x1b[6n");
        assert!(fixture.state.lock().unwrap().writes.is_empty());
        session.pump(2).unwrap();
        session.pump(3).unwrap();
        assert_eq!(fixture.state.lock().unwrap().writes, b"\x1b[1;1R");
        session.shell_ready(3).unwrap();
        assert_eq!(
            session.write(&owner("owner"), writer(), &text("no lease"), false),
            Err(TerminalSessionError::Input(
                TerminalInputError::LeaseConflict
            ))
        );
    }

    #[test]
    fn resize_and_signal_cannot_overtake_an_admitted_partial_write() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        acquire(&mut session);
        fixture.state.lock().unwrap().write_limit = 1;
        session
            .write(&owner("owner"), writer(), &text("abc"), false)
            .unwrap();
        let dimensions = TerminalDimensions::new(4, 30).unwrap();
        assert_eq!(
            session.resize(&owner("owner"), &dimensions, 1),
            Err(TerminalInputError::Busy.into())
        );
        assert_eq!(
            session.signal(&owner("owner"), TerminalSignal::Interrupt, 1),
            Err(TerminalInputError::Busy.into())
        );
        assert!(fixture.state.lock().unwrap().resizes.is_empty());
        assert!(fixture.state.lock().unwrap().signals.is_empty());
        session.pump(1).unwrap();
        session.pump(2).unwrap();
        session.resize(&owner("owner"), &dimensions, 2).unwrap();
        session
            .signal(&owner("owner"), TerminalSignal::Interrupt, 2)
            .unwrap();
        assert_eq!(fixture.state.lock().unwrap().writes, b"abc");
    }

    #[test]
    fn output_and_screen_monitors_observe_committed_cursors_and_waits_do_not_own_close() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let raw = add(
            &mut session,
            Condition::OutputContains {
                pattern: "hello".into(),
            },
        );
        let screen = add(
            &mut session,
            Condition::ScreenMatches {
                pattern: "*hello*".into(),
            },
        );
        let mut wait = TerminalWaitState::new(
            TerminalWaitRequest {
                condition: TerminalReturnCondition::Match {
                    pattern: "hello".into(),
                },
                safety_ceiling_ms: 100,
            },
            &session.context(),
            session.last_output_ms(),
            false,
        )
        .unwrap();
        fixture
            .state
            .lock()
            .unwrap()
            .output
            .extend([b"hel".to_vec(), b"lo".to_vec()]);
        for now in 1..=2 {
            let step = session.pump(now).unwrap();
            wait.output(&step.output, now).unwrap();
            assert_eq!(step.cursor, session.context().cursor);
        }
        let events = session.events(&owner("owner"), &query()).unwrap();
        for id in [&raw, &screen] {
            assert!(
                events.iter().any(
                    |event| &event.monitor_id == id && event.cursor == session.context().cursor
                )
            );
        }
        assert_eq!(
            wait.poll(&session.context(), session.outcome(), false)
                .unwrap(),
            Some(TerminalWaitOutcome::ConditionMet)
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
    }

    #[test]
    fn probe_descriptions_need_explicit_evidence_and_stale_completion_cannot_rewind_clock() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let monitor_id = add(
            &mut session,
            Condition::TcpReady {
                host: "example.invalid".into(),
                port: 1234,
            },
        );
        let request = session.pump(10).unwrap().probes.remove(0);
        let evidence = TerminalProbeEvidence {
            session_id: request.session_id.clone(),
            monitor_id: monitor_id.clone(),
            generation: request.generation,
            request_sequence: request.request_sequence,
            completed_at_ms: 11,
            output_bytes: 0,
            truncated: false,
            timed_out: false,
            result: Ok(TerminalProbeObservation::Tcp { connected: true }),
        };
        session
            .resize(
                &owner("owner"),
                &TerminalDimensions::new(4, 30).unwrap(),
                20,
            )
            .unwrap();
        assert_eq!(
            session.complete_probe(evidence.clone(), 11),
            Err(TerminalSessionError::Clock)
        );
        assert_eq!(session.context().now_ms, 20);
        assert!(session.complete_probe(evidence.clone(), 20).unwrap());
        assert!(!session.complete_probe(evidence, -1).unwrap());
        assert_eq!(session.context().now_ms, 20);
    }

    #[test]
    fn close_drains_final_output_then_notifies_monitors_and_preserves_readable_history() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        acquire(&mut session);
        let monitor_id = add(
            &mut session,
            Condition::OutputContains {
                pattern: "tail".into(),
            },
        );
        fixture.state.lock().unwrap().tail = [b"tail".as_slice(), &b"\x1b[6n".repeat(20)].concat();
        session
            .close(&owner("owner"), TerminalClosePolicy::Graceful, 5)
            .unwrap();
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Closed);
        assert_eq!(session.outcome(), Some(TerminalProcessOutcome::Exited(0)));
        assert_eq!(session.monitors.len(), 0);
        assert!(
            session
                .events(&owner("owner"), &query())
                .unwrap()
                .iter()
                .any(|event| event.monitor_id == monitor_id)
        );
        assert!(fixture.state.lock().unwrap().writes.is_empty());
        let expected = session.screen(&owner("owner")).unwrap();
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 6)
            .unwrap();
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        drop(session);
        assert_eq!(fixture.recover().screen().unwrap(), expected);
    }

    #[test]
    fn incomplete_close_and_failed_history_publication_never_skip_native_cleanup() {
        for publication_failure in [false, true] {
            let fixture = Fixture::new();
            let mut session = fixture.session();
            session.shell_ready(0).unwrap();
            if publication_failure {
                std::fs::write(fixture.path.join("tj-meta.tmp"), b"abandoned").unwrap();
            } else {
                fixture.state.lock().unwrap().incomplete = true;
            }
            let result = session.close(&owner("owner"), TerminalClosePolicy::Force, 1);
            assert_eq!(result.is_err(), publication_failure);
            assert_eq!(fixture.state.lock().unwrap().closes, 1);
            assert!(session.input.is_quiesced());
            assert!(session.screen(&owner("owner")).is_err());
            assert_eq!(session.monitors.len(), 0);
        }
    }

    #[test]
    fn failed_close_keeps_owned_backend_for_retry_but_no_pending_input() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        acquire(&mut session);
        fixture.state.lock().unwrap().write_limit = 0;
        let receipt = session
            .write(&owner("owner"), writer(), &text("not-yet-written"), false)
            .unwrap();
        fixture.state.lock().unwrap().close_fails = true;
        assert_eq!(
            session.close(&owner("owner"), TerminalClosePolicy::Force, 1),
            Err(TerminalSessionError::Native)
        );
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Lost);
        assert!(session.backend.is_some());
        assert_eq!(
            session
                .write_receipt(&owner("owner"), writer(), receipt.operation_id.unwrap())
                .unwrap()
                .progress,
            TerminalInputProgress::Closed
        );
        fixture.state.lock().unwrap().close_fails = false;
        session
            .close(&owner("owner"), TerminalClosePolicy::Force, 2)
            .unwrap();
        assert_eq!(fixture.state.lock().unwrap().closes, 2);
        assert!(fixture.state.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn event_counter_failure_cannot_leave_monitors_or_late_probe_authority_after_close() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let monitor_id = add(&mut session, Condition::ProcessExit);
        let mut saved: serde_json::Value =
            serde_json::from_slice(&session.monitors.snapshot().unwrap()).unwrap();
        saved["next_event_id"] = serde_json::json!(u64::MAX);
        saved["dropped_through_event_id"] = serde_json::json!(u64::MAX - 1);
        saved["events"] = serde_json::json!([]);
        session.monitors =
            TerminalMonitorSet::restore(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(
            session.close(&owner("owner"), TerminalClosePolicy::Force, 1),
            Err(TerminalSessionError::Monitor(TerminalMonitorError::Counter))
        );
        assert_eq!(session.monitors.len(), 0);
        assert!(session.backend.is_none());
        assert_eq!(fixture.state.lock().unwrap().closes, 1);
        let evidence = TerminalProbeEvidence {
            session_id: id(),
            monitor_id,
            generation: 1,
            request_sequence: 1,
            completed_at_ms: 999,
            output_bytes: 0,
            truncated: false,
            timed_out: false,
            result: Ok(TerminalProbeObservation::Tcp { connected: true }),
        };
        assert!(!session.complete_probe(evidence, 999).unwrap());
        assert_eq!(session.context().now_ms, 1);
    }

    #[test]
    fn flushing_signal_records_a_gap_and_failure_never_escalates() {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        session.shell_ready(0).unwrap();
        let monitor_id = add(
            &mut session,
            Condition::OutputContains {
                pattern: "pattern".into(),
            },
        );
        {
            let mut state = fixture.state.lock().unwrap();
            state.signal_flushes = true;
            state.signal_fails = true;
        }
        assert_eq!(
            session.signal(&owner("owner"), TerminalSignal::Interrupt, 1),
            Err(TerminalSessionError::Native)
        );
        assert_eq!(
            fixture.state.lock().unwrap().signals,
            vec![TerminalSignal::Interrupt]
        );
        assert_eq!(fixture.state.lock().unwrap().closes, 0);
        assert!(session.screen(&owner("owner")).is_err());
        assert_eq!(
            session.monitors.state(&monitor_id),
            Some(TerminalMonitorState::Degraded)
        );
    }
}
