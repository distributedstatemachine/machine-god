//! Explicit workspace selection shared by tools, permissions and conversations.

use crate::{NativeWorkspaceAuthority, NativeWorkspaceContexts};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct WorkspaceBinding {
    pub(crate) authority: NativeWorkspaceAuthority,
    pub(crate) contexts: Arc<NativeWorkspaceContexts>,
}

impl super::NativeReferenceHost {
    pub(crate) fn workspace_service_for_runtime(
        &self,
        runtime: &crate::NativeConversationRuntime,
        store: Option<Arc<crate::NativeUserConfigStore>>,
    ) -> Option<Arc<crate::NativeWorkspaceService>> {
        let authority = runtime.workspace_authority()?;
        let workers = self.services.control_workers.clone()?;
        Some(Arc::new(match store {
            Some(store) => crate::NativeWorkspaceService::new(authority, store, workers),
            None => crate::NativeWorkspaceService::without_settings(authority, workers),
        }))
    }
    /// Pure validation against the exact scope used during host composition.
    pub(crate) fn has_workspace_primary(
        &self,
        scope: &crate::NativeWorkspaceScopeSnapshot,
    ) -> bool {
        self.workspace_binding.as_ref().is_some_and(|binding| {
            binding
                .authority
                .snapshot()
                .is_ok_and(|current| current.same_primary(scope))
        })
    }

    /// Attaches this host's exact descriptor authority before conversation admission.
    /// Hosts without workspace authority still select their injected undo history.
    /// No root is opened or refreshed.
    ///
    /// # Errors
    /// Rejects duplicate or busy workspace registration.
    pub fn configure_conversation_workspace(
        &self,
        conversation: crate::NativeConversation,
    ) -> Result<crate::NativeConversation, crate::NativeConversationError> {
        let conversation = match &self.workspace_binding {
            Some(binding) => conversation
                .with_workspace_contexts(binding.authority.clone(), &binding.contexts)?,
            None => conversation,
        };
        match &self.undo_tracker {
            Some(undo) => conversation.with_undo_tracker(undo.clone()),
            None => Ok(conversation),
        }
    }

    /// Constructs a service over this host's exact workspace and worker ownership.
    /// The explicitly selected settings store is retained without loading it.
    /// Hosts without workspace or complete-terminal workers return `None`.
    #[must_use]
    pub fn workspace_service(
        &self,
        store: Arc<crate::NativeUserConfigStore>,
    ) -> Option<Arc<crate::NativeWorkspaceService>> {
        Some(Arc::new(crate::NativeWorkspaceService::new(
            self.workspace_binding.as_ref()?.authority.clone(),
            store,
            self.services.control_workers.clone()?,
        )))
    }

    /// Constructs a listing-only service without selecting or discovering settings.
    /// Hosts without workspace or complete-terminal workers return `None`.
    #[must_use]
    pub fn workspace_service_without_settings(&self) -> Option<Arc<crate::NativeWorkspaceService>> {
        Some(Arc::new(crate::NativeWorkspaceService::without_settings(
            self.workspace_binding.as_ref()?.authority.clone(),
            self.services.control_workers.clone()?,
        )))
    }
}
