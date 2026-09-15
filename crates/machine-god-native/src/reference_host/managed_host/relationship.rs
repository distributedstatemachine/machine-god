//! Human consent for one exact admitted relationship proposal, never a reusable grant.
use crate::managed::{
    manager::factory::{
        ManagedRelationshipAuthorizer, ManagedRelationshipProposal, ManagedRuntimeError,
    },
    principal::NativePrincipalRequester,
};
use crate::{PermissionPromptDecision, PermissionPrompter};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ManagedRelationshipAction, PermissionRequest,
    PermissionRequestId, PermissionRisk,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(in crate::reference_host) struct RelationshipConsent {
    principals: NativePrincipalRequester,
    prompter: Arc<dyn PermissionPrompter>,
    sequence: Arc<AtomicU64>,
}
impl RelationshipConsent {
    pub(in crate::reference_host) fn new(
        principals: NativePrincipalRequester,
        prompter: Arc<dyn PermissionPrompter>,
    ) -> Self {
        Self {
            principals,
            prompter,
            sequence: Arc::new(AtomicU64::new(1)),
        }
    }
}
impl ManagedRelationshipAuthorizer for RelationshipConsent {
    fn authorize(
        &self,
        proposal: ManagedRelationshipProposal,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<bool, ManagedRuntimeError>> {
        let principals = self.principals.clone();
        let prompter = self.prompter.clone();
        let sequence = self.sequence.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(ManagedRuntimeError::Unavailable);
            }
            validate(&proposal)?;
            let stamp = principals
                .stamp(&proposal.context)
                .map_err(|_| ManagedRuntimeError::Unavailable)?;
            if !stamp.matches_principal(&proposal.origin.principal) {
                return Err(ManagedRuntimeError::Unavailable);
            }
            let id = sequence
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| ManagedRuntimeError::Capacity)?;
            let request = PermissionRequest {
                id: PermissionRequestId::new(format!("managed-relationship-{id}"))
                    .map_err(|_| ManagedRuntimeError::Invalid)?,
                session_id: proposal.context.session_id,
                session_incarnation_id: proposal.context.session_incarnation_id,
                turn_id: proposal.context.turn_id,
                capability: Capability::Custom {
                    name: "managed_relationship".into(),
                    details: serde_json::json!({
                        "call_id": proposal.context.call_id,
                        "action": proposal.action,
                        "child_id": proposal.child_id,
                        "child_generation": proposal.generation,
                        "child_revision": proposal.revision,
                        "previous_parent": proposal.previous_parent,
                        "parent": proposal.parent,
                    }),
                },
                risk: PermissionRisk::High,
                reason: "Approve this exact managed-agent parent relationship change.".into(),
            };
            let decision = match futures_util::future::select(
                prompter.prompt(request),
                Box::pin(cancellation.cancelled()),
            )
            .await
            {
                futures_util::future::Either::Left((decision, _)) => {
                    decision.map_err(|_| ManagedRuntimeError::Unavailable)?
                }
                futures_util::future::Either::Right(_) => {
                    return Err(ManagedRuntimeError::Unavailable);
                }
            };
            if cancellation.is_cancelled() || !stamp.matches_principal(&proposal.origin.principal) {
                return Err(ManagedRuntimeError::Unavailable);
            }
            // Even AllowTurn/AllowSession approves only this frozen proposal.
            // No grant is registered in the permission controller or copied to a child.
            Ok(decision != PermissionPromptDecision::Deny)
        })
    }
}

fn validate(proposal: &ManagedRelationshipProposal) -> Result<(), ManagedRuntimeError> {
    if proposal.generation == 0
        || proposal.revision == 0
        || proposal.child_id.is_empty()
        || proposal.child_id.len() > 255
        || matches!(proposal.child_id.as_str(), "." | "..")
        || !proposal
            .child_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        || !matches!(
            proposal.action,
            ManagedRelationshipAction::Attach | ManagedRelationshipAction::Reparent
        )
    {
        return Err(ManagedRuntimeError::Invalid);
    }
    Ok(())
}
