//! Exact factory-paired permission preparation without consuming execution claims.
use super::{
    CapturedRoute, NativePrincipalMcpRequester, PrincipalMcpError, Result, TurnRoute,
    select_captured,
};
use crate::mcp::{
    context::NativeMcpContexts, permission::NativeMcpPermissionPreparer, runtime::NativeMcpRuntime,
};
use crate::{
    NativePermissionActionPreparer, NativePermissionContexts, NativePermissionReviewer,
    NativePermissionTargetAuthority, NativePreparedPermissionAction, NativeWorkspaceContexts,
};
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionError, PermissionInvocation, PermissionRequest,
    SessionId, SessionIncarnationId, TurnId,
};
use std::sync::{Arc, Weak};

#[path = "permission/action.rs"]
mod action;

/// All inputs are the existing concrete native factory selections, including
/// the original helper-bearing builtin preparer; none are reconstructed here.
pub(crate) struct NativePrincipalMcpPermissionInputs {
    pub(crate) builtins: Arc<NativePermissionTargetAuthority>,
    pub(crate) builtin_preparer: Arc<dyn NativePermissionActionPreparer>,
    pub(crate) contexts: Arc<NativeMcpContexts>,
    pub(crate) review_contexts: Arc<NativePermissionContexts>,
    pub(crate) reviewer: Arc<dyn NativePermissionReviewer>,
    pub(crate) workspace: Arc<str>,
    pub(crate) workspace_contexts: Option<Arc<NativeWorkspaceContexts>>,
}

/// Outer manager owns this bundle/controller through actual cleanup. A route
/// receives only a weak preparer, never this strong preparer→runtime edge.
pub(crate) struct NativePrincipalMcpPermissions {
    runtime: Weak<NativeMcpRuntime>,
    preparer: Arc<NativeMcpPermissionPreparer>,
}
impl NativePrincipalMcpPermissions {
    pub(crate) fn new(
        runtime: &Arc<NativeMcpRuntime>,
        inputs: NativePrincipalMcpPermissionInputs,
    ) -> Result<Self> {
        if !runtime.uses_contexts(&inputs.contexts) || runtime.publication_checkpoint().is_err() {
            return Err(PrincipalMcpError::Unavailable);
        }
        let preparer = NativeMcpPermissionPreparer::new(
            inputs.builtins,
            inputs.builtin_preparer,
            runtime.clone(),
            inputs.contexts,
            inputs.review_contexts,
            inputs.reviewer,
            &inputs.workspace,
        )
        .map_err(|_| PrincipalMcpError::Unavailable)?;
        let preparer = match inputs.workspace_contexts {
            Some(contexts) => preparer.with_workspace_contexts(contexts),
            None => preparer,
        };
        Ok(Self {
            runtime: Arc::downgrade(runtime),
            preparer: Arc::new(preparer),
        })
    }
    pub(super) fn for_runtime(
        &self,
        runtime: &Arc<NativeMcpRuntime>,
    ) -> Result<Weak<NativeMcpPermissionPreparer>> {
        if !self.runtime.ptr_eq(&Arc::downgrade(runtime)) {
            return Err(PrincipalMcpError::Unavailable);
        }
        Ok(Arc::downgrade(&self.preparer))
    }
}

pub(crate) struct NativePrincipalMcpPermissionRouter(pub(super) NativePrincipalMcpRequester);
impl NativePermissionActionPreparer for NativePrincipalMcpPermissionRouter {
    fn close_turn(&self, _: &SessionId, _: &SessionIncarnationId, _: &TurnId) {
        // Public IDs cannot distinguish a delayed old callback from a current
        // replacement allocation, even when lookup finds exactly one route.
        // The manager drops the exact NativePrincipalMcpTurn on finish/cancel;
        // that guard and owner retirement close only their original preparer.
    }
    fn prepare<'a>(
        &'a self,
        request: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, std::result::Result<Box<dyn NativePreparedPermissionAction>, PermissionError>>
    {
        let captured = self.capture(request);
        Box::pin(async move {
            check_cancel(&cancellation)?;
            let selected = select_captured(captured).map_err(|_| unavailable())?;
            let owner = selected.route.owner.upgrade().ok_or_else(unavailable)?;
            let preparer = owner
                .permissions
                .as_ref()
                .and_then(Weak::upgrade)
                .ok_or_else(unavailable)?;
            drop(owner);
            let action = preparer
                .prepare(request, invocation, cancellation.clone())
                .await?;
            check_cancel(&cancellation)?;
            if !selected.route.live() {
                return Err(unavailable());
            }
            Ok(Box::new(action::Action {
                route: Arc::downgrade(&selected.route),
                inner: action,
            }) as Box<dyn NativePreparedPermissionAction>)
        })
    }
}
impl NativePrincipalMcpPermissionRouter {
    fn capture(&self, request: &PermissionRequest) -> Result<CapturedRoute> {
        let registry = self.0.0.upgrade().ok_or(PrincipalMcpError::Unavailable)?;
        let stamp = registry
            .principals
            .stamp_for_turn(
                &request.session_id,
                &request.session_incarnation_id,
                &request.turn_id,
            )
            .map_err(|_| PrincipalMcpError::Unavailable)?;
        super::capture_route(registry.route(&stamp)?)
    }
}
fn check_route(route: &Weak<TurnRoute>) -> std::result::Result<(), PermissionError> {
    if route.upgrade().is_some_and(|route| route.live()) {
        Ok(())
    } else {
        Err(unavailable())
    }
}
fn check_cancel(cancellation: &CancellationToken) -> std::result::Result<(), PermissionError> {
    if cancellation.is_cancelled() {
        Err(unavailable())
    } else {
        Ok(())
    }
}
fn unavailable() -> PermissionError {
    PermissionError::new(
        "mcp_principal_permission_unavailable",
        "The original principal permission route is unavailable",
    )
}

macro_rules! redacted {($($ty:ty),+)=>{$(impl std::fmt::Debug for $ty {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.debug_struct(stringify!($ty)).finish_non_exhaustive()}})+};}
redacted!(
    NativePrincipalMcpPermissionInputs,
    NativePrincipalMcpPermissions,
    NativePrincipalMcpPermissionRouter
);
