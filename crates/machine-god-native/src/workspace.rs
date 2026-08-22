use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{FileType, Mode, OFlags};

#[cfg(feature = "ai-gateway-http")]
use crate::{ListFilesTool, ReadFileTool};

pub(crate) struct WorkspaceRoot {
    descriptor: OwnedFd,
}

#[derive(Clone, Copy)]
pub(crate) struct WorkspaceRootError;

impl WorkspaceRoot {
    pub(crate) fn open(root: &Path) -> Result<Self, WorkspaceRootError> {
        let lexical_root = root.components().collect::<std::path::PathBuf>();
        if !lexical_root.is_absolute() {
            return Err(WorkspaceRootError);
        }

        let descriptor = rustix::fs::open(
            &lexical_root,
            OFlags::RDONLY
                | OFlags::DIRECTORY
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| WorkspaceRootError)?;
        let metadata = rustix::fs::fstat(&descriptor).map_err(|_| WorkspaceRootError)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
            return Err(WorkspaceRootError);
        }

        Ok(Self { descriptor })
    }

    #[cfg(feature = "ai-gateway-http")]
    pub(crate) fn into_tools(self) -> Result<(ListFilesTool, ReadFileTool), WorkspaceRootError> {
        let list_files_root = self
            .descriptor
            .try_clone()
            .map_err(|_| WorkspaceRootError)?;
        Ok((
            ListFilesTool::from_root_descriptor(list_files_root),
            ReadFileTool::from_root_descriptor(self.descriptor),
        ))
    }

    pub(crate) const fn descriptor(&self) -> &OwnedFd {
        &self.descriptor
    }
}
