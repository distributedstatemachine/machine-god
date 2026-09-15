use super::{TurnRoute, check_cancel, check_route};
use crate::{
    NativeConfiguredPermissionRules, NativePermissionAutomaticOutcome,
    NativePermissionConfiguredOutcome, NativePermissionExecutionProof, NativePermissionRuleKey,
    NativePreparedPermissionAction, PermissionMode,
};
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionError, PermissionExecutionAdmission,
};
use std::sync::Weak;

pub(super) struct Action {
    pub(super) route: Weak<TurnRoute>,
    pub(super) inner: Box<dyn NativePreparedPermissionAction>,
}
impl NativePreparedPermissionAction for Action {
    fn saved_rule_key(&self) -> Option<&NativePermissionRuleKey> {
        check_route(&self.route).ok()?;
        self.inner.saved_rule_key()
    }
    fn grant_key(&self) -> Option<&NativePermissionRuleKey> {
        check_route(&self.route).ok()?;
        self.inner.grant_key()
    }
    fn is_file_mutation(&self) -> bool {
        self.inner.is_file_mutation()
    }
    fn configured_outcome(
        &self,
        rules: &NativeConfiguredPermissionRules,
    ) -> Result<NativePermissionConfiguredOutcome, PermissionError> {
        check_route(&self.route)?;
        self.inner.configured_outcome(rules)
    }
    fn allows_without_review(&self, mode: PermissionMode) -> bool {
        check_route(&self.route).is_ok() && self.inner.allows_without_review(mode)
    }
    fn automatic_review(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionAutomaticOutcome, PermissionError>> {
        Box::pin(async move {
            check_cancel(&cancellation)?;
            check_route(&self.route)?;
            let result = self.inner.automatic_review(cancellation.clone()).await?;
            check_cancel(&cancellation)?;
            check_route(&self.route)?;
            Ok(result)
        })
    }
    fn bind_execution(
        self: Box<Self>,
        proof: NativePermissionExecutionProof,
    ) -> Result<Box<dyn PermissionExecutionAdmission>, PermissionError> {
        check_route(&self.route)?;
        Ok(Box::new(Admission {
            route: self.route,
            inner: self.inner.bind_execution(proof)?,
        }))
    }
}
struct Admission {
    route: Weak<TurnRoute>,
    inner: Box<dyn PermissionExecutionAdmission>,
}
impl PermissionExecutionAdmission for Admission {
    fn admit(self: Box<Self>) -> Result<(), PermissionError> {
        check_route(&self.route)?;
        self.inner.admit()
    }
}
