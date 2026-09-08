use super::{Permit, check_context, identity, invalid};
use crate::{
    NativeAutoPermissionAction, NativeAutoPermissionDecision, NativeAutoPermissionFilePreimage,
    NativeAutoPermissionOrigin, NativeAutoPermissionPhase, NativeAutoPermissionReview,
    NativeAutoPermissionSandboxScope, NativeAutoPermissionTarget,
    NativeConfiguredPermissionDecision, NativeConfiguredPermissionRules, NativeFileApprovalKind,
    NativeFileApprovalPreimage, NativePermissionAutomaticOutcome,
    NativePermissionConfiguredOutcome, NativePermissionController, NativePermissionExecutionProof,
    NativePermissionReviewContext, NativePermissionReviewer, NativePermissionRuleKey,
    NativePermissionTargetKind, NativePreparedPermissionAction, NativePreparedPermissionTarget,
    NativePreparedPermissionTargets, PermissionMode, PreparedFileApproval,
};
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionError, PermissionExecutionAdmission, SessionId,
    SessionIncarnationId, TurnId,
};
use std::sync::{Arc, Weak};

pub(super) struct Route {
    pub session: SessionId,
    pub incarnation: SessionIncarnationId,
    pub turn: TurnId,
}
pub(super) struct Action {
    route: Route,
    targets: NativePreparedPermissionTargets,
    file: Option<PreparedFileApproval>,
    context: NativePermissionReviewContext,
    reviewer: Arc<dyn NativePermissionReviewer>,
    saved: Option<NativePermissionRuleKey>,
    grant: Option<NativePermissionRuleKey>,
    semantic: String,
    permit: Permit,
    controller: Weak<NativePermissionController>,
}
impl Action {
    pub(super) fn new(
        route: Route,
        targets: NativePreparedPermissionTargets,
        file: Option<PreparedFileApproval>,
        context: NativePermissionReviewContext,
        reviewer: Arc<dyn NativePermissionReviewer>,
        permit: Permit,
        controller: Weak<NativePermissionController>,
    ) -> Result<Self, PermissionError> {
        let semantic = targets.identity_arguments_json()?;
        let saved = identity::saved(
            &targets,
            file.as_ref(),
            context.permission_policy().ok_or_else(invalid)?,
            &semantic,
        );
        let grant = if targets.tool_name() == "read_file" {
            targets
                .targets()
                .first()
                .and_then(|target| identity::read_grant(targets.workspace(), target.path()))
        } else {
            saved.clone()
        };
        Ok(Self {
            route,
            targets,
            file,
            context,
            reviewer,
            saved,
            grant,
            semantic,
            permit,
            controller,
        })
    }

    fn disclosure_requires_prompt(
        &self,
        rules: &NativeConfiguredPermissionRules,
    ) -> Result<bool, PermissionError> {
        let Some(file) = &self.file else {
            return Ok(false);
        };
        if !matches!(
            file.kind(),
            NativeFileApprovalKind::Write | NativeFileApprovalKind::Edit
        ) || matches!(file.preimage(), NativeFileApprovalPreimage::Missing)
        {
            return Ok(false);
        }
        let path = self.targets.targets().first().ok_or_else(invalid)?.path();
        let target = NativePreparedPermissionTarget::new(
            self.targets.workspace(),
            "read_file",
            path,
            NativePermissionTargetKind::PathExisting,
        )
        .map_err(|_| invalid())?;
        match rules.decide(&target).map_err(|_| invalid())? {
            Some(NativeConfiguredPermissionDecision::Deny) => Ok(true),
            Some(NativeConfiguredPermissionDecision::Ask) => {
                let Some(key) = identity::read_grant(self.targets.workspace(), path) else {
                    return Ok(true);
                };
                let controller = self.controller.upgrade().ok_or_else(invalid)?;
                controller
                    .has_live_grant(
                        &self.route.session,
                        &self.route.incarnation,
                        &self.route.turn,
                        &key,
                    )
                    .map(|granted| !granted)
            }
            Some(NativeConfiguredPermissionDecision::Allow) | None => Ok(false),
        }
    }

    fn review_action(&self) -> NativeAutoPermissionAction<'_> {
        if let Some(file) = &self.file {
            return NativeAutoPermissionAction::FileMutation {
                tool_name: file.tool_name(),
                display_path: self
                    .targets
                    .targets()
                    .iter()
                    .find(|target| matches!(target.role(), "target" | "destination"))
                    .expect("file projection has destination")
                    .path(),
                preimage: match file.preimage() {
                    NativeFileApprovalPreimage::Missing => NativeAutoPermissionFilePreimage::Absent,
                    NativeFileApprovalPreimage::File(bytes) => {
                        NativeAutoPermissionFilePreimage::File(bytes)
                    }
                    NativeFileApprovalPreimage::EmptyDirectory => {
                        NativeAutoPermissionFilePreimage::EmptyDirectory
                    }
                },
                postimage: file.postimage(),
            };
        }
        if let Some(command) = identity::command(&self.targets) {
            return NativeAutoPermissionAction::Command {
                command: command.command,
                resolved_cwd: command.cwd,
                background: command.background,
                backend: identity::backend(
                    self.context.permission_policy().expect("validated policy"),
                ),
                target_os: std::env::consts::OS,
                scope: NativeAutoPermissionSandboxScope::Restricted,
            };
        }
        NativeAutoPermissionAction::Tool {
            tool_name: self.targets.tool_name(),
            arguments_json: &self.semantic,
            schema_json: None,
            schema_required: false, // only explicit builtin registrations reach this adapter
        }
    }
}
impl NativePreparedPermissionAction for Action {
    fn saved_rule_key(&self) -> Option<&NativePermissionRuleKey> {
        self.saved.as_ref()
    }
    fn grant_key(&self) -> Option<&NativePermissionRuleKey> {
        self.grant.as_ref()
    }
    fn is_file_mutation(&self) -> bool {
        self.file.is_some()
    }
    fn configured_outcome(
        &self,
        rules: &NativeConfiguredPermissionRules,
    ) -> Result<NativePermissionConfiguredOutcome, PermissionError> {
        if !self.context.is_live() {
            return Err(invalid());
        }
        let outcome = self.targets.configured_outcome(rules)?;
        if outcome != NativePermissionConfiguredOutcome::Deny
            && self.disclosure_requires_prompt(rules)?
        {
            Ok(NativePermissionConfiguredOutcome::Ask)
        } else {
            Ok(outcome)
        }
    }
    fn allows_without_review(&self, mode: PermissionMode) -> bool {
        if !self.context.is_live() {
            return false;
        }
        if let Some(file) = &self.file {
            return mode == PermissionMode::Auto
                && matches!(
                    file.kind(),
                    NativeFileApprovalKind::Write | NativeFileApprovalKind::Edit
                )
                && !crate::permission_targets::sensitive_path(self.targets.targets()[0].path());
        }
        self.targets.allows_without_review(mode)
    }
    fn automatic_review(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionAutomaticOutcome, PermissionError>> {
        Box::pin(async move {
            check_context(&self.context, &cancellation)?;
            let targets: Vec<_> = self
                .targets
                .targets()
                .iter()
                .map(|target| NativeAutoPermissionTarget {
                    role: target.role(),
                    path: target.path(),
                })
                .collect();
            let assessment = self
                .reviewer
                .review(
                    NativeAutoPermissionReview {
                        session_id: &self.route.session,
                        workspace_root: self.targets.workspace(),
                        source_model: self.context.source_model().ok_or_else(invalid)?,
                        pending_assistant: self.context.pending_assistant(),
                        target_call_id: self.context.target_call_id(),
                        trusted_root_context: self
                            .context
                            .trusted_root_context()
                            .map_err(|_| invalid())?,
                        origin: NativeAutoPermissionOrigin::Root,
                        phase: NativeAutoPermissionPhase::Initial,
                        targets: &targets,
                        action: self.review_action(),
                        escalation_reason: "",
                    },
                    cancellation.clone(),
                )
                .await
                .map_err(|_| invalid())?;
            check_context(&self.context, &cancellation)?;
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
        if !self.context.is_live() {
            return Err(invalid());
        }
        proof.revalidate()?;
        if let Some(file) = self.file {
            Ok(Box::new(file.admit(Arc::new(FilePolicy {
                proof,
                _permit: self.permit,
            }))))
        } else {
            Ok(Box::new(Admission {
                proof,
                _targets: self.targets,
                _permit: self.permit,
            }))
        }
    }
}
struct FilePolicy {
    proof: NativePermissionExecutionProof,
    _permit: Permit,
}
impl crate::NativeFileApprovalPolicy for FilePolicy {
    fn revalidate(&self) -> Result<(), PermissionError> {
        self.proof.revalidate()
    }
}
struct Admission {
    proof: NativePermissionExecutionProof,
    _targets: NativePreparedPermissionTargets,
    _permit: Permit,
}
impl PermissionExecutionAdmission for Admission {
    fn admit(self: Box<Self>) -> Result<(), PermissionError> {
        self.proof.revalidate()
    }
}
