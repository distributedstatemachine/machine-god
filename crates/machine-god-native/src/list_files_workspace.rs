//! Exact-turn enumeration; the selected descriptor is retained only on execution.

use super::{
    ListFilesTool, MAX_LIST_FILES_PATH_BYTES, check_cancellation, decode_execution_arguments,
    decode_requested_arguments, invalid_arguments, invalid_path, is_forbidden_path_character,
    list_files_name, normalize_relative_path,
};
use crate::NativeWorkspaceContexts;
use crate::workspace_path_tools::{Projection, ensure_live, project, unavailable};
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
    if raw.len() > MAX_LIST_FILES_PATH_BYTES || raw.chars().any(is_forbidden_path_character) {
        return Err(invalid_path());
    }
    let projected = project(
        contexts,
        context,
        raw,
        MAX_LIST_FILES_PATH_BYTES,
        normalize_relative_path,
        invalid_path,
    )?;
    if projected.logical.chars().any(is_forbidden_path_character) {
        return Err(invalid_path());
    }
    Ok(projected)
}

pub(super) fn prepare(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    call: ToolCall,
) -> Result<PreparedToolCall, ToolError> {
    if call.name != list_files_name() {
        return Err(invalid_arguments());
    }
    let raw = decode_requested_arguments(call.arguments)?.unwrap_or_else(|| ".".to_owned());
    let projected = projection(contexts, context, &raw)?;
    Ok(PreparedToolCall::new(
        Capability::Filesystem {
            access: FilesystemAccess::Enumerate,
            path: projected.logical.clone(),
        },
        json!({ "path": projected.logical }),
    ))
}

pub(super) fn execute(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    arguments: Value,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    let raw = decode_execution_arguments(arguments)?;
    let projected = projection(contexts, context, &raw)?;
    if raw != projected.logical {
        return Err(invalid_arguments());
    }
    ensure_live(&projected.scope)?;
    let descriptor = projected
        .route
        .root_descriptor()
        .try_clone()
        .map_err(|_| unavailable())?;
    ensure_live(&projected.scope)?;
    let output = ListFilesTool::from_root_descriptor(descriptor).execute_unix_logical(
        &projected.relative,
        &projected.logical,
        cancellation,
    )?;
    check_cancellation(cancellation)?;
    ensure_live(&projected.scope)?;
    Ok(output)
}

#[cfg(test)]
#[path = "list_files_workspace/tests.rs"]
mod tests;
