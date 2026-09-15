//! Explicit preparation and actual cleanup, implemented by the shared native host.

use super::super::{
    conversation::ManagedConversationOwner,
    principal::NativePrincipal,
    scheduler::RunRef,
    store::{JournalOwner, JournalTranscript},
};
use crate::{
    NativeConversationRuntime, NativeModelPreferences, NativePermissionPolicySnapshot,
    NativeWorkspaceScopeSnapshot,
};
use machine_god_core::{
    BoxFuture, CancellationToken, ManagedConfiguration, ManagedRelationshipAction, ToolContext,
};
use std::{
    fmt,
    sync::Arc,
    task::{Context, Poll},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedRuntimeError {
    Missing,
    Unavailable,
    Capacity,
    Invalid,
    Persistence,
    Ambiguous,
}

/// Captured at actual call admission, never a later mutable workspace lookup.
#[derive(Clone)]
pub(crate) struct ManagedRuntimeOrigin {
    pub principal: Arc<NativePrincipal>,
    pub workspace: NativeWorkspaceScopeSnapshot,
    pub policy: NativePermissionPolicySnapshot,
    pub preferences: NativeModelPreferences,
}

#[derive(Clone)]
pub(crate) struct ManagedRuntimeRequest {
    pub kind: ManagedRuntimePreparationKind,
    pub child_id: String,
    pub generation: u64,
    pub transcript: JournalTranscript,
    /// Retain inside actual owned cleanup, even if the manager observer vanishes.
    pub journal_owner: JournalOwner,
    pub configuration: ManagedConfiguration,
    /// None restores this exact saved transcript under explicit factory authority.
    pub origin: Option<ManagedRuntimeOrigin>,
    pub now_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedRuntimePreparationKind {
    Create,
    Restore,
}

/// Shared host services implement this; never construct an engine per child.
/// Both returned futures must remain inert before their first poll.
pub(crate) trait ManagedRuntimeFactory: Send + Sync + 'static {
    fn allocate_identity(
        &self,
    ) -> BoxFuture<'static, Result<JournalTranscript, ManagedRuntimeError>>;
    /// Reserve residency/runtime and prepare an empty transcript before journal
    /// acceptance. This must never poll a provider or execute a model-facing tool.
    fn prepare(
        &self,
        request: ManagedRuntimeRequest,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<ManagedPreparation, ManagedRuntimeError>>;
}

pub(crate) struct ManagedRelationshipProposal {
    pub origin: ManagedRuntimeOrigin,
    pub context: ToolContext,
    pub child_id: String,
    pub generation: u64,
    pub revision: u64,
    pub action: ManagedRelationshipAction,
    pub previous_parent: Option<JournalTranscript>,
    pub parent: JournalTranscript,
}

pub(crate) trait ManagedRelationshipAuthorizer: Send + Sync + 'static {
    /// Ask the original principal's shared human inbox for this exact proposal.
    /// Permission modes and supplied public IDs never imply approval here.
    fn authorize(
        &self,
        proposal: ManagedRelationshipProposal,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<bool, ManagedRuntimeError>>;
}

pub(crate) enum ManagedPreparation {
    Ready(PreparedManagedRuntime),
    Ambiguous(Box<dyn ManagedPreparationReceipt>),
}

/// Exact candidate ownership survives indeterminate publication and read errors.
pub(crate) trait ManagedPreparationReceipt: Send + 'static {
    /// Some confirms preparation; None confirms nonpublication. Errors retain
    /// this receipt and never authorize another identity allocation or retry.
    fn poll_reconcile(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<PreparedManagedRuntime>, ManagedRuntimeError>>;
}

pub(crate) struct PreparedManagedRuntime {
    pub runtime: Arc<NativeConversationRuntime>,
    pub owner: ManagedConversationOwner,
    pub resources: Box<dyn ManagedRuntimeResources>,
    pub notice_context: Option<Arc<super::super::prompt_context::ParentNoticeContext>>,
}

/// Per-principal/run cleanup custody, not a shared global completion observer.
pub(crate) trait ManagedRuntimeResources: Send + 'static {
    fn mcp_controls(&self) -> Option<ManagedMcpControls> {
        None
    }
    /// Confirm readiness/auth/checkpoint cleanup even when no `RunRef` was minted.
    fn poll_admission_settled(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), ManagedRuntimeError>>;
    /// Confirm original attributed worker, TLS and process-reap obligations.
    /// Runtime stream completion alone does not satisfy this method.
    fn poll_turn_settled(
        &mut self,
        cx: &mut Context<'_>,
        run: &RunRef,
    ) -> Poll<Result<(), ManagedRuntimeError>>;
    /// Idempotently start retiring this principal's controls and resources only.
    fn begin_close(&mut self);
    fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), ManagedRuntimeError>>;
}

/// Exact principal selection for native human controls, never model admission.
pub(crate) struct ManagedMcpControls {
    pub runtime: Option<Arc<crate::mcp::runtime::NativeMcpRuntime>>,
    pub controller: Option<Arc<crate::mcp::controller::NativeMcpController>>,
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(impl fmt::Debug for $ty {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
redacted_debug!(
    ManagedRuntimeOrigin,
    ManagedRuntimeRequest,
    ManagedPreparation,
    PreparedManagedRuntime,
    ManagedRelationshipProposal
);
