//! Explicit workspace selection shared by tools, permissions and conversations.

use crate::{NativeWorkspaceAuthority, NativeWorkspaceContexts};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct WorkspaceBinding {
    pub(crate) authority: NativeWorkspaceAuthority,
    pub(crate) contexts: Arc<NativeWorkspaceContexts>,
}

impl super::NativeReferenceHost {
    /// Attaches this host's exact descriptor authority before conversation admission.
    /// Legacy hosts leave the conversation unchanged. No root is opened or refreshed.
    ///
    /// # Errors
    /// Rejects duplicate or busy workspace registration.
    pub fn configure_conversation_workspace(
        &self,
        conversation: crate::NativeConversation,
    ) -> Result<crate::NativeConversation, crate::NativeConversationError> {
        match &self.workspace_binding {
            Some(binding) => {
                conversation.with_workspace_contexts(binding.authority.clone(), &binding.contexts)
            }
            None => Ok(conversation),
        }
    }

    /// Constructs a service over this host's exact workspace and worker ownership.
    /// The explicitly selected settings store is retained without loading it.
    /// Legacy hosts without workspace or complete-terminal workers return `None`.
    #[must_use]
    pub fn workspace_service(
        &self,
        store: Arc<crate::NativeUserConfigStore>,
    ) -> Option<Arc<crate::NativeWorkspaceService>> {
        Some(Arc::new(crate::NativeWorkspaceService::new(
            self.workspace_binding.as_ref()?.authority.clone(),
            store,
            self.control_workers.clone()?,
        )))
    }
}
