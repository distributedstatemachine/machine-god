//! Human consent for one exact admitted relationship proposal, never a reusable grant.
use crate::PermissionPrompter;
use crate::managed::{
    manager::factory::{
        ManagedRelationshipAuthorizer, ManagedRelationshipProposal, ManagedRuntimeError,
    },
    principal::NativePrincipalRequester,
};
use machine_god_core::{BoxFuture, CancellationToken, Capability, ManagedRelationshipAction};
use std::sync::Arc;

pub(crate) struct RelationshipConsent {
    principals: NativePrincipalRequester,
    prompter: Arc<dyn PermissionPrompter>,
}
impl RelationshipConsent {
    pub(crate) fn new(
        principals: NativePrincipalRequester,
        prompter: Arc<dyn PermissionPrompter>,
    ) -> Self {
        Self {
            principals,
            prompter,
        }
    }
}
impl ManagedRelationshipAuthorizer for RelationshipConsent {
    fn authorize(
        &self,
        mut proposal: ManagedRelationshipProposal,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<bool, ManagedRuntimeError>> {
        let principals = self.principals.clone();
        let prompter = self.prompter.clone();
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
            let source = proposal
                .consent
                .take()
                .ok_or(ManagedRuntimeError::Unavailable)?;
            let request = source.request(
                Capability::Custom {
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
                "Approve this exact managed-agent parent relationship change.".into(),
            );
            if !request.is_live() || request.context() != &proposal.context {
                return Err(ManagedRuntimeError::Unavailable);
            }
            let decision = match futures_util::future::select(
                prompter.prompt_execution_consent(request),
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
            // This boolean can approve only the exact frozen proposal. No
            // permission grant is registered or copied to a child.
            Ok(decision)
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
