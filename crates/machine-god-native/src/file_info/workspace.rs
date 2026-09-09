use super::{
    FileInfoTool, MAX_FILE_INFO_PATH_BYTES, check_cancellation, decode_arguments, file_info_name,
    invalid_arguments, is_forbidden_path_character, normalize_relative_path,
};
use crate::{
    NativeWorkspaceContexts,
    workspace_path_tools::{Projection, ensure_live, project, unavailable},
};
use machine_god_core::{
    CancellationToken, Capability, FilesystemAccess, PreparedToolCall, ToolCall, ToolContext,
    ToolError, ToolOutput,
};
use serde_json::{Value, json};

fn projection(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    raw: &str,
) -> Result<Projection, ToolError> {
    if raw.len() > MAX_FILE_INFO_PATH_BYTES || raw.chars().any(is_forbidden_path_character) {
        return Err(invalid_arguments());
    }
    let projected = project(
        contexts,
        context,
        raw,
        MAX_FILE_INFO_PATH_BYTES,
        normalize_relative_path,
        invalid_arguments,
    )?;
    if projected.logical.chars().any(is_forbidden_path_character) {
        return Err(invalid_arguments());
    }
    Ok(projected)
}

pub(super) fn prepare(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    call: ToolCall,
) -> Result<PreparedToolCall, ToolError> {
    if call.name != file_info_name() {
        return Err(invalid_arguments());
    }
    let raw = decode_arguments(call.arguments)?.path;
    let projection = projection(contexts, context, &raw)?;
    Ok(PreparedToolCall::new(
        Capability::Filesystem {
            access: FilesystemAccess::Metadata,
            path: projection.logical.clone(),
        },
        json!({"path": projection.logical}),
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
    let projection = projection(contexts, context, &raw)?;
    if raw != projection.logical {
        return Err(invalid_arguments());
    }
    let root = projection
        .route
        .root_descriptor()
        .try_clone()
        .map_err(|_| unavailable())?;
    ensure_live(&projection.scope)?;
    let mut output = FileInfoTool::from_root_descriptor(root)
        .execute_unix(&projection.relative, cancellation)?;
    ensure_live(&projection.scope)?;
    output.content["path"] = Value::String(projection.logical);
    Ok(output)
}
