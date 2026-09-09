use super::{Error, NativeFileApprovalKind as Kind};
use std::{fs::File, sync::Arc};

/// An exact logical label bound to an already retained root and private path.
/// Construction is pure; directory validity is checked by the effect boundary.
#[derive(Clone)]
pub(crate) struct NativeFileEndpoint {
    root: Arc<File>,
    relative_path: String,
    logical_path: String,
}

impl NativeFileEndpoint {
    pub(crate) fn new(
        root: Arc<File>,
        relative_path: String,
        logical_path: String,
    ) -> Result<Self, Error> {
        let invalid_text = |path: &str| {
            path.is_empty() || path.len() > 4096 || path.chars().any(|c| {
                c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
        };
        if invalid_text(&logical_path)
            || invalid_text(&relative_path)
            || relative_path.starts_with('/')
            || relative_path.split('/').count() > 256
            || relative_path
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            root,
            relative_path,
            logical_path,
        })
    }

    pub(crate) fn root(&self) -> &Arc<File> {
        &self.root
    }
    pub(crate) fn relative_path(&self) -> &str {
        &self.relative_path
    }
    pub(crate) fn logical_path(&self) -> &str {
        &self.logical_path
    }
}

pub(crate) fn project(
    kind: Kind,
    arguments: &serde_json::Value,
    source: Option<&NativeFileEndpoint>,
    target: &NativeFileEndpoint,
) -> Result<serde_json::Value, Error> {
    let (source_key, target_key) = match kind {
        Kind::Copy => (Some("source"), "destination"),
        Kind::Rename => (Some("old_path"), "new_path"),
        _ => (None, "path"),
    };
    if arguments[target_key].as_str() != Some(target.logical_path())
        || source_key.is_some() != source.is_some()
        || !arguments_fit(arguments)
    {
        return Err(Error::Invalid);
    }
    let mut private = arguments.clone();
    private[target_key] = target.relative_path().into();
    if let Some(key) = source_key {
        let source = source.ok_or(Error::Invalid)?;
        if arguments[key].as_str() != Some(source.logical_path()) {
            return Err(Error::Invalid);
        }
        private[key] = source.relative_path().into();
    }
    match kind {
        Kind::Write => crate::write_file::validate_approval_arguments(&private)?,
        Kind::Edit => crate::edit_file::validate_approval_arguments(&private)?,
        Kind::Delete => crate::delete_file::validate_approval_arguments(&private)?,
        Kind::Copy => crate::copy_file::validate_endpoint_arguments(&private)?,
        Kind::Rename => crate::rename_file::validate_endpoint_arguments(&private)?,
    }
    Ok(private)
}

fn arguments_fit(arguments: &serde_json::Value) -> bool {
    let Some(object) = arguments.as_object() else {
        return false;
    };
    object.len() <= 3
        && object.iter().all(|(key, value)| {
            key.len() <= 32 && value.as_str().is_some_and(|text| text.len() <= 64 * 1024)
        })
        && serde_json::to_vec(arguments).is_ok_and(|bytes| bytes.len() <= 64 * 1024)
}
