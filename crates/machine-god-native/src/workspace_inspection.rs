//! Bounded lexical inspection of the process current workspace.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Inclusive maximum UTF-8 byte length of a reported current-workspace path.
pub const MAX_WORKSPACE_PATH_BYTES: usize = 4096;

/// One bounded lexical snapshot of the process current workspace.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeWorkspaceInspection {
    primary_workspace: PathBuf,
}

impl NativeWorkspaceInspection {
    /// Returns the captured absolute Unicode current-workspace path.
    #[must_use]
    pub fn primary_workspace(&self) -> &Path {
        &self.primary_workspace
    }
}

impl fmt::Debug for NativeWorkspaceInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeWorkspaceInspection")
            .finish_non_exhaustive()
    }
}

/// Stable category for native workspace-inspection failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeWorkspaceInspectionErrorKind {
    /// The current directory was unavailable or was not a valid workspace path.
    Unavailable,
    /// The captured path exceeded the fixed UTF-8 byte limit.
    ResourceLimit,
}

impl NativeWorkspaceInspectionErrorKind {
    /// Returns the stable, machine-readable name of this category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::ResourceLimit => "resource_limit",
        }
    }
}

/// Fixed, redacted failure to inspect the process current workspace.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NativeWorkspaceInspectionError {
    kind: NativeWorkspaceInspectionErrorKind,
}

impl NativeWorkspaceInspectionError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub const fn kind(&self) -> NativeWorkspaceInspectionErrorKind {
        self.kind
    }

    const fn new(kind: NativeWorkspaceInspectionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for NativeWorkspaceInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeWorkspaceInspectionError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for NativeWorkspaceInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            NativeWorkspaceInspectionErrorKind::Unavailable => {
                "native workspace inspection is unavailable"
            }
            NativeWorkspaceInspectionErrorKind::ResourceLimit => {
                "native workspace inspection exceeded a resource limit"
            }
        })
    }
}

impl Error for NativeWorkspaceInspectionError {}

/// Captures one bounded lexical snapshot of the process current workspace.
///
/// The current directory is requested exactly once. Inspection requires an
/// absolute Unicode path without a lexical parent component and performs no
/// metadata access, canonicalization, configuration or state discovery,
/// filesystem mutation, runtime construction, or network access.
///
/// # Errors
///
/// Returns a fixed, redacted error when current-directory capture fails, the
/// captured path is not an accepted lexical workspace, or its UTF-8 encoding
/// exceeds [`MAX_WORKSPACE_PATH_BYTES`].
pub fn inspect_process_workspace()
-> Result<NativeWorkspaceInspection, NativeWorkspaceInspectionError> {
    inspect_process_workspace_with(std::env::current_dir)
}

fn inspect_process_workspace_with(
    current_directory: impl FnOnce() -> io::Result<PathBuf>,
) -> Result<NativeWorkspaceInspection, NativeWorkspaceInspectionError> {
    let primary_workspace = current_directory().map_err(|_| {
        NativeWorkspaceInspectionError::new(NativeWorkspaceInspectionErrorKind::Unavailable)
    })?;
    validate_workspace_path(&primary_workspace)?;
    Ok(NativeWorkspaceInspection { primary_workspace })
}

fn validate_workspace_path(path: &Path) -> Result<(), NativeWorkspaceInspectionError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(NativeWorkspaceInspectionError::new(
            NativeWorkspaceInspectionErrorKind::Unavailable,
        ));
    }
    let path = path.to_str().ok_or_else(|| {
        NativeWorkspaceInspectionError::new(NativeWorkspaceInspectionErrorKind::Unavailable)
    })?;
    if path.len() > MAX_WORKSPACE_PATH_BYTES {
        return Err(NativeWorkspaceInspectionError::new(
            NativeWorkspaceInspectionErrorKind::ResourceLimit,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{
        MAX_WORKSPACE_PATH_BYTES, NativeWorkspaceInspectionErrorKind,
        inspect_process_workspace_with,
    };

    fn absolute_unicode_path(byte_length: usize) -> PathBuf {
        #[cfg(target_os = "windows")]
        let prefix = "C:\\";
        #[cfg(not(target_os = "windows"))]
        let prefix = "/";

        assert!(byte_length >= prefix.len());
        PathBuf::from(format!(
            "{prefix}{}",
            "a".repeat(byte_length - prefix.len())
        ))
    }

    #[test]
    fn captures_the_current_directory_exactly_once_and_preserves_the_snapshot() {
        let calls = Cell::new(0usize);
        let expected = absolute_unicode_path(37);
        let inspection = inspect_process_workspace_with(|| {
            calls.set(calls.get() + 1);
            Ok(expected.clone())
        })
        .unwrap();

        assert_eq!(calls.get(), 1);
        assert_eq!(inspection.primary_workspace(), expected);
        assert_eq!(
            format!("{inspection:?}"),
            "NativeWorkspaceInspection { .. }"
        );
    }

    #[test]
    fn current_directory_failure_is_fixed_and_redacted() {
        let error = inspect_process_workspace_with(|| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "/secret/workspace",
            ))
        })
        .unwrap_err();

        assert_eq!(
            error.kind(),
            NativeWorkspaceInspectionErrorKind::Unavailable
        );
        assert_eq!(error.kind().as_str(), "unavailable");
        assert_eq!(
            error.to_string(),
            "native workspace inspection is unavailable"
        );
        assert_eq!(
            format!("{error:?}"),
            "NativeWorkspaceInspectionError { kind: Unavailable }"
        );
        assert!(!format!("{error:?} {error}").contains("secret"));
    }

    #[test]
    fn relative_and_parent_paths_are_unavailable() {
        for path in [PathBuf::from("relative"), absolute_parent_path()] {
            let error = inspect_process_workspace_with(|| Ok(path.clone())).unwrap_err();
            assert_eq!(
                error.kind(),
                NativeWorkspaceInspectionErrorKind::Unavailable
            );
        }
    }

    fn absolute_parent_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            PathBuf::from("C:\\workspace\\..\\other")
        }
        #[cfg(not(target_os = "windows"))]
        {
            PathBuf::from("/workspace/../other")
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_path_is_unavailable() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/workspace/\xff".to_vec()));
        let error = inspect_process_workspace_with(|| Ok(path.clone())).unwrap_err();

        assert_eq!(
            error.kind(),
            NativeWorkspaceInspectionErrorKind::Unavailable
        );
    }

    #[test]
    fn path_byte_limit_is_inclusive() {
        let boundary = absolute_unicode_path(MAX_WORKSPACE_PATH_BYTES);
        let accepted = inspect_process_workspace_with(|| Ok(boundary.clone())).unwrap();
        assert_eq!(accepted.primary_workspace(), Path::new(&boundary));

        let overflow = absolute_unicode_path(MAX_WORKSPACE_PATH_BYTES + 1);
        let error = inspect_process_workspace_with(|| Ok(overflow.clone())).unwrap_err();
        assert_eq!(
            error.kind(),
            NativeWorkspaceInspectionErrorKind::ResourceLimit
        );
        assert_eq!(error.kind().as_str(), "resource_limit");
        assert_eq!(
            error.to_string(),
            "native workspace inspection exceeded a resource limit"
        );
        assert_eq!(
            format!("{error:?}"),
            "NativeWorkspaceInspectionError { kind: ResourceLimit }"
        );
    }
}
