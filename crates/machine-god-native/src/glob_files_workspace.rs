//! One captured workspace root per search, with qualified bytes charged in scan.

use super::{
    GlobFilesTool, GlobMode, MAX_GLOB_FILES_PATH_BYTES, check_cancellation,
    decode_execution_arguments, decode_requested_arguments, glob_files_name, invalid_arguments,
    invalid_path, is_forbidden_path_character, normalize_pattern, normalize_relative_path,
    render_results, scan_tree,
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
    if raw.len() > MAX_GLOB_FILES_PATH_BYTES || raw.chars().any(is_forbidden_path_character) {
        return Err(invalid_path());
    }
    let projected = project(
        contexts,
        context,
        raw,
        MAX_GLOB_FILES_PATH_BYTES,
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
    if call.name != glob_files_name() {
        return Err(invalid_arguments());
    }
    let arguments = decode_requested_arguments(call.arguments)?;
    let pattern = normalize_pattern(&arguments.pattern)?;
    let raw = arguments.path.unwrap_or_else(|| ".".to_owned());
    let projected = projection(contexts, context, &raw)?;
    let mode = arguments.mode.unwrap_or(GlobMode::Matches);
    Ok(PreparedToolCall::new(
        Capability::Filesystem {
            access: FilesystemAccess::EnumerateRecursive,
            path: projected.logical.clone(),
        },
        json!({"path": projected.logical, "pattern": pattern, "mode": mode.as_str()}),
    ))
}

pub(super) fn execute(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    arguments: Value,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    let arguments = decode_execution_arguments(arguments)?;
    let pattern = normalize_pattern(&arguments.pattern)?;
    let projected = projection(contexts, context, &arguments.path)?;
    if arguments.path != projected.logical || arguments.pattern != pattern {
        return Err(invalid_arguments());
    }
    ensure_live(&projected.scope)?;
    let descriptor = projected
        .route
        .root_descriptor()
        .try_clone()
        .map_err(|_| unavailable())?;
    ensure_live(&projected.scope)?;
    let tool = GlobFilesTool::from_root_descriptor(descriptor);
    let search_root = tool.open_search_root(&projected.relative, cancellation)?;
    ensure_live(&projected.scope)?;
    // Matching remains search-root-relative. Only candidate output spelling
    // uses this prefix, before the existing path, retained and output bounds.
    let results = scan_tree(&pattern, &projected.logical, search_root, cancellation)?;
    let output = render_results(
        &pattern,
        &projected.logical,
        arguments.mode,
        results,
        cancellation,
    )?;
    check_cancellation(cancellation)?;
    ensure_live(&projected.scope)?;
    Ok(output)
}

#[cfg(test)]
#[path = "glob_files_workspace/tests.rs"]
mod tests;
