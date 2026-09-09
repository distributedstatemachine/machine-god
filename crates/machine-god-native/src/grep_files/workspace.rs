//! Exact-turn content search. Logical labels enter the original scan budgets.

use super::{
    ExecutionArguments, GrepFilesTool, GrepMode, MAX_GREP_FILES_HEAD_LIMIT,
    MAX_GREP_FILES_PATH_BYTES, ScanCheck, check_cancellation, decode_execution_arguments,
    decode_requested_arguments, grep_files_name, invalid_arguments, invalid_path,
    is_forbidden_character, normalize_include_pattern, normalize_literal_pattern,
    normalize_relative_path,
};
use crate::workspace_path_tools::{ensure_live, project, unavailable};
use crate::{
    NativeWorkspaceContextError, NativeWorkspaceContexts, NativeWorkspaceRoute,
    NativeWorkspaceTurnScope,
};
use machine_god_core::{
    CancellationToken, Capability, FilesystemAccess, PermissionInvocation, PermissionRequest,
    PreparedToolCall, ToolCall, ToolContext, ToolError, ToolOutput,
};
use serde_json::Value;
use std::path::Path;

fn validate_path(raw: &str) -> Result<(), ToolError> {
    if raw.is_empty()
        || raw.len() > MAX_GREP_FILES_PATH_BYTES
        || raw.chars().any(is_forbidden_character)
    {
        return Err(invalid_path());
    }
    Ok(())
}

pub(super) fn prepare(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    call: ToolCall,
) -> Result<PreparedToolCall, ToolError> {
    if call.name != grep_files_name() {
        return Err(invalid_arguments());
    }
    let requested = decode_requested_arguments(call.arguments)?;
    let raw = requested.path.as_deref().unwrap_or(".");
    validate_path(raw)?;
    let projected = project(
        contexts,
        context,
        raw,
        MAX_GREP_FILES_PATH_BYTES,
        normalize_relative_path,
        invalid_path,
    )?;
    validate_path(&projected.logical)?;
    let arguments = ExecutionArguments {
        pattern: normalize_literal_pattern(&requested.pattern)?,
        path: projected.logical,
        include: requested
            .include
            .as_deref()
            .map(normalize_include_pattern)
            .transpose()?,
        case_insensitive: requested.case_insensitive.unwrap_or(false),
        mode: requested.mode.unwrap_or(GrepMode::Matches),
        head_limit: requested.head_limit.unwrap_or(MAX_GREP_FILES_HEAD_LIMIT),
        offset: requested.offset.unwrap_or(0),
        context_lines: requested.context_lines.unwrap_or(0),
    };
    ensure_live(&projected.scope)?;
    Ok(PreparedToolCall::new(
        Capability::Filesystem {
            access: FilesystemAccess::SearchContent,
            path: arguments.path.clone(),
        },
        arguments.as_json(),
    ))
}

fn canonical_route(
    scope: &NativeWorkspaceTurnScope,
    arguments: &ExecutionArguments,
) -> Result<(NativeWorkspaceRoute, String), ToolError> {
    validate_path(&arguments.path)?;
    if normalize_literal_pattern(&arguments.pattern)? != arguments.pattern
        || arguments
            .include
            .as_deref()
            .map(normalize_include_pattern)
            .transpose()?
            != arguments.include
    {
        return Err(invalid_arguments());
    }
    let route = scope
        .snapshot()
        .map_err(|_| unavailable())?
        .route(Path::new(&arguments.path))
        .map_err(|_| invalid_path())?;
    let relative = route.relative_path().to_str().ok_or_else(invalid_path)?;
    let relative = normalize_relative_path(if relative.is_empty() { "." } else { relative })?;
    let logical = if Path::new(&arguments.path).is_absolute() {
        let path = if relative == "." {
            route.root_identity().to_owned()
        } else {
            route.root_identity().join(&relative)
        };
        path.to_str().ok_or_else(invalid_path)?.to_owned()
    } else {
        relative.clone()
    };
    if logical != arguments.path {
        return Err(invalid_arguments());
    }
    ensure_live(scope)?;
    Ok((route, relative))
}

pub(super) fn validate_permission(
    contexts: &NativeWorkspaceContexts,
    request: &PermissionRequest,
    invocation: PermissionInvocation<'_>,
) -> Result<(), ToolError> {
    let scope = contexts
        .snapshot_for_permission(request)
        .map_err(|_| unavailable())?;
    let arguments = decode_execution_arguments(invocation.arguments.clone())?;
    canonical_route(&scope, &arguments)?;
    if arguments.as_json() != *invocation.arguments
        || request.capability
            != (Capability::Filesystem {
                access: FilesystemAccess::SearchContent,
                path: arguments.path,
            })
    {
        return Err(invalid_arguments());
    }
    ensure_live(&scope)
}

struct ScopedCheck<'a> {
    scope: &'a NativeWorkspaceTurnScope,
    cancellation: &'a CancellationToken,
}
impl ScanCheck for ScopedCheck<'_> {
    fn check(&self) -> Result<(), ToolError> {
        self.cancellation.check()?;
        ensure_live(self.scope)
    }
}

pub(super) fn execute(
    scope: Result<NativeWorkspaceTurnScope, NativeWorkspaceContextError>,
    value: Value,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    check_cancellation(cancellation)?;
    let scope = scope.map_err(|_| unavailable())?;
    let arguments = decode_execution_arguments(value)?;
    let (route, relative) = canonical_route(&scope, &arguments)?;
    let check = ScopedCheck {
        scope: &scope,
        cancellation,
    };
    check.check()?;
    let root = route
        .root_descriptor()
        .try_clone()
        .map_err(|_| unavailable())?;
    check.check()?;
    let tool = GrepFilesTool::from_root_descriptor(root);
    // Matching stays selected-root-relative; path construction and every result
    // budget see the qualified label before retaining or rendering anything.
    tool.execute_unix_at(&arguments, &relative, &check)
}

#[cfg(test)]
mod tests;
