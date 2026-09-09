//! Pure exact-turn path projection shared by scoped native path tools.

use crate::{NativeWorkspaceContexts, NativeWorkspaceRoute, NativeWorkspaceTurnScope};
use machine_god_core::{ToolContext, ToolError, ToolErrorKind};
use std::path::Path;

pub(crate) struct Projection {
    pub scope: NativeWorkspaceTurnScope,
    pub route: NativeWorkspaceRoute,
    pub logical: String,
    pub relative: String,
}

pub(crate) fn project(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    raw: &str,
    limit: usize,
    normalize: fn(&str) -> Result<String, ToolError>,
    invalid: fn() -> ToolError,
) -> Result<Projection, ToolError> {
    if raw.is_empty() || raw.len() > limit || raw.contains('\0') {
        return Err(invalid());
    }
    let scope = contexts
        .snapshot_for_tool(context)
        .map_err(|_| unavailable())?;
    let route = scope
        .snapshot()
        .map_err(|_| unavailable())?
        .route(Path::new(raw))
        .map_err(|_| invalid())?;
    let relative = route.relative_path().to_str().ok_or_else(invalid)?;
    let relative = normalize(if relative.is_empty() { "." } else { relative })?;
    let logical = if Path::new(raw).is_absolute() {
        let path = if relative == "." {
            route.root_identity().to_owned()
        } else {
            route.root_identity().join(&relative)
        };
        path.to_str().ok_or_else(invalid)?.to_owned()
    } else {
        relative.clone()
    };
    if logical.len() > limit {
        return Err(invalid());
    }
    Ok(Projection {
        scope,
        route,
        logical,
        relative,
    })
}

pub(crate) fn ensure_live(scope: &NativeWorkspaceTurnScope) -> Result<(), ToolError> {
    if scope.is_live() {
        Ok(())
    } else {
        Err(unavailable())
    }
}

pub(crate) fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "workspace_context_unavailable",
        "workspace context is unavailable",
        false,
    )
}

#[cfg(test)]
mod tests;
