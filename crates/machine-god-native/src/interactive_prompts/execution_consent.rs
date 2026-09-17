//! Execution-time consent is not a policy grant or a reconstructed permission review.

use machine_god_core::{CancellationToken, Capability, ToolCall, ToolContext, TurnWitness};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::Arc;
use std::sync::{
    Weak,
    atomic::{AtomicBool, Ordering},
};

/// An immutable exact proposal originating in an authenticated, still-owned
/// model invocation. There is deliberately no public constructor, clone, or
/// deserializer: matching structural IDs cannot manufacture consent authority.
pub struct NativeExecutionConsentRequest {
    source: ExecutionConsentSource,
    capability: Capability,
    reason: String,
}

pub(crate) struct ExecutionConsentSource {
    context: ToolContext,
    call: ToolCall,
    job: Weak<AtomicBool>,
    turn: TurnWitness,
    cancellation: CancellationToken,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    principal: crate::managed::principal::NativePrincipalTurnStamp,
}

impl ExecutionConsentSource {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn from_admitted(
        invocation: &machine_god_core::ManagedSubagentInvocation,
        lease: &crate::managed::principal::NativeManagedCallLease,
        job: &Arc<AtomicBool>,
        cancellation: CancellationToken,
    ) -> Option<Self> {
        if !lease.is_live() || cancellation.is_cancelled() {
            return None;
        }
        let machine_god_core::ManagedSubagentCommand::Relationship(command) = invocation.command()
        else {
            return None;
        };
        if !matches!(
            command.action,
            machine_god_core::ManagedRelationshipAction::Attach
                | machine_god_core::ManagedRelationshipAction::Reparent
        ) {
            return None;
        }
        let context = invocation.context().clone();
        Some(Self {
            call: ToolCall {
                id: context.call_id.clone(),
                name: invocation.tool_name().clone(),
                arguments: invocation.arguments().clone(),
            },
            context,
            job: Arc::downgrade(job),
            turn: lease.witness().clone(),
            cancellation,
            principal: lease.consent_stamp(),
        })
    }

    #[cfg(all(
        feature = "ai-gateway-http",
        any(target_os = "linux", target_os = "macos")
    ))]
    pub(crate) fn request(
        self,
        capability: Capability,
        reason: String,
    ) -> NativeExecutionConsentRequest {
        NativeExecutionConsentRequest {
            source: self,
            capability,
            reason,
        }
    }
}

impl NativeExecutionConsentRequest {
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
    #[must_use]
    pub const fn capability(&self) -> &Capability {
        &self.capability
    }
    /// Display/correlation labels only; these cannot mint a new request.
    #[must_use]
    pub const fn context(&self) -> &ToolContext {
        &self.source.context
    }
    pub(crate) fn call(&self) -> &ToolCall {
        &self.source.call
    }
    pub(crate) fn is_live(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if !self.source.principal.execution_is_live() {
            return false;
        }
        self.source
            .job
            .upgrade()
            .is_some_and(|live| live.load(Ordering::Acquire))
            && self.source.turn.is_live()
            && !self.source.cancellation.is_cancelled()
    }
}

impl std::fmt::Debug for NativeExecutionConsentRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeExecutionConsentRequest { .. }")
    }
}
