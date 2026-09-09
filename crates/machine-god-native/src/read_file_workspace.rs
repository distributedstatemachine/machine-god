//! Exact-turn path projection; descriptor duplication occurs only in execution.

use super::{
    MAX_READ_FILE_PATH_BYTES, ReadFileTool, check_cancellation, decode_arguments,
    invalid_arguments, normalize_relative_path, read_file_name,
};
use crate::{NativeWorkspaceContexts, NativeWorkspaceRoute, NativeWorkspaceTurnScope};
use machine_god_core::{
    CancellationToken, Capability, FilesystemAccess, PreparedToolCall, ToolCall, ToolContext,
    ToolError, ToolErrorKind, ToolOutput,
};
use serde_json::{Value, json};
use std::path::Path;

struct Projection {
    scope: NativeWorkspaceTurnScope,
    route: NativeWorkspaceRoute,
    logical: String,
    relative: String,
}

fn project(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    raw: &str,
) -> Result<Projection, ToolError> {
    if raw.is_empty() || raw.len() > MAX_READ_FILE_PATH_BYTES || raw.contains('\0') {
        return Err(invalid_arguments());
    }
    let scope = contexts
        .snapshot_for_tool(context)
        .map_err(|_| unavailable())?;
    let route = scope
        .snapshot()
        .map_err(|_| unavailable())?
        .route(Path::new(raw))
        .map_err(|_| invalid_arguments())?;
    let relative = route
        .relative_path()
        .to_str()
        .ok_or_else(invalid_arguments)?;
    let relative = normalize_relative_path(relative)?;
    let logical = if Path::new(raw).is_absolute() {
        route
            .root_identity()
            .join(&relative)
            .to_str()
            .ok_or_else(invalid_arguments)?
            .to_owned()
    } else {
        relative.clone()
    };
    if logical.len() > MAX_READ_FILE_PATH_BYTES {
        return Err(invalid_arguments());
    }
    Ok(Projection {
        scope,
        route,
        logical,
        relative,
    })
}

pub(super) fn prepare(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    call: ToolCall,
) -> Result<PreparedToolCall, ToolError> {
    if call.name != read_file_name() {
        return Err(invalid_arguments());
    }
    let raw = decode_arguments(call.arguments)?.path;
    let projection = project(contexts, context, &raw)?;
    Ok(PreparedToolCall::new(
        Capability::Filesystem {
            access: FilesystemAccess::Read,
            path: projection.logical.clone(),
        },
        json!({ "path": projection.logical }),
    ))
}

pub(super) fn execute(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    arguments: Value,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    let raw = decode_arguments(arguments)?.path;
    let projection = project(contexts, context, &raw)?;
    if raw != projection.logical {
        return Err(invalid_arguments());
    }
    let descriptor = projection
        .route
        .root_descriptor()
        .try_clone()
        .map_err(|_| unavailable())?;
    if !projection.scope.is_live() {
        return Err(unavailable());
    }
    ReadFileTool::from_root_descriptor(descriptor).execute_unix(&projection.relative, cancellation)
}

fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::PermissionDenied,
        "workspace_context_unavailable",
        "workspace context is unavailable",
        false,
    )
}

#[cfg(test)]
#[path = "read_file_workspace/tests.rs"]
mod tests;
