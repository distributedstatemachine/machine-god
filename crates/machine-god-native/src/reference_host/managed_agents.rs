//! Outer native driver ownership, deliberately excluded from shared engine services.
pub(super) mod history;
mod staged;
use super::{
    NativeReferenceHost,
    managed_factory::{
        ManagedRestorationAuthority, SharedManagedRuntimeFactory,
        SharedManagedRuntimeFactoryOptions,
    },
    mcp::ManagedParentMcpSeed,
};
use crate::managed::{
    manager::{
        ManagedForegroundReservation, ManagedForegroundSelection, ManagedManager, ManagedSelection,
        ManagerLimits,
        factory::{ManagedRuntimeError, PreparedManagedRuntime},
    },
    notices::NoticePrincipal,
    store::{JournalLimits, ManagedJournal},
};
use crate::{
    NativeConversation, NativeConversationRuntime, NativeModelPreferences,
    NativePermissionPolicySnapshot, NativeSessionOrigin, NativeWorkspaceScopeSnapshot,
};
pub use history::{
    NativeManagedHistoryError, NativeManagedHistoryOutcome, NativeManagedHistoryRequest,
    NativeManagedHistorySnapshot,
};
use machine_god_core::{BoxFuture, ManagedAgentState};
use rustix::fd::OwnedFd;
#[cfg(any(test, feature = "ai-gateway-http"))]
pub(crate) use staged::{NativeManagedStagedFailure, NativeManagedStagedParent};
use std::{
    fmt,
    num::NonZeroU64,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

/// Fixed host-operation categories; never raw filesystem or provider diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeManagedAgentsError {
    Configuration,
    Unavailable,
    Capacity,
    Persistence,
    Ambiguous,
}
impl fmt::Display for NativeManagedAgentsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "managed-agent host configuration failed",
            Self::Unavailable => "managed-agent host unavailable",
            Self::Capacity => "managed-agent host capacity unavailable",
            Self::Persistence => "managed-agent journal unavailable",
            Self::Ambiguous => "managed-agent operation requires reconciliation",
        })
    }
}
impl std::error::Error for NativeManagedAgentsError {}

/// Bounded result observation only. The outer native manager retains accepted
/// command custody; callers must co-poll it and retain cancellation explicitly.
pub type NativeManagedCommandResponse = BoxFuture<
    'static,
    Result<machine_god_core::ManagedSubagentResult, machine_god_core::ManagedSubagentError>,
>;

/// Weak native navigation identity. Labels alone cannot construct this value.
#[derive(Clone, Debug)]
pub struct NativeManagedAgentSelection(ManagedSelection);

/// Bounded immutable resident projection, not permission or process authority.
#[derive(Debug)]
pub struct NativeManagedAgentView {
    pub id: String,
    pub generation: u64,
    pub name: String,
    pub state: ManagedAgentState,
    pub selection: NativeManagedAgentSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeManagedAgentsProgress {
    pub residents: usize,
    pub executing: usize,
    pub waiters: usize,
    pub closing: bool,
    pub blocked: bool,
}
impl From<crate::managed::manager::ManagerProgress> for NativeManagedAgentsProgress {
    fn from(progress: crate::managed::manager::ManagerProgress) -> Self {
        Self {
            residents: progress.residents,
            executing: progress.executing,
            waiters: progress.waiters,
            closing: progress.closing,
            blocked: progress.blocked.is_some(),
        }
    }
}

/// Owns children and foreground resources independently of selected presentation.
/// The caller must co-poll this driver with foreground streams and host shutdown.
pub struct NativeManagedAgents {
    history: Box<history::Reader>,
    manager: ManagedManager,
    factory: Arc<SharedManagedRuntimeFactory>,
    parent_mcp: Arc<ManagedParentMcpSeed>,
    journal: ManagedJournal,
}
impl fmt::Debug for NativeManagedAgents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedAgents { .. }")
    }
}

impl NativeReferenceHost {
    /// Whether this host selected the managed-agent lifecycle, including after
    /// its assembly transfers to the outer owner. An MCP seed alone does not
    /// select that lifecycle.
    #[must_use]
    pub fn managed_agents_selected(&self) -> bool {
        self.managed_selected
    }

    fn validate_managed_open(&self) -> Result<(), NativeManagedAgentsError> {
        self.managed
            .as_ref()
            .filter(|assembly| assembly.mailbox.is_some() && assembly.parent_mcp.is_some())
            .map(|_| ())
            .ok_or(NativeManagedAgentsError::Configuration)
    }

    /// Opens a workspace/origin-specific private journal beneath the explicitly
    /// supplied state descriptor. No workspace path is reopened for authority.
    /// Directory preparation and journal ownership use the existing native workers.
    /// # Errors
    /// Invalid private state authority, unavailable workers, or an existing owner
    /// of this exact workspace/origin journal. Construction is inert before poll.
    pub fn open_workspace_managed_agents(
        &mut self,
        state: OwnedFd,
        preferences: NativeModelPreferences,
        origin: NativeSessionOrigin,
    ) -> BoxFuture<'_, Result<NativeManagedAgents, NativeManagedAgentsError>> {
        Box::pin(async move {
            self.validate_managed_open()?;
            let workspace = self
                .workspace_binding
                .as_ref()
                .ok_or(NativeManagedAgentsError::Configuration)?
                .authority
                .snapshot()
                .map_err(|_| NativeManagedAgentsError::Configuration)?;
            let workers = self
                .services
                .control_workers
                .as_ref()
                .ok_or(NativeManagedAgentsError::Configuration)?
                .clone();
            let directory = ManagedJournal::workspace_directory(
                state,
                workspace.primary_identity().to_owned(),
                origin,
                workers,
            )
            .await
            .map_err(|error| match error {
                crate::managed::store::JournalError::Invalid => {
                    NativeManagedAgentsError::Configuration
                }
                _ => NativeManagedAgentsError::Persistence,
            })?;
            self.open_managed_agents(directory, preferences, origin)
                .await
        })
    }

    pub(crate) fn prepare_managed_foreground(
        &self,
        agents: &NativeManagedAgents,
        conversation: NativeConversation,
        workspace: Option<NativeWorkspaceScopeSnapshot>,
        policy: Option<NativePermissionPolicySnapshot>,
        preferences: NativeModelPreferences,
    ) -> BoxFuture<'static, Result<PreparedManagedRuntime, NativeManagedAgentsError>> {
        let selection = self.managed_foreground_authority(workspace, policy);
        let preparation = selection.map(|(workspace, policy)| {
            agents.prepare_foreground(conversation, workspace, policy, preferences)
        });
        Box::pin(async move { preparation?.await.map_err(map_error) })
    }

    pub(crate) fn managed_foreground_authority(
        &self,
        workspace: Option<NativeWorkspaceScopeSnapshot>,
        policy: Option<NativePermissionPolicySnapshot>,
    ) -> Result<
        (NativeWorkspaceScopeSnapshot, NativePermissionPolicySnapshot),
        NativeManagedAgentsError,
    > {
        let workspace = match workspace {
            Some(workspace) => workspace,
            None => self
                .workspace_binding
                .as_ref()
                .ok_or(NativeManagedAgentsError::Configuration)?
                .authority
                .snapshot()
                .map_err(|_| NativeManagedAgentsError::Configuration)?,
        };
        let policy = match policy {
            Some(policy) => policy,
            None => super::configured_permission_policy(
                self.loaded_config.config(),
                &self.workspace_root,
            )
            .map_err(|_| NativeManagedAgentsError::Configuration)?,
        };
        Ok((workspace, policy))
    }

    /// Opens one manager using this host's existing engine, workers and weak routes.
    /// The directory descriptor must identify a private managed-journal directory.
    /// Construction is inert before poll. Failed validation or journal opening
    /// leaves the original host assembly available; successful opening transfers
    /// it once to the returned outer owner, never into shared engine services.
    ///
    /// # Errors
    /// Missing managed selection, invalid captured authority, capacity, or failure
    /// to acquire the exact journal's exclusive owner. No provider is polled.
    pub fn open_managed_agents(
        &mut self,
        directory: OwnedFd,
        preferences: NativeModelPreferences,
        origin: NativeSessionOrigin,
    ) -> BoxFuture<'_, Result<NativeManagedAgents, NativeManagedAgentsError>> {
        Box::pin(async move {
            self.validate_managed_open()?;
            let assembly = self
                .managed
                .as_ref()
                .ok_or(NativeManagedAgentsError::Configuration)?;
            let workspace = self
                .workspace_binding
                .as_ref()
                .ok_or(NativeManagedAgentsError::Configuration)?;
            let factory = Arc::new(
                SharedManagedRuntimeFactory::new(SharedManagedRuntimeFactoryOptions {
                    prompts: assembly.prompts.clone(),
                    services: self.services.clone(),
                    principals: assembly.principals.clone(),
                    scheduler: assembly.scheduler.clone(),
                    mcp: assembly.mcp.clone(),
                    notices: assembly.notices.clone(),
                    workspace_contexts: workspace.contexts.clone(),
                    restoration: ManagedRestorationAuthority {
                        workspace: workspace
                            .authority
                            .snapshot()
                            .map_err(|_| NativeManagedAgentsError::Configuration)?,
                        policy: super::configured_permission_policy(
                            self.loaded_config.config(),
                            &self.workspace_root,
                        )
                        .map_err(|_| NativeManagedAgentsError::Configuration)?,
                        preferences,
                    },
                    reserved_tool_names: self.reserved_tool_names.to_vec(),
                    origin,
                    cleanup_timeout: Duration::from_secs(30),
                })
                .map_err(map_error)?,
            );
            let journal = ManagedJournal::open(
                directory,
                self.services
                    .control_workers
                    .as_ref()
                    .ok_or(NativeManagedAgentsError::Configuration)?
                    .clone(),
                JournalLimits::default(),
            )
            .await
            .map_err(|_| NativeManagedAgentsError::Persistence)?;
            let mut assembly = self
                .managed
                .take()
                .ok_or(NativeManagedAgentsError::Configuration)?;
            let manager = ManagedManager::new(
                journal.clone(),
                assembly
                    .mailbox
                    .take()
                    .ok_or(NativeManagedAgentsError::Configuration)?,
                factory.clone(),
                assembly.relationships,
                assembly.notices,
                assembly.clock,
                ManagerLimits::default(),
            )
            .map_err(map_error)?;
            Ok(NativeManagedAgents {
                history: Box::default(),
                manager,
                factory,
                parent_mcp: Arc::new(
                    assembly
                        .parent_mcp
                        .take()
                        .ok_or(NativeManagedAgentsError::Configuration)?,
                ),
                journal,
            })
        })
    }
}

impl NativeManagedAgents {
    /// Queues one bounded journal observation. The manager owns the read through
    /// completion even if presentation is abandoned. No child runtime is loaded.
    /// # Errors
    /// Rejects shutdown, a pending/unconsumed page, invalid bounds or foreign cursor.
    pub fn request_catalog(
        &mut self,
        filter: crate::NativeManagedCatalogFilter,
        cursor: Option<crate::NativeManagedCatalogCursor>,
        limit: usize,
    ) -> Result<crate::NativeManagedCatalogRequest, crate::NativeManagedCatalogError> {
        self.manager.request_catalog(filter, cursor, limit)
    }

    #[must_use]
    pub fn take_catalog_outcome(&mut self) -> Option<crate::NativeManagedCatalogOutcome> {
        self.manager.take_catalog_outcome()
    }

    pub(crate) fn request_observed_human_command(
        &mut self,
        selection: &ManagedForegroundSelection,
        observed: crate::NativeObservedManagedAgent,
        command: machine_god_core::ManagedSubagentCommand,
        cancellation: machine_god_core::CancellationToken,
    ) -> Result<NativeManagedCommandResponse, machine_god_core::ManagedSubagentError> {
        self.manager.request_observed_human_command(
            selection,
            Some(observed),
            command,
            cancellation,
        )
    }

    pub(crate) fn request_human_command(
        &mut self,
        selection: &ManagedForegroundSelection,
        command: machine_god_core::ManagedSubagentCommand,
        cancellation: machine_god_core::CancellationToken,
    ) -> Result<NativeManagedCommandResponse, machine_god_core::ManagedSubagentError> {
        self.manager
            .request_human_command(selection, command, cancellation)
    }

    pub(crate) fn manages_prompt_inbox(
        &self,
        inbox: &crate::NativeInteractivePromptInbox,
    ) -> Result<bool, NativeManagedAgentsError> {
        self.factory.manages_prompt_inbox(inbox)
    }

    pub(crate) fn belongs_to(&self, host: &NativeReferenceHost) -> bool {
        self.factory.belongs_to(&host.services)
    }

    pub(crate) fn is_closing(&self) -> bool {
        self.manager.is_closing()
    }

    /// Observes current resident agents without loading history or starting work.
    #[must_use]
    pub fn agents(&self) -> Vec<NativeManagedAgentView> {
        self.manager
            .children()
            .into_iter()
            .map(|child| NativeManagedAgentView {
                id: child.id,
                generation: child.generation,
                name: child.name,
                state: child.state,
                selection: NativeManagedAgentSelection(child.selection),
            })
            .collect()
    }

    /// Read-only counters, not admission capacity or a command receipt.
    #[must_use]
    pub fn progress_snapshot(&self) -> NativeManagedAgentsProgress {
        self.manager.progress().into()
    }

    /// Progress does not depend on terminal output or a selected agent page.
    /// # Errors
    /// Reports fixed resource/persistence categories without discarding owned work.
    pub fn poll_progress(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Poll<Result<NativeManagedAgentsProgress, NativeManagedAgentsError>> {
        self.history.poll(cx);
        self.manager
            .poll_progress(cx, now_ms)
            .map(|result| result.map(Into::into).map_err(map_error))
    }

    /// Explicitly retry retained reconciliation receipts, never blind creation.
    pub fn retry_reconciliation(&self) {
        self.manager.retry_reconciliation();
    }

    /// Host teardown is not a durable user cancellation command.
    pub fn request_shutdown(&mut self) {
        self.history.close();
        self.manager.request_shutdown();
    }

    /// # Errors
    /// Actual child, foreground, control or worker cleanup failed. A display or
    /// stream completion alone can never produce a successful shutdown receipt.
    pub fn poll_shutdown(
        &mut self,
        cx: &mut Context<'_>,
        now_ms: i64,
    ) -> Poll<Result<(), NativeManagedAgentsError>> {
        if !self.history.is_closed() {
            self.request_shutdown();
        }
        self.history.poll(cx);
        let settled = self.manager.poll_shutdown(cx, now_ms).map_err(map_error);
        if !self.history.settled() {
            if let Poll::Ready(Err(error)) = settled {
                return Poll::Ready(Err(error));
            }
            return Poll::Pending;
        }
        settled
    }

    pub(crate) fn selected_runtime(
        &self,
        selection: &NativeManagedAgentSelection,
    ) -> Option<&Arc<NativeConversationRuntime>> {
        self.manager.selected_runtime(&selection.0)
    }

    pub(crate) fn observed_runtime(
        &self,
        observed: &crate::NativeObservedManagedAgent,
    ) -> Option<&Arc<NativeConversationRuntime>> {
        let selection = NativeManagedAgentSelection(self.manager.observed_selection(observed)?);
        self.selected_runtime(&selection)
    }

    pub(crate) fn prepare_foreground(
        &self,
        conversation: NativeConversation,
        workspace: NativeWorkspaceScopeSnapshot,
        policy: NativePermissionPolicySnapshot,
        preferences: NativeModelPreferences,
    ) -> BoxFuture<'static, Result<PreparedManagedRuntime, ManagedRuntimeError>> {
        let principal = NoticePrincipal {
            id: conversation.id().to_string(),
            generation: NonZeroU64::MIN,
        };
        self.factory.prepare_parent(
            conversation,
            ManagedRestorationAuthority {
                workspace,
                policy,
                preferences,
            },
            principal,
            self.journal.owner_lease(),
            self.parent_mcp.clone(),
        )
    }

    pub(crate) fn enroll_foreground(
        &mut self,
        prepared: Box<PreparedManagedRuntime>,
        reservation: &ManagedForegroundReservation,
    ) -> Result<ManagedForegroundSelection, (ManagedRuntimeError, Box<PreparedManagedRuntime>)>
    {
        self.manager.enroll_foreground(prepared, reservation)
    }

    pub(crate) fn stage_foreground(
        &mut self,
        prepared: Box<PreparedManagedRuntime>,
        reservation: &ManagedForegroundReservation,
    ) -> Result<ManagedForegroundSelection, (ManagedRuntimeError, Box<PreparedManagedRuntime>)>
    {
        self.manager.stage_foreground(prepared, reservation)
    }

    pub(crate) fn activate_foreground(
        &mut self,
        selection: &ManagedForegroundSelection,
    ) -> Result<(), NativeManagedAgentsError> {
        self.manager
            .activate_foreground(selection)
            .map_err(map_error)
    }

    pub(crate) fn reserve_foreground(
        &mut self,
    ) -> Result<ManagedForegroundReservation, NativeManagedAgentsError> {
        self.manager.reserve_foreground().map_err(map_error)
    }

    pub(crate) fn poll_foreground_reservation(
        &self,
        reservation: &ManagedForegroundReservation,
        cx: &Context<'_>,
    ) -> Poll<Result<(), NativeManagedAgentsError>> {
        self.manager
            .poll_foreground_reservation(reservation, cx)
            .map_err(map_error)
    }

    #[cfg(test)]
    pub(crate) fn foreground_runtime(
        &self,
        selected: &ManagedForegroundSelection,
    ) -> Option<&Arc<NativeConversationRuntime>> {
        self.manager.foreground_runtime(selected)
    }

    pub(crate) fn retire_foreground(&mut self, selected: &ManagedForegroundSelection) -> bool {
        self.manager.retire_foreground(selected)
    }

    pub(crate) fn quiesce_foreground(
        &mut self,
        selected: &ManagedForegroundSelection,
    ) -> Result<crate::NativeRuntimeQuiescence, crate::NativeConversationRuntimeError> {
        self.manager.quiesce_foreground(selected)
    }

    pub(crate) fn foreground_mcp_controls(
        &self,
        selected: &ManagedForegroundSelection,
    ) -> Option<crate::managed::manager::factory::ManagedMcpControls> {
        self.manager.foreground_mcp_controls(selected)
    }

    pub(crate) fn foreground_turn_settled(&self, selected: &ManagedForegroundSelection) -> bool {
        self.manager.foreground_turn_settled(selected)
    }
}

pub(crate) fn map_error(error: ManagedRuntimeError) -> NativeManagedAgentsError {
    match error {
        ManagedRuntimeError::Invalid => NativeManagedAgentsError::Configuration,
        ManagedRuntimeError::Capacity => NativeManagedAgentsError::Capacity,
        ManagedRuntimeError::Persistence => NativeManagedAgentsError::Persistence,
        ManagedRuntimeError::Ambiguous => NativeManagedAgentsError::Ambiguous,
        ManagedRuntimeError::Missing | ManagedRuntimeError::Unavailable => {
            NativeManagedAgentsError::Unavailable
        }
    }
}
