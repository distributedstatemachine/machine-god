//! Scoped mutation evidence is acquired only on the preparer's owned worker.

use super::{NativePermissionTargetAuthority, invalid, paths, validate_workspace_binding};
use crate::NativeWorkspaceTurnScope;
use crate::file_approval::NativeFileEndpoint;
use machine_god_core::{PermissionError, PermissionRequest, ToolCall};
use std::{fs::File, path::Path, sync::Arc};

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
        let (source_key, target_key) = match call.name.as_str() {
            "copy_file" => (Some("source"), "destination"),
            "rename_file" => (Some("old_path"), "new_path"),
            "write_file" | "edit_file" | "delete_file" => (None, "path"),
            _ => return Err(invalid()),
        };
        let endpoint = |key: &str| -> Result<NativeFileEndpoint, PermissionError> {
            let raw = call
                .arguments
                .get(key)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(invalid)?;
            let route = scope
                .snapshot()
                .map_err(|_| invalid())?
                .route(Path::new(raw))
                .map_err(|_| invalid())?;
            let relative = route
                .relative_path()
                .to_str()
                .ok_or_else(invalid)?
                .to_owned();
            let logical = if Path::new(raw).is_absolute() {
                route
                    .root_identity()
                    .join(&relative)
                    .to_str()
                    .ok_or_else(invalid)?
                    .to_owned()
            } else {
                relative.clone()
            };
            if logical != raw {
                return Err(invalid());
            }
            let root = File::from(route.root_descriptor().try_clone().map_err(|_| invalid())?);
            NativeFileEndpoint::new(Arc::new(root), relative, logical).map_err(|_| invalid())
        };
        let source = source_key.map(endpoint).transpose()?;
        let target = endpoint(target_key)?;
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
