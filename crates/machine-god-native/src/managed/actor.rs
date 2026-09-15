//! Distinct native-human and actual model-call admission for the shared journal.
use super::{
    principal::{NativeManagedCallLease, NativePrincipal},
    scheduler::RunRef,
};
use crate::{
    NativeConversationRuntime, NativeModelPreferences, NativePermissionPolicySnapshot,
    NativeWorkspaceScopeSnapshot, conversation_runtime::NativeManagedCommandSnapshot,
};
use machine_god_core::{CancellationToken, ManagedSubagentError};
use std::sync::Arc;

/// Non-clone custody. Human admission cannot be projected as a core call witness.
pub(crate) enum ManagedCommandActor {
    Model(NativeManagedCallLease),
    Human(Box<HumanCommandLease>),
}

pub(crate) struct HumanCommandLease {
    principal: Arc<NativePrincipal>,
    selected: NativeManagedCommandSnapshot,
    cancellation: CancellationToken,
}

impl ManagedCommandActor {
    pub(crate) fn human(
        runtime: &NativeConversationRuntime,
        principal: Arc<NativePrincipal>,
        cancellation: CancellationToken,
    ) -> Result<Self, ManagedSubagentError> {
        if cancellation.is_cancelled() {
            return Err(ManagedSubagentError::Cancelled);
        }
        let selected = runtime
            .capture_managed_command(&principal)
            .map_err(|_| ManagedSubagentError::Unavailable)?;
        Ok(Self::Human(Box::new(HumanCommandLease {
            principal,
            selected,
            cancellation,
        })))
    }

    pub(crate) fn is_human(&self) -> bool {
        matches!(self, Self::Human(_))
    }

    pub(crate) fn is_live(&self) -> bool {
        match self {
            Self::Model(lease) => lease.is_live(),
            Self::Human(lease) => {
                lease.principal.is_live()
                    && !lease.selected.permit.was_quiesced()
                    && !lease.cancellation.is_cancelled()
            }
        }
    }

    pub(crate) fn principal(&self) -> &Arc<NativePrincipal> {
        match self {
            Self::Model(lease) => lease.principal(),
            Self::Human(lease) => &lease.principal,
        }
    }

    pub(crate) fn policy(&self) -> &NativePermissionPolicySnapshot {
        match self {
            Self::Model(lease) => lease.policy(),
            Self::Human(lease) => &lease.selected.policy,
        }
    }

    pub(crate) fn workspace(&self) -> &NativeWorkspaceScopeSnapshot {
        match self {
            Self::Model(lease) => lease.workspace(),
            Self::Human(lease) => &lease.selected.workspace,
        }
    }

    pub(crate) fn preferences(&self) -> &NativeModelPreferences {
        match self {
            Self::Model(lease) => lease.preferences(),
            Self::Human(lease) => &lease.selected.preferences,
        }
    }

    pub(crate) fn run(&self) -> Option<&RunRef> {
        match self {
            Self::Model(lease) => lease.run(),
            Self::Human(_) => None,
        }
    }
}

impl std::fmt::Debug for ManagedCommandActor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagedCommandActor { .. }")
    }
}
