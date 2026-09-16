//! One selected native ACP session, with owned preparation and retirement.

use super::session::{AcpSessionError, NativeAcpSession, NativeAcpSessionSelection};
use crate::mcp::ephemeral::{NativeMcpEphemeralConfiguration, NativeMcpNetworkRequirement};
use crate::{
    NativeInteractiveSessionOptions, NativePermissionContexts, NativeReferenceHost,
    NativeRuntimeQuiescence, NativeSessionCatalogCursor, NativeSessionCatalogPage,
    NativeSessionCatalogReadError,
};
use machine_god_core::{
    BackgroundOutputOwner, BoxFuture, CancellationToken, EngineEvent, SessionId,
};
use std::{
    fmt,
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll, Waker},
};

mod cleanup;
mod driver;
mod managed;
mod reuse;
mod reuse_driver;
pub use reuse::NativeAcpHostReuse;
mod rollback;
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
type AfterOpenHook = Box<dyn FnOnce(&NativeReferenceHost, &CancellationToken) + Send>;

/// Trusted, explicitly captured host effects. Implementations must keep any
/// admitted worker and unsuccessful preparation cleanup owned until completion.
/// Preparation receives only the network requirement derived from the admitted
/// MCP selection, not another copy of its secret-bearing configuration. This
/// requirement grants no authority and never replaces peer readiness checks.
pub trait NativeAcpHostFactory: Send + Sync {
    /// Checks explicit requested roots against an existing managed host before
    /// preparing a new domain. `None` selects fresh composition, never a guessed
    /// path-only reuse. The default is for explicitly unmanaged factories.
    fn prepare_reuse(
        &self,
        _current: Arc<NativeReferenceHost>,
        _workspace: PathBuf,
        _network: NativeMcpNetworkRequirement,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<Option<NativeAcpHostReuse>, AcpSessionError>> {
        Box::pin(async { Ok(None) })
    }

    fn prepare(
        &self,
        workspace: PathBuf,
        network: NativeMcpNetworkRequirement,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>>;
    fn list(
        &self,
        workspace: Option<PathBuf>,
        cursor: Option<NativeSessionCatalogCursor>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>>;
}

/// A fresh dedicated host; no peer publication or native session yet.
pub struct NativeAcpPreparedHost {
    host: Arc<NativeReferenceHost>,
    options: NativeInteractiveSessionOptions,
    permission_contexts: Arc<NativePermissionContexts>,
    workspace: NativeAcpWorkspaceIdentity,
    managed: Option<Box<managed::Preparation>>,
}

/// A request spelling bound to the descriptor-checked primary used by composition.
/// Retaining the scope pins the actual object, not a path that can be retargeted.
pub struct NativeAcpWorkspaceIdentity {
    requested: PathBuf,
    scope: crate::NativeWorkspaceScopeSnapshot,
}
impl NativeAcpWorkspaceIdentity {
    /// Captures on the explicitly owned preparation worker, before consuming roots.
    /// Validates existing descriptors only; never resolves or opens another path.
    /// # Errors
    /// Rejects invalid request paths or a scope not backed by the prepared roots.
    pub fn capture(
        roots: &crate::PreparedNativeRoots,
        authority: &crate::NativeWorkspaceAuthority,
    ) -> Result<Self, AcpSessionError> {
        crate::NativeSessionMetadata::new(
            roots.workspace_root(),
            0,
            crate::NativeSessionOrigin::Acp,
        )
        .map_err(|_| AcpSessionError::InvalidConfiguration)?;
        let scope = authority
            .snapshot()
            .map_err(|_| AcpSessionError::Unavailable)?;
        let primary = roots
            .try_clone_workspace()
            .map_err(|_| AcpSessionError::Unavailable)?;
        let state = roots
            .try_clone_state()
            .map_err(|_| AcpSessionError::Unavailable)?;
        scope
            .validate_host_binding(&primary, roots.canonical_workspace_root(), &state)
            .map_err(|_| AcpSessionError::InvalidConfiguration)?;
        Ok(Self {
            requested: roots.workspace_root().to_owned(),
            scope,
        })
    }
}
impl NativeAcpPreparedHost {
    /// Pure validation; no configuration, credential or environment discovery.
    /// # Errors
    /// Rejects mismatched options/contexts or a missing/already published owner.
    pub fn new(
        host: Arc<NativeReferenceHost>,
        options: NativeInteractiveSessionOptions,
        permission_contexts: Arc<NativePermissionContexts>,
        workspace: NativeAcpWorkspaceIdentity,
    ) -> Result<Self, AcpSessionError> {
        let value = Self {
            host,
            options,
            permission_contexts,
            workspace,
            managed: None,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), AcpSessionError> {
        self.validate_binding()?;
        if self.managed.is_some() {
            return if self.host.managed_agents_selected() {
                Ok(())
            } else {
                Err(AcpSessionError::InvalidConfiguration)
            };
        }
        if self.host.mcp_ephemeral_owner().is_none()
            || self.host.mcp_management().is_some()
            || self.host.mcp_controller().is_some()
            || !self
                .host
                .mcp_runtime()
                .ok_or(AcpSessionError::InvalidConfiguration)?
                .publication_checkpoint()
                .map_err(|_| AcpSessionError::Unavailable)?
                .is_unpublished()
        {
            return Err(AcpSessionError::InvalidConfiguration);
        }
        Ok(())
    }
    fn validate_binding(&self) -> Result<(), AcpSessionError> {
        self.options.validate_for_host(&self.host)?;
        if self.workspace.scope.primary_identity() != self.host.workspace_root()
            || !self.host.has_workspace_primary(&self.workspace.scope)
        {
            return Err(AcpSessionError::InvalidConfiguration);
        }
        let contexts = self
            .host
            .permission_contexts()
            .ok_or(AcpSessionError::InvalidConfiguration)?;
        if !Arc::ptr_eq(&contexts, &self.permission_contexts) {
            return Err(AcpSessionError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeAcpSelectionId(pub u64);

pub struct NativeAcpSelectionTurnOutcome {
    pub owner: BackgroundOutputOwner,
    pub outcome: Result<EngineEvent, crate::NativeInteractiveError>,
}

/// Publication and cleanup receipts, never mere request acknowledgements.
pub enum NativeAcpSelectionOutcome {
    Selected {
        id: NativeAcpSelectionId,
        previous: Option<BackgroundOutputOwner>,
        current: BackgroundOutputOwner,
    },
    Closed {
        id: NativeAcpSelectionId,
        previous: Option<BackgroundOutputOwner>,
    },
    Rejected {
        id: NativeAcpSelectionId,
        error: AcpSessionError,
        old_preserved: bool,
        candidate_may_have_persisted: bool,
    },
    Indeterminate {
        id: NativeAcpSelectionId,
        error: AcpSessionError,
        previous: Option<BackgroundOutputOwner>,
        candidate: Option<BackgroundOutputOwner>,
    },
}

struct Current {
    session: NativeAcpSession,
    host: Arc<NativeReferenceHost>,
    permission_contexts: Arc<NativePermissionContexts>,
}
struct Request {
    id: NativeAcpSelectionId,
    selection: Option<NativeAcpSessionSelection>,
    workspace: PathBuf,
    configuration: Option<NativeMcpEphemeralConfiguration>,
    now_ms: i64,
    cancellation: CancellationToken,
    previous: Option<BackgroundOutputOwner>,
    candidate_may_have_persisted: bool,
    rollback_guard: Option<NativeRuntimeQuiescence>,
}
struct Pending {
    request: Request,
    phase: Phase,
}
enum Phase {
    Requested,
    CheckingReuse(BoxFuture<'static, Result<Option<NativeAcpHostReuse>, AcpSessionError>>),
    ReuseStarting(Box<reuse::Stage>),
    ReuseDraining(Box<reuse::Stage>),
    ReuseTransition {
        request: crate::NativeInteractiveRequestId,
        replay: bool,
    },
    Preparing(BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>>),
    Starting {
        host: NativeAcpPreparedHost,
        future: BoxFuture<'static, Result<(), AcpSessionError>>,
    },
    ManagedStarting(NativeAcpPreparedHost),
    Draining(Option<NativeAcpPreparedHost>),
    Quiescing {
        host: Option<NativeAcpPreparedHost>,
        future: BoxFuture<'static, Result<NativeRuntimeQuiescence, AcpSessionError>>,
    },
    Opening {
        host: NativeAcpPreparedHost,
        guard: Option<NativeRuntimeQuiescence>,
        future: BoxFuture<'static, Result<NativeAcpSession, AcpSessionError>>,
    },
    ManagedOpening {
        host: NativeAcpPreparedHost,
        guard: Option<NativeRuntimeQuiescence>,
    },
    Retiring {
        candidate: Option<Box<Current>>,
        future: BoxFuture<'static, cleanup::Receipt>,
    },
    Rejecting {
        error: AcpSessionError,
        future: Option<BoxFuture<'static, cleanup::Receipt>>,
    },
    Revalidating {
        error: AcpSessionError,
        future: BoxFuture<'static, bool>,
    },
}

/// A bounded native owner: one current, one candidate and one retained outcome.
/// Dropping response futures cannot abandon accepted selection work. Poll this
/// owner through cancellation/EOF to obtain actual retirement observations.
pub struct NativeAcpSelectionOwner {
    factory: Arc<dyn NativeAcpHostFactory>,
    current: Option<Current>,
    pending: Option<Pending>,
    outcome: Option<NativeAcpSelectionOutcome>,
    turn_outcome: Option<NativeAcpSelectionTurnOutcome>,
    reuse_outcome: Option<crate::NativeInteractiveOutcome>,
    retained_cleanup: Vec<cleanup::Receipt>,
    fenced_candidate: Option<Current>,
    fenced_guard: Option<NativeRuntimeQuiescence>,
    next_id: u64,
    shutdown: bool,
    fenced: bool,
    wake: Option<Waker>,
    #[cfg(test)]
    after_open: Option<AfterOpenHook>,
}
impl NativeAcpSelectionOwner {
    #[must_use]
    pub fn new(factory: Arc<dyn NativeAcpHostFactory>) -> Self {
        Self {
            factory,
            current: None,
            pending: None,
            outcome: None,
            turn_outcome: None,
            reuse_outcome: None,
            retained_cleanup: Vec::new(),
            fenced_candidate: None,
            fenced_guard: None,
            next_id: 1,
            shutdown: false,
            fenced: false,
            wake: None,
            #[cfg(test)]
            after_open: None,
        }
    }
    /// Stores bounded inert intent. Factory effects begin only during polling.
    /// # Errors
    /// Rejects another retained operation/outcome, closure or invalid workspace.
    pub fn request(
        &mut self,
        selection: NativeAcpSessionSelection,
        workspace: PathBuf,
        configuration: NativeMcpEphemeralConfiguration,
        now_ms: i64,
    ) -> Result<NativeAcpSelectionId, AcpSessionError> {
        crate::NativeSessionMetadata::new(&workspace, now_ms, crate::NativeSessionOrigin::Acp)
            .map_err(|_| AcpSessionError::InvalidConfiguration)?;
        self.enqueue_request(Some(selection), workspace, Some(configuration), now_ms)
    }
    /// Closes the exact selected session, not its durable history or connection.
    /// # Errors
    /// Rejects a foreign identity or another retained operation/outcome.
    pub fn request_close(
        &mut self,
        expected: &SessionId,
    ) -> Result<NativeAcpSelectionId, AcpSessionError> {
        let current = self.current.as_ref().ok_or(AcpSessionError::WrongSession)?;
        if current.session.id() != *expected {
            return Err(AcpSessionError::WrongSession);
        }
        let workspace = current.host.workspace_root().to_path_buf();
        self.enqueue_request(None, workspace, None, 0)
    }
    fn enqueue_request(
        &mut self,
        selection: Option<NativeAcpSessionSelection>,
        workspace: PathBuf,
        configuration: Option<NativeMcpEphemeralConfiguration>,
        now_ms: i64,
    ) -> Result<NativeAcpSelectionId, AcpSessionError> {
        if self.shutdown {
            return Err(AcpSessionError::Closed);
        }
        if self.pending.is_some()
            || self.outcome.is_some()
            || self.fenced
            || self
                .current
                .as_ref()
                .is_some_and(|current| current.session.has_pending_model_save())
        {
            return Err(AcpSessionError::Busy);
        }
        let id = NativeAcpSelectionId(self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(AcpSessionError::Limit)?;
        self.pending = Some(Pending {
            request: Request {
                id,
                selection,
                workspace,
                configuration,
                now_ms,
                cancellation: CancellationToken::new(),
                previous: self
                    .current
                    .as_ref()
                    .map(|current| current.session.principal()),
                candidate_may_have_persisted: false,
                rollback_guard: None,
            },
            phase: Phase::Requested,
        });
        self.notify();
        Ok(id)
    }
    pub fn cancel_pending(&mut self) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        pending.request.cancellation.cancel();
        self.notify();
        true
    }
    /// Cancels only the exact selected prompt, including during replacement.
    /// # Errors
    /// Rejects an identity other than the current native session.
    pub fn request_cancel(&mut self, expected: &SessionId) -> Result<bool, AcpSessionError> {
        let cancelled = self
            .current
            .as_mut()
            .ok_or(AcpSessionError::WrongSession)?
            .session
            .request_cancel(expected)?;
        self.notify();
        Ok(cancelled)
    }
    /// Terminal connection cutoff. Already accepted effects remain polled;
    /// this does not discard a turn result, selection receipt or cleanup job.
    pub fn request_shutdown(&mut self) {
        self.shutdown = true;
        self.cancel_pending();
        if let Some(current) = &mut self.current {
            let _ = current.session.request_cancel(&current.session.id());
        }
        self.notify();
    }
    #[must_use]
    pub fn current(&self) -> Option<&NativeAcpSession> {
        self.current.as_ref().map(|current| &current.session)
    }
    #[must_use]
    pub fn current_mut(&mut self) -> Option<&mut NativeAcpSession> {
        if self.is_busy() || self.fenced || self.shutdown {
            None
        } else {
            self.current.as_mut().map(|current| &mut current.session)
        }
    }
    #[must_use]
    pub fn current_host(&self) -> Option<&Arc<NativeReferenceHost>> {
        self.current.as_ref().map(|current| &current.host)
    }
    #[must_use]
    pub fn current_permission_contexts(&self) -> Option<Arc<NativePermissionContexts>> {
        self.current
            .as_ref()
            .map(|current| current.permission_contexts.clone())
    }
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.pending.is_some() || self.outcome.is_some()
    }
    #[must_use]
    pub const fn is_fenced(&self) -> bool {
        self.fenced
    }
    /// No selected actor or pending operation remains. Check retained outcomes
    /// separately: this observation alone is not a successful cleanup receipt.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shutdown
            && self.current.is_none()
            && self.pending.is_none()
            // Failed managed startup retains actual cleanup custody and cannot
            // masquerade as a settled connection merely because no session opened.
            && self.retained_cleanup.iter().all(|receipt| {
                receipt.managed.is_none() && receipt.staged.is_none()
            })
            && self
                .retained_cleanup
                .iter()
                .flat_map(|receipt| &receipt.workers)
                .all(crate::NativeOwnedWorkerCompletion::is_complete)
    }
    #[must_use]
    pub fn take_outcome(&mut self) -> Option<NativeAcpSelectionOutcome> {
        let value = self.outcome.take();
        if value.is_some() {
            self.notify();
        }
        value
    }
    #[must_use]
    pub fn take_turn_outcome(&mut self) -> Option<NativeAcpSelectionTurnOutcome> {
        let value = self.turn_outcome.take();
        if value.is_some() {
            self.notify();
        }
        value
    }
    /// Drains an exact accepted save even after connection shutdown fences mutation.
    #[must_use]
    pub fn take_model_save_outcome(
        &mut self,
    ) -> Option<Result<crate::NativeModelPreferencePersistence, AcpSessionError>> {
        let result = self.current.as_mut()?.session.take_model_save_outcome();
        if result.is_some() {
            self.notify();
        }
        result
    }
    /// Drains an exact command receipt while shutdown fences new mutations.
    #[must_use]
    pub fn take_command_control_outcome(
        &mut self,
    ) -> Option<crate::NativeInteractiveControlOutcome> {
        let result = self
            .current
            .as_mut()?
            .session
            .take_command_control_outcome();
        if result.is_some() {
            self.notify();
        }
        result
    }
    #[must_use]
    pub fn take_presentation(&mut self) -> Option<(BackgroundOutputOwner, EngineEvent)> {
        let current = self.current.as_mut()?;
        current
            .session
            .take_presentation()
            .map(|event| (current.session.principal(), event))
    }
    pub fn poll_progress(&mut self, cx: &mut Context<'_>, now_ms: i64) -> Poll<()> {
        self.drive(cx, now_ms)
    }
    fn notify(&mut self) {
        if let Some(wake) = self.wake.take() {
            wake.wake();
        }
    }
}
impl Drop for NativeAcpSelectionOwner {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.request.cancellation.cancel();
        }
        if let Some(current) = &mut self.current {
            let _ = current.session.request_cancel(&current.session.id());
            current.host.close_mcp();
        }
        // Drop is only a cutoff. No successful selection or cleanup receipt is
        // manufactured for work whose owner was abandoned.
    }
}
macro_rules! redacted {($($ty:ty),+)=>{$(impl fmt::Debug for $ty {fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{f.write_str(concat!(stringify!($ty)," { .. }"))}})+};}
redacted!(
    NativeAcpWorkspaceIdentity,
    NativeAcpPreparedHost,
    NativeAcpSelectionTurnOutcome,
    NativeAcpSelectionOutcome,
    NativeAcpSelectionOwner
);
