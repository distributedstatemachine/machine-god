//! Explicit, owned startup for administrative and conversation workspace scopes.

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use machine_god_core::BoxFuture;
use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags};

use crate::workspace_authority::{NativeWorkspaceAuthority, NativeWorkspaceSource};
use crate::workspace_service::{
    NativeWorkspaceServiceError as Error, merge_workspace_sources_blocking,
};
use crate::{NativeOwnedWorkerScope, NativeRootSelection, NativeUserConfigStore};

const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
const MAX_PATH_BYTES: usize = 4096;
/// Bounds startup work even when repeated launch arguments resolve to one root.
pub const MAX_WORKSPACE_LAUNCH_ARGUMENTS: usize = 64;

/// Opens an explicit workspace scope on the caller's owned worker scope.
///
/// Construction is inert. Polling reads the selected roots and native settings;
/// it never discovers environment, creates state/configuration directories,
/// obtains credentials, or starts a provider. Dropping a started response does
/// not detach the worker: the host must close and settle its worker scope.
#[must_use]
pub fn prepare_native_workspace(
    selection: NativeRootSelection,
    store: Arc<NativeUserConfigStore>,
    launch: Vec<PathBuf>,
    saved_suppressed: bool,
    workers: NativeOwnedWorkerScope,
) -> BoxFuture<'static, Result<NativeWorkspaceAuthority, Error>> {
    Box::pin(async move {
        workers
            .run(move || {
                prepare_workspace_blocking(&selection, Some(&store), &launch, saved_suppressed)
            })
            .await
            .map_err(|_| Error::Unavailable)?
    })
}

/// Opens launch-only workspace authority without observing any settings path.
/// Construction is inert; polling and started-worker ownership match
/// [`prepare_native_workspace`]. No saved roots are inferred or discovered.
#[must_use]
pub fn prepare_native_workspace_without_settings(
    selection: NativeRootSelection,
    launch: Vec<PathBuf>,
    saved_suppressed: bool,
    workers: NativeOwnedWorkerScope,
) -> BoxFuture<'static, Result<NativeWorkspaceAuthority, Error>> {
    Box::pin(async move {
        workers
            .run(move || prepare_workspace_blocking(&selection, None, &launch, saved_suppressed))
            .await
            .map_err(|_| Error::Unavailable)?
    })
}

fn prepare_workspace_blocking(
    selection: &NativeRootSelection,
    store: Option<&NativeUserConfigStore>,
    launch: &[PathBuf],
    saved_suppressed: bool,
) -> Result<NativeWorkspaceAuthority, Error> {
    if launch.len() > MAX_WORKSPACE_LAUNCH_ARGUMENTS {
        return Err(Error::InvalidPath);
    }
    validate_path(selection.workspace_root())?;
    validate_path(selection.state_root())?;
    for path in launch {
        validate_path(path)?;
    }
    let primary = open_directory(selection.workspace_root())?;
    let primary_identity =
        std::fs::canonicalize(selection.workspace_root()).map_err(|_| Error::Unavailable)?;
    let (state, state_identity) =
        match rustix::fs::open(selection.state_root(), DIRECTORY, Mode::empty()) {
            Ok(descriptor) => (
                Some(descriptor),
                std::fs::canonicalize(selection.state_root()).map_err(|_| Error::Unavailable)?,
            ),
            Err(rustix::io::Errno::NOENT) => (None, selection.state_root().to_owned()),
            Err(_) => return Err(Error::Unavailable),
        };
    let saved = store
        .map(|store| {
            store
                .load()
                .map_err(Error::Config)?
                .loaded()
                .config()
                .saved_workspace_directories(primary_identity.as_os_str().as_bytes())
                .map(<[_]>::to_vec)
                .map_err(|error| Error::Config(crate::NativeUserConfigError::InvalidConfig(error)))
        })
        .transpose()?
        .unwrap_or_default();
    let launch = launch
        .iter()
        .map(|path| resolve_launch(&primary_identity, path))
        .collect::<Result<Vec<_>, _>>()?;
    let entries = merge_workspace_sources_blocking(&saved, &launch)?;
    let authority = NativeWorkspaceAuthority::open_blocking(
        primary,
        primary_identity,
        state,
        state_identity,
        entries,
        saved_suppressed,
    )
    .map_err(Error::Authority)?;
    if authority
        .snapshot()
        .map_err(Error::Authority)?
        .entries()
        .iter()
        .any(|entry| entry.launch() && !entry.available())
    {
        return Err(Error::InvalidPath);
    }
    Ok(authority)
}

fn validate_path(path: &Path) -> Result<(), Error> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES || bytes.contains(&0) {
        return Err(Error::InvalidPath);
    }
    Ok(())
}

fn open_directory(path: &Path) -> Result<OwnedFd, Error> {
    rustix::fs::open(path, DIRECTORY, Mode::empty()).map_err(|_| Error::Unavailable)
}

fn resolve_launch(primary: &Path, path: &Path) -> Result<NativeWorkspaceSource, Error> {
    let source = primary.join(path);
    validate_path(&source)?;
    let identity = std::fs::canonicalize(&source).map_err(|_| Error::InvalidPath)?;
    validate_path(&identity)?;
    // The authority reopens and verifies this canonical directory. In
    // particular a file, or a vanished root, is not accepted as a launch source.
    let descriptor = open_directory(&identity).map_err(|_| Error::InvalidPath)?;
    drop(descriptor);
    NativeWorkspaceSource::new(identity.clone(), identity, true).map_err(Error::Authority)
}

#[cfg(test)]
mod tests;
