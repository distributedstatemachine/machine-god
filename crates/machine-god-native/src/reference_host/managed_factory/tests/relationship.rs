use super::FactoryFixture;
use crate::managed::{
    manager::factory::{ManagedRelationshipAuthorizer, ManagedRelationshipProposal},
    store::JournalTranscript,
};
use crate::reference_host::RelationshipConsent;
use crate::{PermissionPromptDecision, PermissionPromptError, PermissionPrompter};
use futures_executor::block_on;
use machine_god_core::{
    BoxFuture, CancellationToken, ManagedRelationshipAction, PermissionRequest, ToolCallId,
    ToolContext,
};
use std::sync::Arc;

struct NeverPrompt;
impl PermissionPrompter for NeverPrompt {
    fn prompt(
        &self,
        _: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        panic!("execution consent must not use the ordinary policy prompt");
    }
    fn prompt_execution_consent(
        &self,
        _: crate::NativeExecutionConsentRequest,
    ) -> BoxFuture<'_, Result<bool, PermissionPromptError>> {
        panic!("structural contexts cannot acquire execution consent");
    }
}

#[test]
fn live_structural_context_without_admitted_invocation_cannot_request_consent() {
    let f = FactoryFixture::new();
    let turn = block_on(f.parent_session.prompt("relationship request")).unwrap();
    let origin = f.request("child").origin.unwrap();
    let _guard = f
        .parent
        .begin_turn(
            &turn,
            origin.policy.clone(),
            origin.preferences.clone(),
            None,
        )
        .unwrap();
    let proposal = || ManagedRelationshipProposal {
        consent: None,
        origin: f.request("child").origin.unwrap(),
        context: ToolContext {
            session_id: f.parent_session.id(),
            session_incarnation_id: f.parent_session.incarnation_id(),
            turn_id: turn.id().clone(),
            call_id: ToolCallId::new("invented-call").unwrap(),
        },
        child_id: "child".into(),
        generation: 7,
        revision: 11,
        action: ManagedRelationshipAction::Reparent,
        previous_parent: None,
        parent: JournalTranscript {
            session_id: f.parent_session.id(),
            incarnation: f.parent_session.incarnation_id(),
        },
    };
    let consent =
        RelationshipConsent::new(f.factory.0.principals.requester(), Arc::new(NeverPrompt));
    // Inert construction, live matching structural IDs, and cancellation all
    // remain insufficient. Positive coverage uses real core admission below
    // managed::manager::tests::execution_consent, never a test-only mint.
    drop(consent.authorize(proposal(), CancellationToken::new()));
    assert!(block_on(consent.authorize(proposal(), CancellationToken::new())).is_err());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(block_on(consent.authorize(proposal(), cancelled)).is_err());
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}
