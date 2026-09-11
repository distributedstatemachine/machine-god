use super::{Permit, check, identity, invalid};
use crate::mcp::{
    context::NativeMcpTurnContext,
    submission::{McpSubmissionAdmission, McpToolRequest, PreparedMcpSubmission},
};
use crate::{
    NativeAutoPermissionAction, NativeAutoPermissionDecision, NativeAutoPermissionOrigin,
    NativeAutoPermissionPhase, NativeAutoPermissionReview, NativeConfiguredPermissionDecision,
    NativeConfiguredPermissionRules, NativePermissionAutomaticOutcome,
    NativePermissionConfiguredOutcome, NativePermissionExecutionProof,
    NativePermissionReviewContext, NativePermissionReviewer, NativePermissionRuleKey,
    NativePermissionTargetKind, NativePreparedPermissionAction, NativePreparedPermissionTarget,
    PermissionMode,
};
use futures_util::future::{Either, select};
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionError, PermissionExecutionAdmission, SessionId,
};
use std::sync::Arc;

pub(super) struct Evidence {
    workspace: Arc<str>,
    tool: String,
    arguments: String,
    schema: crate::mcp::schema::McpSchema,
    key: Option<NativePermissionRuleKey>,
}
impl Evidence {
    pub(super) fn new(workspace: &Arc<str>, projection: &McpToolRequest) -> Self {
        let binding = projection.binding();
        let fingerprint = identity::runtime_fingerprint(&[
            binding.configuration_bytes(),
            binding.schema_bytes(),
            binding.authentication_bytes(),
        ]);
        let key = identity::key(
            workspace,
            binding.server(),
            binding.tool_name().as_str(),
            binding.remote_tool(),
            &fingerprint,
            projection.arguments_json(),
        );
        Self {
            workspace: workspace.clone(),
            tool: binding.tool_name().as_str().into(),
            arguments: projection.arguments_json().into(),
            schema: projection.schema().clone(),
            key,
        }
    }
}

pub(super) struct Action {
    prepared: PreparedMcpSubmission,
    evidence: Evidence,
    turn: NativeMcpTurnContext,
    review_context: NativePermissionReviewContext,
    reviewer: Arc<dyn NativePermissionReviewer>,
    permit: Permit,
    cancellation: CancellationToken,
    session: SessionId,
    workspace_scope: Option<crate::NativeWorkspaceTurnScope>,
}
impl Action {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        prepared: PreparedMcpSubmission,
        evidence: Evidence,
        turn: NativeMcpTurnContext,
        review_context: NativePermissionReviewContext,
        reviewer: Arc<dyn NativePermissionReviewer>,
        permit: Permit,
        cancellation: CancellationToken,
        session: SessionId,
        workspace_scope: Option<crate::NativeWorkspaceTurnScope>,
    ) -> Self {
        Self {
            prepared,
            evidence,
            turn,
            review_context,
            reviewer,
            permit,
            cancellation,
            session,
            workspace_scope,
        }
    }
    pub(super) fn revalidate(&self) -> Result<(), PermissionError> {
        check(&self.cancellation)?;
        if self
            .workspace_scope
            .as_ref()
            .is_some_and(|scope| !scope.is_live())
        {
            return Err(invalid());
        }
        self.turn.revalidate().map_err(|_| invalid())?;
        self.prepared.revalidate().map_err(|_| invalid())?;
        self.review_context
            .is_live()
            .then_some(())
            .ok_or_else(invalid)
    }
}
impl NativePreparedPermissionAction for Action {
    fn saved_rule_key(&self) -> Option<&NativePermissionRuleKey> {
        self.evidence.key.as_ref()
    }
    fn grant_key(&self) -> Option<&NativePermissionRuleKey> {
        self.evidence.key.as_ref()
    }
    fn is_file_mutation(&self) -> bool {
        false
    }
    fn configured_outcome(
        &self,
        rules: &NativeConfiguredPermissionRules,
    ) -> Result<NativePermissionConfiguredOutcome, PermissionError> {
        self.revalidate()?;
        let target = NativePreparedPermissionTarget::new(
            &self.evidence.workspace,
            &self.evidence.tool,
            &self.evidence.arguments,
            NativePermissionTargetKind::None,
        )
        .map_err(|_| invalid())?;
        Ok(match rules.decide(&target).map_err(|_| invalid())? {
            Some(NativeConfiguredPermissionDecision::Deny) => {
                NativePermissionConfiguredOutcome::Deny
            }
            Some(NativeConfiguredPermissionDecision::Ask) => NativePermissionConfiguredOutcome::Ask,
            Some(NativeConfiguredPermissionDecision::Allow) => {
                NativePermissionConfiguredOutcome::Allow
            }
            None => NativePermissionConfiguredOutcome::Unresolved,
        })
    }
    fn allows_without_review(&self, _: PermissionMode) -> bool {
        false
    }
    fn automatic_review(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionAutomaticOutcome, PermissionError>> {
        Box::pin(async move {
            self.revalidate()?;
            check(&cancellation)?;
            let review = self.reviewer.review(
                NativeAutoPermissionReview {
                    session_id: &self.session,
                    workspace_root: &self.evidence.workspace,
                    source_model: self.review_context.source_model().ok_or_else(invalid)?,
                    pending_assistant: self.review_context.pending_assistant(),
                    target_call_id: self.review_context.target_call_id(),
                    trusted_root_context: self
                        .review_context
                        .trusted_root_context()
                        .map_err(|_| invalid())?,
                    origin: NativeAutoPermissionOrigin::Root,
                    phase: NativeAutoPermissionPhase::Initial,
                    targets: &[],
                    action: NativeAutoPermissionAction::Tool {
                        tool_name: &self.evidence.tool,
                        arguments_json: &self.evidence.arguments,
                        schema_json: Some(self.evidence.schema.raw_json()),
                        schema_required: true,
                    },
                    escalation_reason: "",
                },
                cancellation.clone(),
            );
            let cancelled = async {
                select(cancellation.cancelled(), self.turn.cancelled()).await;
            };
            let assessment = match select(review, Box::pin(cancelled)).await {
                Either::Left((result, _)) => result.map_err(|_| invalid())?,
                Either::Right(_) => return Err(invalid()),
            };
            self.revalidate()?;
            check(&cancellation)?;
            Ok(match assessment.decision() {
                NativeAutoPermissionDecision::Allow => NativePermissionAutomaticOutcome::Allow,
                NativeAutoPermissionDecision::Ask => NativePermissionAutomaticOutcome::Ask,
            })
        })
    }
    fn bind_execution(
        self: Box<Self>,
        proof: NativePermissionExecutionProof,
    ) -> Result<Box<dyn PermissionExecutionAdmission>, PermissionError> {
        self.revalidate()?;
        proof.revalidate()?;
        Ok(Box::new(Admission {
            turn: self.turn,
            submission: self.prepared.bind_execution(proof),
            _permit: self.permit,
            workspace_scope: self.workspace_scope,
        }))
    }
}
struct Admission {
    turn: NativeMcpTurnContext,
    submission: McpSubmissionAdmission,
    _permit: Permit,
    workspace_scope: Option<crate::NativeWorkspaceTurnScope>,
}
impl PermissionExecutionAdmission for Admission {
    fn admit(self: Box<Self>) -> Result<(), PermissionError> {
        if self
            .workspace_scope
            .as_ref()
            .is_some_and(|scope| !scope.is_live())
        {
            return Err(invalid());
        }
        self.turn.revalidate().map_err(|_| invalid())?;
        Box::new(self.submission).admit()
    }
}
