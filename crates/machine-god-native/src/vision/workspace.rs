//! Pure image-path projection; all images still share one invocation pipeline.

use super::{
    BTreeMap, CanonicalVisionRequest, Capability, JsonValueOwner, MAX_VISION_PATH_BYTES, OwnedFd,
    PreparedToolCall, RenderedImage, ToolCall, ToolContext, ToolError, ToolOutput, VisionSources,
    VisionTool, canonical_request_with, invalid_arguments_error, invalid_path_error,
    is_forbidden_path_character, normalize_relative_path, render_results_in_source_order,
    vision_name,
};
use crate::NativeWorkspaceContexts;
use crate::workspace_path_tools::{Projection, project};

pub(super) fn canonical(
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    arguments: &JsonValueOwner,
) -> Result<(CanonicalVisionRequest, Vec<Projection>), ToolError> {
    let mut routes = Vec::new();
    let request = canonical_request_with(arguments, |raw| {
        // Preserve the strict image grammar even in the absolute root prefix.
        // In particular, lexical routing must not erase repeated separators or '.'.
        if raw.len() > MAX_VISION_PATH_BYTES {
            return Err(invalid_path_error());
        }
        normalize_relative_path(raw.strip_prefix('/').unwrap_or(raw))?;
        let projection = project(
            contexts,
            context,
            raw,
            MAX_VISION_PATH_BYTES,
            normalize_relative_path,
            invalid_path_error,
        )?;
        if projection.logical != raw || projection.logical.chars().any(is_forbidden_path_character)
        {
            return Err(invalid_path_error());
        }
        let logical = projection.logical.clone();
        routes.push(projection);
        Ok(logical)
    })?;
    Ok((request, routes))
}

pub(super) fn prepare(
    tool: &VisionTool,
    contexts: &NativeWorkspaceContexts,
    context: &ToolContext,
    call: ToolCall,
) -> Result<PreparedToolCall, ToolError> {
    let arguments = JsonValueOwner::new(call.arguments);
    if call.name != vision_name() {
        return Err(invalid_arguments_error());
    }
    let (request, _) = canonical(contexts, context, &arguments)?;
    match request.sources {
        VisionSources::Paths(paths) => Ok(PreparedToolCall::new(
            Capability::Vision {
                paths,
                target: tool.target.clone(),
            },
            request.arguments,
        )),
        VisionSources::ImageIds(_) => Ok(PreparedToolCall::without_authority(request.arguments)),
    }
}

pub(super) fn ensure_live(routes: &[Projection]) -> Result<(), ToolError> {
    if let Some(first) = routes.first() {
        crate::workspace_path_tools::ensure_live(&first.scope)?;
    }
    Ok(())
}

pub(super) fn location<'a>(
    routes: &'a [Projection],
    index: usize,
    primary: &'a OwnedFd,
    path: &'a str,
) -> Result<(&'a OwnedFd, &'a str), ToolError> {
    if routes.is_empty() {
        return Ok((primary, path));
    }
    let route = routes
        .get(index)
        .ok_or_else(crate::workspace_path_tools::unavailable)?;
    Ok((route.route.root_descriptor(), route.relative.as_str()))
}

pub(super) fn expiry_output(
    routes: &[Projection],
    source_count: usize,
    results: &mut BTreeMap<u64, RenderedImage>,
) -> Result<Option<ToolOutput>, ToolError> {
    if ensure_live(routes).is_ok() {
        return Ok(None);
    }
    if results.is_empty() {
        return Err(crate::workspace_path_tools::unavailable());
    }
    for index in 1..=source_count {
        let id = u64::try_from(index).expect("bounded vision source count fits u64");
        results
            .entry(id)
            .or_insert_with(|| RenderedImage::unavailable(id));
    }
    render_results_in_source_order(source_count, results).map(Some)
}

#[cfg(test)]
mod tests;
