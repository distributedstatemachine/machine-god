use super::{
    CreateFolderTool, MAX_CREATE_FOLDER_PATH_BYTES, MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES,
    NativeCreateFolderEvidence, build_success_output, check_cancellation, create_folder_name,
    invalid_arguments, is_forbidden_path_character, normalize_relative_path, serialized_value_fits,
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
    arguments: &Value,
) -> Result<Projection, ToolError> {
    let object = arguments.as_object().ok_or_else(invalid_arguments)?;
    if object.len() != 1 {
        return Err(invalid_arguments());
    }
    let raw = object
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(invalid_arguments)?;
    if raw.len() > MAX_CREATE_FOLDER_PATH_BYTES
        || raw.chars().any(is_forbidden_path_character)
        || raw.starts_with('~')
        || !serialized_value_fits(arguments, MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES)
    {
        return Err(invalid_arguments());
    }
    let projected = project(
        contexts,
        context,
        raw,
        MAX_CREATE_FOLDER_PATH_BYTES,
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
    call: &ToolCall,
) -> Result<PreparedToolCall, ToolError> {
    if call.name != create_folder_name() {
        return Err(invalid_arguments());
    }
    let projection = projection(contexts, context, &call.arguments)?;
    let arguments = json!({"path": projection.logical});
    if !serialized_value_fits(&arguments, MAX_CREATE_FOLDER_SERIALIZED_ARGUMENT_BYTES) {
        return Err(invalid_arguments());
    }
    Ok(PreparedToolCall::new(
        Capability::Filesystem {
            access: FilesystemAccess::Create,
            path: projection.logical,
        },
        arguments,
    ))
}

pub(super) fn execute(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    arguments: &Value,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    let projection = projection(contexts, context, arguments)?;
    if arguments["path"].as_str() != Some(projection.logical.as_str()) {
        return Err(invalid_arguments());
    }
    let success = build_success_output(&projection.logical)?;
    let root = projection
        .route
        .root_descriptor()
        .try_clone()
        .map_err(|_| unavailable())?;
    ensure_live(&projection.scope)?;
    let mut evidence = NativeCreateFolderEvidence {
        scope: Some(projection.scope),
    };
    CreateFolderTool::from_root_descriptor(root).execute_supported_with_evidence(
        &projection.relative,
        cancellation,
        &mut evidence,
    )?;
    Ok(success)
}
