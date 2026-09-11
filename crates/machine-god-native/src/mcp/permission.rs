//! Explicit builtin/MCP permission composition with retained one-shot submission.

mod action;
mod identity;

use super::{
    context::{NativeMcpContexts, NativeMcpTurnContext},
    submission::McpToolRequest,
};
use crate::{
    NativePermissionActionPreparer, NativePermissionContexts, NativePermissionReviewer,
    NativePermissionTargetAuthority, NativePreparedPermissionAction,
};
use futures_util::future::{Either, select};
use machine_god_core::{
    BoxFuture, CancellationToken, PermissionError, PermissionInvocation, PermissionRequest,
    SessionId, SessionIncarnationId, TurnId,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Trusted runtime-owned lookup over actual published executable registrations.
/// `None` means no exact MCP route; malformed/stale/ambiguous routes must return
/// an error. Implementations must retain no engine/session ownership cycle and
/// perform no work until polled. A returned request grants no permission.
pub trait NativeMcpPermissionAuthority: Send + Sync + 'static {
    fn resolve<'a>(
        &'a self,
        request: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        context: &'a NativeMcpTurnContext,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<McpToolRequest>, PermissionError>>;
}

/// Finite simultaneous preparations/reviews awaiting core admission. Published
/// submissions have the registry's independent exact-turn slot bound.
pub const MAX_NATIVE_MCP_PERMISSION_PREPARATIONS: usize = 4;

/// Native composition, not an unknown-tool bypass. Builtin routing uses the
/// same actual registration authority as the builtin preparer; every other
/// name must resolve to an exact runtime-owned typed MCP request.
pub struct NativeMcpPermissionPreparer {
    builtins: Arc<NativePermissionTargetAuthority>,
    builtin_preparer: Arc<dyn NativePermissionActionPreparer>,
    authority: Arc<dyn NativeMcpPermissionAuthority>,
    contexts: Arc<NativeMcpContexts>,
    review_contexts: Arc<NativePermissionContexts>,
    reviewer: Arc<dyn NativePermissionReviewer>,
    workspace: Arc<str>,
    workspace_contexts: Option<Arc<crate::NativeWorkspaceContexts>>,
    active: Arc<AtomicUsize>,
}
impl NativeMcpPermissionPreparer {
    /// Retains explicitly admitted authorities. No observation, reservation,
    /// review, worker or external effect occurs during construction.
    /// # Errors
    /// Rejects invalid/oversized workspace presentation, without opening it.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        builtins: Arc<NativePermissionTargetAuthority>,
        builtin_preparer: Arc<dyn NativePermissionActionPreparer>,
        authority: Arc<dyn NativeMcpPermissionAuthority>,
        contexts: Arc<NativeMcpContexts>,
        review_contexts: Arc<NativePermissionContexts>,
        reviewer: Arc<dyn NativePermissionReviewer>,
        workspace: &str,
    ) -> Result<Self, PermissionError> {
        if !workspace.starts_with('/') || workspace.len() > 4096 || workspace.contains('\0') {
            return Err(invalid());
        }
        Ok(Self {
            builtins,
            builtin_preparer,
            authority,
            contexts,
            review_contexts,
            reviewer,
            workspace: workspace.into(),
            workspace_contexts: None,
            active: Arc::default(),
        })
    }

    /// Pins workspace presentation to the already admitted exact turn scope.
    /// Missing/retired scopes reject preparation rather than using startup data.
    #[must_use]
    pub fn with_workspace_contexts(
        mut self,
        contexts: Arc<crate::NativeWorkspaceContexts>,
    ) -> Self {
        self.workspace_contexts = Some(contexts);
        self
    }
}
impl std::fmt::Debug for NativeMcpPermissionPreparer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeMcpPermissionPreparer { <redacted> }")
    }
}
impl NativePermissionActionPreparer for NativeMcpPermissionPreparer {
    fn close_turn(&self, session: &SessionId, incarnation: &SessionIncarnationId, turn: &TurnId) {
        self.contexts.close_turn(session, incarnation, turn);
        self.builtin_preparer.close_turn(session, incarnation, turn);
    }

    fn prepare<'a>(
        &'a self,
        request: &'a PermissionRequest,
        invocation: PermissionInvocation<'a>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn NativePreparedPermissionAction>, PermissionError>> {
        Box::pin(async move {
            check(&cancellation)?;
            if self.builtins.has_registered_tool(invocation.tool_name)? {
                return self
                    .builtin_preparer
                    .prepare(request, invocation, cancellation)
                    .await;
            }
            let permit = Permit::acquire(&self.active)?;
            let context = self
                .contexts
                .snapshot_for_permission(request)
                .map_err(|_| invalid())?;
            let review_context = self
                .review_contexts
                .snapshot(request)
                .map_err(|_| invalid())?;
            let workspace_scope = self
                .workspace_contexts
                .as_ref()
                .map(|contexts| contexts.snapshot_for_permission(request))
                .transpose()
                .map_err(|_| invalid())?;
            let workspace: Arc<str> = match &workspace_scope {
                Some(scope) => scope
                    .snapshot()
                    .map_err(|_| invalid())?
                    .primary_identity()
                    .to_str()
                    .ok_or_else(invalid)?
                    .into(),
                None => self.workspace.clone(),
            };
            if review_context.permission_policy().is_none()
                || review_context.target_call_id() != invocation.call_id
            {
                return Err(invalid());
            }
            let resolve =
                self.authority
                    .resolve(request, invocation, &context, cancellation.clone());
            let cancelled = async {
                select(cancellation.cancelled(), context.cancelled()).await;
            };
            let projection = match select(resolve, Box::pin(cancelled)).await {
                Either::Left((result, _)) => result?.ok_or_else(invalid)?,
                Either::Right(_) => return Err(invalid()),
            };
            check(&cancellation)?;
            projection.revalidate().map_err(|_| invalid())?;
            let evidence = action::Evidence::new(&workspace, &projection);
            let prepared = context
                .registry()
                .map_err(|_| invalid())?
                .prepare_tool(request, invocation, projection, cancellation.clone())
                .await
                .map_err(|_| invalid())?;
            let action = action::Action::new(
                prepared,
                evidence,
                context,
                review_context,
                self.reviewer.clone(),
                permit,
                cancellation,
                request.session_id.clone(),
                workspace_scope,
            );
            action.revalidate()?;
            Ok(Box::new(action) as Box<dyn NativePreparedPermissionAction>)
        })
    }
}

struct Permit(Arc<AtomicUsize>);
impl Permit {
    fn acquire(active: &Arc<AtomicUsize>) -> Result<Self, PermissionError> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_NATIVE_MCP_PERMISSION_PREPARATIONS).then_some(count + 1)
            })
            .map_err(|_| invalid())?;
        Ok(Self(active.clone()))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
fn check(cancellation: &CancellationToken) -> Result<(), PermissionError> {
    if cancellation.is_cancelled() {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn invalid() -> PermissionError {
    PermissionError::new(
        "mcp_permission_preparation_failed",
        "MCP permission preparation failed",
    )
}

#[cfg(test)]
mod tests;
