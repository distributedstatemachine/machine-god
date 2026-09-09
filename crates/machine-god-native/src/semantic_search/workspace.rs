//! Exact-turn lexical search. Additional roots are selected, never fanned out.

use super::{
    ExecutionArguments, MAX_SEMANTIC_SEARCH_PATH_BYTES, decode_execution_arguments,
    decode_requested_arguments, invalid_arguments, invalid_path, is_forbidden_path_character,
    normalize_query, normalize_relative_path, semantic_search_name,
};
use crate::workspace_path_tools::{ensure_live, project, unavailable};
use crate::{
    NativeWorkspaceContextError, NativeWorkspaceContexts, NativeWorkspaceRoute,
    NativeWorkspaceTurnScope,
};
use machine_god_core::{
    CancellationToken, Capability, FilesystemAccess, PreparedToolCall, ToolCall, ToolContext,
    ToolError, ToolOutput,
};
use serde_json::Value;
use std::path::Path;

fn validate_path(raw: &str) -> Result<(), ToolError> {
    if raw.is_empty()
        || raw.len() > MAX_SEMANTIC_SEARCH_PATH_BYTES
        || raw.chars().any(is_forbidden_path_character)
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
    if call.name != semantic_search_name() {
        return Err(invalid_arguments());
    }
    let requested = decode_requested_arguments(call.arguments)?;
    let query = normalize_query(&requested.query)?;
    let raw = requested.path.as_deref().unwrap_or(".");
    validate_path(raw)?;
    let projected = project(
        contexts,
        context,
        raw,
        MAX_SEMANTIC_SEARCH_PATH_BYTES,
        normalize_relative_path,
        invalid_path,
    )?;
    validate_path(&projected.logical)?;
    let arguments = ExecutionArguments {
        query,
        path: projected.logical,
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
    if normalize_query(&arguments.query)? != arguments.query {
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

#[cfg(target_os = "linux")]
struct ScopedCheck<'a> {
    scope: &'a NativeWorkspaceTurnScope,
    cancellation: &'a CancellationToken,
}
#[cfg(target_os = "linux")]
impl super::ScanCheck for ScopedCheck<'_> {
    fn check(&self) -> Result<(), ToolError> {
        super::check_cancellation(self.cancellation)?;
        ensure_live(self.scope)
    }
}

pub(super) fn execute(
    scope: Result<NativeWorkspaceTurnScope, NativeWorkspaceContextError>,
    value: Value,
    cancellation: &CancellationToken,
) -> Result<ToolOutput, ToolError> {
    #[cfg(target_os = "linux")]
    super::check_cancellation(cancellation)?;
    let scope = scope.map_err(|_| unavailable())?;
    let arguments = decode_execution_arguments(value)?;
    let (route, relative) = canonical_route(&scope, &arguments)?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (route, relative, cancellation);
        Err(super::unsupported_platform())
    }
    #[cfg(target_os = "linux")]
    {
        let check = ScopedCheck {
            scope: &scope,
            cancellation,
        };
        // The shared scanner extracts keywords before invoking this closure.
        // Thus even descriptor duplication is absent for a stopword-only query.
        super::SemanticSearchTool::execute_scan(&arguments, &check, || {
            super::check_cancellation(&check)?;
            let root = route
                .root_descriptor()
                .try_clone()
                .map_err(|_| unavailable())?;
            super::check_cancellation(&check)?;
            let tool = super::SemanticSearchTool::from_root_descriptor(root);
            tool.open_search_root(&relative, &check)
        })
    }
}

#[cfg(test)]
mod tests;
