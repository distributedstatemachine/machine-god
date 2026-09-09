use super::{Kind, invalid};
use crate::file_approval::NativeFileEndpoint;
use crate::{NativeWorkspaceRoute, NativeWorkspaceScopeSnapshot};
use machine_god_core::{Capability, FilesystemAccess, ToolError};
use serde_json::Value;
use std::{path::Path, sync::Arc};

pub(crate) struct WorkspaceMutationEndpoint {
    route: NativeWorkspaceRoute,
    logical: String,
    relative: String,
}

impl WorkspaceMutationEndpoint {
    pub(crate) fn route(&self) -> &NativeWorkspaceRoute {
        &self.route
    }
    pub(crate) fn logical_path(&self) -> &str {
        &self.logical
    }
    pub(crate) fn relative_path(&self) -> &str {
        &self.relative
    }
    /// Explicit effect boundary: never call from `Tool::prepare_for_turn`.
    pub(crate) fn retain(&self) -> Result<NativeFileEndpoint, ToolError> {
        let descriptor = self
            .route()
            .root_descriptor()
            .try_clone()
            .map_err(|_| super::unavailable())?;
        NativeFileEndpoint::new(
            Arc::new(descriptor.into()),
            self.relative_path().to_owned(),
            self.logical_path().to_owned(),
        )
        .map_err(|_| invalid())
    }
}

pub(crate) struct WorkspaceMutationProjection {
    kind: Kind,
    logical: Value,
    private: Value,
    source: Option<WorkspaceMutationEndpoint>,
    target: WorkspaceMutationEndpoint,
}

impl WorkspaceMutationProjection {
    pub(crate) fn logical_arguments(&self) -> &Value {
        &self.logical
    }
    pub(crate) fn private_arguments(&self) -> &Value {
        &self.private
    }
    pub(crate) fn source(&self) -> Option<&WorkspaceMutationEndpoint> {
        self.source.as_ref()
    }
    pub(crate) fn target(&self) -> &WorkspaceMutationEndpoint {
        &self.target
    }
    pub(crate) fn capability(&self) -> Capability {
        match self.kind {
            Kind::Copy => Capability::FilesystemCopy {
                source: self.source.as_ref().expect("copy source").logical.clone(),
                destination: self.target.logical.clone(),
            },
            Kind::Rename => Capability::FilesystemRename {
                old_path: self.source.as_ref().expect("rename source").logical.clone(),
                new_path: self.target.logical.clone(),
            },
            kind => Capability::Filesystem {
                path: self.target.logical.clone(),
                access: match kind {
                    Kind::Write => FilesystemAccess::Write,
                    Kind::Edit => FilesystemAccess::Edit,
                    _ => FilesystemAccess::Delete,
                },
            },
        }
    }
}

pub(super) fn path_keys(kind: Kind) -> (Option<&'static str>, &'static str) {
    match kind {
        Kind::Copy => (Some("source"), "destination"),
        Kind::Rename => (Some("old_path"), "new_path"),
        _ => (None, "path"),
    }
}

pub(crate) fn project(
    snapshot: &NativeWorkspaceScopeSnapshot,
    kind: Kind,
    arguments: &Value,
) -> Result<WorkspaceMutationProjection, ToolError> {
    check_arguments_bound(arguments)?;
    let (source_key, target_key) = path_keys(kind);
    let target = endpoint(
        snapshot,
        arguments[target_key].as_str().ok_or_else(invalid)?,
    )?;
    let source = source_key
        .map(|key| endpoint(snapshot, arguments[key].as_str().ok_or_else(invalid)?))
        .transpose()?;
    let mut private = arguments.clone();
    let mut logical = arguments.clone();
    private[target_key] = target.relative.clone().into();
    logical[target_key] = target.logical.clone().into();
    if let Some(key) = source_key {
        let source = source.as_ref().ok_or_else(invalid)?;
        if source.logical == target.logical {
            return Err(invalid());
        }
        private[key] = source.relative.clone().into();
        logical[key] = source.logical.clone().into();
    }
    check_arguments_bound(&logical)?;
    let projection = WorkspaceMutationProjection {
        kind,
        logical,
        private,
        source,
        target,
    };
    validate_private(kind, projection.private_arguments())?;
    Ok(projection)
}

fn endpoint(
    snapshot: &NativeWorkspaceScopeSnapshot,
    raw: &str,
) -> Result<WorkspaceMutationEndpoint, ToolError> {
    if raw.is_empty() || raw.len() > 4096 || raw.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) { return Err(invalid()); }
    let route = snapshot.route(Path::new(raw)).map_err(|_| invalid())?;
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
    if logical.len() > 4096 {
        return Err(invalid());
    }
    Ok(WorkspaceMutationEndpoint {
        route,
        logical,
        relative,
    })
}

fn validate_private(kind: Kind, private: &Value) -> Result<(), ToolError> {
    match kind {
        Kind::Write => crate::write_file::validate_approval_arguments(private),
        Kind::Edit => crate::edit_file::validate_approval_arguments(private),
        Kind::Delete => crate::delete_file::validate_approval_arguments(private),
        Kind::Copy => crate::copy_file::validate_endpoint_arguments(private),
        Kind::Rename => crate::rename_file::validate_endpoint_arguments(private),
    }
    .map_err(|_| invalid())
}

fn check_arguments_bound(arguments: &Value) -> Result<(), ToolError> {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or_else(|| std::io::Error::other("argument limit"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let object = arguments.as_object().ok_or_else(invalid)?;
    if object.len() > 3
        || object.iter().any(|(key, value)| {
            key.len() > 32 || value.as_str().is_none_or(|value| value.len() > 64 * 1024)
        })
    {
        return Err(invalid());
    }
    serde_json::to_writer(Budget(64 * 1024), arguments).map_err(|_| invalid())
}
