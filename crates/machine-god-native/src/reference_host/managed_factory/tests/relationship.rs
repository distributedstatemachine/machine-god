use super::FactoryFixture;
use crate::managed::{
    manager::factory::{
        ManagedRelationshipAuthorizer, ManagedRelationshipProposal, ManagedRuntimeError,
    },
    principal::{NativePrincipal, NativePrincipalTurn},
    store::JournalTranscript,
};
use crate::reference_host::managed_host::relationship::RelationshipConsent;
use crate::{PermissionPromptDecision, PermissionPromptError, PermissionPrompter};
use futures_executor::block_on;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, ManagedRelationshipAction, PermissionRequest,
    SessionId, SessionIncarnationId, ToolCallId, ToolContext, Turn,
};
use std::{
    sync::{Arc, Mutex},
    task::{Context, Waker},
};

struct Prompt {
    requests: Mutex<Vec<PermissionRequest>>,
    decision: PermissionPromptDecision,
    retire: Option<Arc<NativePrincipal>>,
    cancel: Option<CancellationToken>,
    pending: bool,
}
impl Prompt {
    fn new(decision: PermissionPromptDecision) -> Self {
        Self {
            requests: Mutex::default(),
            decision,
            retire: None,
            cancel: None,
            pending: false,
        }
    }
}
impl PermissionPrompter for Prompt {
    fn prompt(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionPromptDecision, PermissionPromptError>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            if let Some(principal) = &self.retire {
                principal.retire();
            }
            if let Some(token) = &self.cancel {
                token.cancel();
            }
            if self.pending {
                std::future::pending::<()>().await;
            }
            Ok(self.decision)
        })
    }
}
fn live(f: &FactoryFixture) -> (Turn, NativePrincipalTurn) {
    let turn = block_on(f.parent_session.prompt("relationship request")).unwrap();
    let origin = f.request("child").origin.unwrap();
    let guard = f
        .parent
        .begin_turn(&turn, origin.policy, origin.preferences, None)
        .unwrap();
    (turn, guard)
}
fn proposal(f: &FactoryFixture, turn: &Turn) -> ManagedRelationshipProposal {
    ManagedRelationshipProposal {
        origin: f.request("child").origin.unwrap(),
        context: ToolContext {
            session_id: f.parent_session.id(),
            session_incarnation_id: f.parent_session.incarnation_id(),
            turn_id: turn.id().clone(),
            call_id: ToolCallId::new("original-call").unwrap(),
        },
        child_id: "child".into(),
        generation: 7,
        revision: 11,
        action: ManagedRelationshipAction::Reparent,
        previous_parent: Some(JournalTranscript {
            session_id: SessionId::new("old-parent").unwrap(),
            incarnation: SessionIncarnationId::new("old-life").unwrap(),
        }),
        parent: JournalTranscript {
            session_id: f.parent_session.id(),
            incarnation: f.parent_session.incarnation_id(),
        },
    }
}

#[test]
fn every_positive_decision_is_consent_only_for_the_exact_proposal() {
    let f = FactoryFixture::new();
    let (turn, _guard) = live(&f);
    for decision in [
        PermissionPromptDecision::AllowOnce,
        PermissionPromptDecision::AllowTurn,
        PermissionPromptDecision::AllowSession,
        PermissionPromptDecision::Deny,
    ] {
        let prompt = Arc::new(Prompt::new(decision));
        let consent = RelationshipConsent::new(f.factory.0.principals.requester(), prompt.clone());
        let result =
            block_on(consent.authorize(proposal(&f, &turn), CancellationToken::new())).unwrap();
        assert_eq!(result, decision != PermissionPromptDecision::Deny);
        let requests = prompt.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].session_id, f.parent_session.id());
        assert_eq!(
            requests[0].session_incarnation_id,
            f.parent_session.incarnation_id()
        );
        let Capability::Custom { name, details } = &requests[0].capability else {
            panic!("relationship proposal");
        };
        assert_eq!(name, "managed_relationship");
        assert_eq!(details["child_generation"], 7);
        assert_eq!(details["child_revision"], 11);
        assert_eq!(details["previous_parent"]["incarnation"], "old-life");
        assert_eq!(
            details["parent"]["incarnation"],
            f.parent_session.incarnation_id().as_str()
        );
    }
    assert!(f.host.transport.requests.lock().unwrap().is_empty());
}

#[test]
fn consent_is_inert_and_rejects_cancelled_or_retired_calls_before_prompting() {
    let f = FactoryFixture::new();
    let (turn, guard) = live(&f);
    let prompt = Arc::new(Prompt::new(PermissionPromptDecision::AllowOnce));
    let consent = RelationshipConsent::new(f.factory.0.principals.requester(), prompt.clone());
    drop(consent.authorize(proposal(&f, &turn), CancellationToken::new()));
    let token = CancellationToken::new();
    token.cancel();
    assert!(block_on(consent.authorize(proposal(&f, &turn), token)).is_err());
    let pending = consent.authorize(proposal(&f, &turn), CancellationToken::new());
    drop(guard);
    assert!(block_on(pending).is_err());
    assert!(prompt.requests.lock().unwrap().is_empty());
}

#[test]
fn positive_answer_cannot_outlive_original_principal_or_cancellation() {
    for cancel in [false, true] {
        let f = FactoryFixture::new();
        let (turn, _guard) = live(&f);
        let token = CancellationToken::new();
        let mut prompt = Prompt::new(PermissionPromptDecision::AllowSession);
        if cancel {
            prompt.cancel = Some(token.clone());
        } else {
            prompt.retire = Some(f.parent.clone());
        }
        let consent =
            RelationshipConsent::new(f.factory.0.principals.requester(), Arc::new(prompt));
        assert!(matches!(
            block_on(consent.authorize(proposal(&f, &turn), token)),
            Err(ManagedRuntimeError::Unavailable)
        ));
    }
}

#[test]
fn cancelling_pending_consent_drops_the_original_prompt_wait() {
    let f = FactoryFixture::new();
    let (turn, _guard) = live(&f);
    let mut prompt = Prompt::new(PermissionPromptDecision::AllowOnce);
    prompt.pending = true;
    let prompt = Arc::new(prompt);
    let consent = RelationshipConsent::new(f.factory.0.principals.requester(), prompt.clone());
    let token = CancellationToken::new();
    let mut future = consent.authorize(proposal(&f, &turn), token.clone());
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(prompt.requests.lock().unwrap().len(), 1);
    token.cancel();
    assert!(block_on(future).is_err());
}
