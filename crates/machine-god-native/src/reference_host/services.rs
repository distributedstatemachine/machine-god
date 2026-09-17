//! One shared execution domain, independent of a principal's mutable selection.

use std::sync::Arc;

use machine_god_core::Engine;

use crate::{
    FileSessionStore, NativeConversationModelRoutes, NativeConversationObservations,
    NativeOwnedWorkerCompletion, NativeOwnedWorkerScope, NativePermissionContexts,
    NativePermissionController, NativeSessionLifecycle, NativeTerminalBackgroundRequester,
    NativeTerminalLifecycleRequester,
};

/// Actual service ownership. A principal may share this allocation without
/// rebuilding the engine, creating another worker pool, or copying live grants.
/// Reverse tool/context routes must remain weak: services never own their
/// manager or the conversation runtimes which use them.
pub(super) struct NativeHostServices {
    pub engine: Engine,
    pub session_store: Arc<FileSessionStore>,
    pub session_lifecycle: NativeSessionLifecycle,
    pub terminal_shutdown: Option<NativeOwnedWorkerCompletion>,
    pub control_workers: Option<NativeOwnedWorkerScope>,
    pub terminal_lifecycle: Option<NativeTerminalLifecycleRequester>,
    pub terminal_background: Option<NativeTerminalBackgroundRequester>,
    pub model_routes: Option<Arc<NativeConversationModelRoutes>>,
    pub managed_model_catalog: crate::conversation_runtime::SharedModelCatalog,
    pub observations: Option<Arc<NativeConversationObservations>>,
    pub permissions: Option<Arc<NativePermissionController>>,
    pub permission_preparation: Option<Arc<super::permissions::SharedPermissionPreparation>>,
    pub managed_mcp_seed: Option<Arc<super::mcp::ManagedMcpSeed>>,
    pub permission_contexts: Option<Arc<NativePermissionContexts>>,
}
