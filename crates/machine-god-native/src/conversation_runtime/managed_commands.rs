//! Actual-runtime selection for explicit human commands, never a model call.
use super::{
    Arc, LifecyclePermit, NativeConversationError, NativeConversationRuntime,
    NativeConversationRuntimeError, NativeModelPreferences, NativePermissionPolicySnapshot,
};
use crate::{NativeWorkspaceScopeSnapshot, managed::principal::NativePrincipal};

pub(crate) struct NativeManagedCommandSnapshot {
    pub permit: LifecyclePermit,
    pub workspace: NativeWorkspaceScopeSnapshot,
    pub policy: NativePermissionPolicySnapshot,
    pub preferences: NativeModelPreferences,
}

impl NativeConversationRuntime {
    /// Captures the actual bound principal and retains its ordinary lifecycle
    /// admission through command settlement. Does not cancel an active turn.
    pub(crate) fn capture_managed_command(
        &self,
        principal: &Arc<NativePrincipal>,
    ) -> Result<NativeManagedCommandSnapshot, NativeConversationRuntimeError> {
        let permit = self.lifecycle.acquire()?;
        if !self.conversation.managed_principal_matches(principal) {
            return Err(NativeConversationError::ManagedAdmission.into());
        }
        let policy = self
            .permissions()
            .ok_or(NativeConversationError::ManagedAdmission)?
            .snapshot_admitted(&permit)
            .map_err(|_| NativeConversationError::ManagedAdmission)?;
        let workspace = self
            .conversation
            .capture_workspace_scope()?
            .ok_or(NativeConversationError::ManagedAdmission)?;
        let preferences = self.model_preferences();
        Ok(NativeManagedCommandSnapshot {
            permit,
            workspace,
            policy,
            preferences,
        })
    }
}
