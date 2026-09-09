use super::{
    MAX_OPEN_FILE_PATH_BYTES, MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES, OpenFileTool,
    invalid_arguments, is_forbidden_path_character, open_file_name, serialized_value_fits,
    validate_canonical_path,
};
use crate::{
    NativeWorkspaceContexts,
    workspace_path_tools::{Projection, project},
};
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, PreparedToolCall, ToolCall, ToolContext, ToolError,
    ToolOutput,
};
use serde_json::{Value, json};

fn canonical(path: &str) -> Result<String, ToolError> {
    validate_canonical_path(path)?;
    Ok(path.to_owned())
}

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
    if raw.len() > MAX_OPEN_FILE_PATH_BYTES
        || raw.chars().any(is_forbidden_path_character)
        || raw.starts_with('~')
        || !serialized_value_fits(arguments, MAX_OPEN_FILE_SERIALIZED_ARGUMENT_BYTES)
    {
        return Err(invalid_arguments());
    }
    let projected = project(
        contexts,
        context,
        raw,
        MAX_OPEN_FILE_PATH_BYTES,
        canonical,
        invalid_arguments,
    )?;
    // Unlike folder creation, open_file accepts canonical spelling only.
    if raw != projected.logical || projected.logical.chars().any(is_forbidden_path_character) {
        return Err(invalid_arguments());
    }
    Ok(projected)
}

pub(super) fn prepare(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    call: &ToolCall,
) -> Result<PreparedToolCall, ToolError> {
    if call.name != open_file_name() {
        return Err(invalid_arguments());
    }
    let projection = projection(contexts, context, &call.arguments)?;
    Ok(PreparedToolCall::new(
        Capability::OpenFile {
            path: projection.logical.clone(),
        },
        json!({"path": projection.logical}),
    ))
}

pub(super) fn execute<'a>(
    tool: &'a OpenFileTool,
    contexts: &'a NativeWorkspaceContexts,
    context: &'a ToolContext,
    arguments: Value,
    cancellation: CancellationToken,
) -> BoxFuture<'a, Result<ToolOutput, ToolError>> {
    Box::pin(async move {
        #[cfg(target_os = "linux")]
        super::check_cancellation(&cancellation)?;
        let projection = projection(contexts, context, &arguments)?;
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (tool, projection, cancellation);
            Err(super::unsupported_platform())
        }
        #[cfg(target_os = "linux")]
        {
            use crate::workspace_path_tools::{ensure_live, unavailable};
            let root = projection
                .route
                .root_descriptor()
                .try_clone()
                .map_err(|_| unavailable())?;
            ensure_live(&projection.scope)?;
            let scoped = OpenFileTool {
                root,
                launcher: tool.launcher.clone(),
                workspace_contexts: None,
            };
            let mut request = scoped.prepare_launch_request(projection.relative, &cancellation)?;
            request.path.clone_from(&projection.logical);
            ensure_live(&projection.scope)?;
            request.workspace_scope = Some(projection.scope);
            match scoped.launcher.launch(request, cancellation.clone()).await {
                super::OpenFileLaunchOutcome::Accepted => super::success(&projection.logical),
                super::OpenFileLaunchOutcome::Cancelled => Err(super::cancelled()),
                super::OpenFileLaunchOutcome::Unavailable => {
                    super::check_cancellation(&cancellation)?;
                    Err(super::launcher_unavailable())
                }
                super::OpenFileLaunchOutcome::ResultUnknown => Err(super::result_unknown()),
            }
        }
    })
}
