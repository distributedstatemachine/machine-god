//! One bounded source merger for startup and post-publication reconciliation.

use super::NativeWorkspaceServiceError as Error;
use crate::user_config_store::WorkspaceDirectoryAlias;
use crate::{
    NativeSavedWorkspaceDirectory as Saved, NativeWorkspaceAuthorityError as AuthorityError,
    NativeWorkspaceEntrySpec as Spec, NativeWorkspaceSource as Source,
};
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

/// Resolves only provisional saved sources, retaining their original spelling.
/// Launch sources must already carry canonical existing-directory observations.
/// This is blocking and must execute in the caller's owned startup/control worker.
/// Descriptor, state-exclusion and root-availability validation remains mandatory
/// when the caller constructs or prepares the actual workspace authority.
pub(crate) fn merge_workspace_sources_blocking(
    saved: &[Saved],
    launch: &[Source],
) -> Result<Vec<Spec>, Error> {
    merge_observed_sources_blocking(saved, launch, &[])
}

pub(super) fn merge_observed_sources_blocking(
    saved: &[Saved],
    launch: &[Source],
    aliases: &[WorkspaceDirectoryAlias],
) -> Result<Vec<Spec>, Error> {
    if saved.len() > 16 || launch.len() > 64 {
        return Err(Error::Authority(AuthorityError::TooManyDirectories));
    }
    let mut specs: Vec<Spec> = Vec::with_capacity(saved.len().saturating_add(launch.len()).min(16));
    for record in saved {
        let source = PathBuf::from(OsString::from_vec(record.source_bytes().to_vec()));
        let mut identity = PathBuf::from(OsString::from_vec(record.identity_bytes().to_vec()));
        let mut canonical = record.identity_canonical();
        if let Some(observed) = aliases.iter().find_map(|alias| alias.identity_for(record)) {
            identity = PathBuf::from(OsString::from_vec(observed.to_vec()));
            canonical = true;
        }
        if !canonical {
            match std::fs::canonicalize(&source) {
                Ok(resolved) => {
                    if std::fs::metadata(&resolved)
                        .map_err(|_| Error::Unavailable)?
                        .is_dir()
                    {
                        identity = resolved;
                        canonical = true;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {}
                Err(_) => return Err(Error::Unavailable),
            }
        }
        if specs
            .iter()
            .any(|entry| entry.source().identity() == identity || entry.source().source() == source)
        {
            return Err(Error::Authority(AuthorityError::DuplicateRoot));
        }
        specs.push(
            Spec::new(
                Source::new(source, identity, canonical).map_err(Error::Authority)?,
                true,
                false,
            )
            .map_err(Error::Authority)?
            .with_saved_record(record.clone())
            .map_err(Error::Authority)?,
        );
    }
    for source in launch {
        if !source.identity_canonical() {
            return Err(Error::Authority(AuthorityError::InvalidPath));
        }
        if let Some(entry) = specs
            .iter_mut()
            .find(|entry| entry.source().identity() == source.identity())
        {
            entry.include_launch_source();
        } else {
            if specs.len() == 16 {
                return Err(Error::Authority(AuthorityError::TooManyDirectories));
            }
            specs.push(Spec::new(source.clone(), false, true).map_err(Error::Authority)?);
        }
    }
    Ok(specs)
}
