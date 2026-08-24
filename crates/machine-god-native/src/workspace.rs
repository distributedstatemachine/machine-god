use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{FileType, Mode, OFlags};

#[cfg(feature = "ai-gateway-http")]
use crate::{
    CopyFileTool, CreateFolderTool, DeleteFileTool, EditFileTool, FileInfoTool, GlobFilesTool,
    GrepFilesTool, ListFilesTool, ReadFileTool, RenameFileTool, WriteFileTool,
};

#[cfg(feature = "ai-gateway-http")]
pub(crate) struct WorkspaceTools {
    pub(crate) copy_file: CopyFileTool,
    pub(crate) create_folder: CreateFolderTool,
    pub(crate) delete_file: DeleteFileTool,
    pub(crate) edit_file: EditFileTool,
    pub(crate) file_info: FileInfoTool,
    pub(crate) glob_files: GlobFilesTool,
    pub(crate) grep_files: GrepFilesTool,
    pub(crate) list_files: ListFilesTool,
    pub(crate) read_file: ReadFileTool,
    pub(crate) rename_file: RenameFileTool,
    pub(crate) write_file: WriteFileTool,
}

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
    pub(crate) fn into_tools(self) -> Result<WorkspaceTools, WorkspaceRootError> {
        self.into_tools_with_clone(|descriptor| {
            descriptor.try_clone().map_err(|_| WorkspaceRootError)
        })
    }

    #[cfg(feature = "ai-gateway-http")]
    pub(crate) fn into_tools_with_clone<CloneDescriptor>(
        self,
        mut clone_descriptor: CloneDescriptor,
    ) -> Result<WorkspaceTools, WorkspaceRootError>
    where
        CloneDescriptor: FnMut(&OwnedFd) -> Result<OwnedFd, WorkspaceRootError>,
    {
        let copy_file_root = clone_descriptor(&self.descriptor)?;
        let create_folder_root = clone_descriptor(&self.descriptor)?;
        let delete_file_root = clone_descriptor(&self.descriptor)?;
        let edit_file_root = clone_descriptor(&self.descriptor)?;
        let file_info_root = clone_descriptor(&self.descriptor)?;
        let grep_files_root = clone_descriptor(&self.descriptor)?;
        let list_files_root = clone_descriptor(&self.descriptor)?;
        let read_file_root = clone_descriptor(&self.descriptor)?;
        let rename_file_root = clone_descriptor(&self.descriptor)?;
        let write_file_root = clone_descriptor(&self.descriptor)?;
        Ok(WorkspaceTools {
            copy_file: CopyFileTool::from_root_descriptor(copy_file_root),
            create_folder: CreateFolderTool::from_root_descriptor(create_folder_root),
            delete_file: DeleteFileTool::from_root_descriptor(delete_file_root),
            edit_file: EditFileTool::from_root_descriptor(edit_file_root),
            file_info: FileInfoTool::from_root_descriptor(file_info_root),
            glob_files: GlobFilesTool::from_root_descriptor(self.descriptor),
            grep_files: GrepFilesTool::from_root_descriptor(grep_files_root),
            list_files: ListFilesTool::from_root_descriptor(list_files_root),
            read_file: ReadFileTool::from_root_descriptor(read_file_root),
            rename_file: RenameFileTool::from_root_descriptor(rename_file_root),
            write_file: WriteFileTool::from_root_descriptor(write_file_root),
        })
    }

    pub(crate) const fn descriptor(&self) -> &OwnedFd {
        &self.descriptor
    }
}
