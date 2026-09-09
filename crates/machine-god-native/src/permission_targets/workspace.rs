//! Scoped mutation evidence is acquired only on the preparer's owned worker.

use super::{NativePermissionTargetAuthority, invalid, paths, validate_workspace_binding};
use crate::file_approval::NativeFileEndpoint;
use crate::{NativeFileApprovalKind, NativeWorkspaceTurnScope};
use machine_god_core::{PermissionError, PermissionRequest, ToolCall};
use std::path::Path;

pub(crate) struct ScopedFileEndpoints {
    pub(crate) scope: NativeWorkspaceTurnScope,
    pub(crate) source: Option<NativeFileEndpoint>,
    pub(crate) target: NativeFileEndpoint,
}

impl NativePermissionTargetAuthority {
    pub(crate) fn file_endpoints_on_worker(
        &self,
        request: &PermissionRequest,
        call: &ToolCall,
    ) -> Result<Option<ScopedFileEndpoints>, PermissionError> {
        let Some(contexts) = &self.workspace_contexts else {
            return Ok(None);
        };
        let scope = contexts
            .snapshot_for_permission(request)
            .map_err(|_| invalid())?;
        validate_workspace_binding(&self.root, &self.workspace, Some(&scope))?;
        let kind = match call.name.as_str() {
            "copy_file" => NativeFileApprovalKind::Copy,
            "rename_file" => NativeFileApprovalKind::Rename,
            "write_file" => NativeFileApprovalKind::Write,
            "edit_file" => NativeFileApprovalKind::Edit,
            "delete_file" => NativeFileApprovalKind::Delete,
            _ => return Err(invalid()),
        };
        let projection = crate::workspace_mutation::project(
            &scope.snapshot().map_err(|_| invalid())?,
            kind,
            &call.arguments,
        )
        .map_err(|_| invalid())?;
        if projection.logical_arguments() != &call.arguments {
            return Err(invalid());
        }
        let source = projection
            .source()
            .map(|endpoint| endpoint.retain().map_err(|_| invalid()))
            .transpose()?;
        let target = projection.target().retain().map_err(|_| invalid())?;
        if !scope.is_live() {
            return Err(invalid());
        }
        Ok(Some(ScopedFileEndpoints {
            scope,
            source,
            target,
        }))
    }
}

pub(super) fn file_path(
    workspace: &str,
    scope: Option<&NativeWorkspaceTurnScope>,
    raw: &str,
) -> Result<String, PermissionError> {
    let Some(scope) = scope else {
        return paths::absolute(workspace, raw);
    };
    let route = scope
        .snapshot()
        .map_err(|_| invalid())?
        .route(Path::new(raw))
        .map_err(|_| invalid())?;
    let absolute = route.root_identity().join(route.relative_path());
    let absolute = absolute.to_str().ok_or_else(invalid)?.to_owned();
    paths::validate_workspace(&absolute)?;
    Ok(absolute)
}
