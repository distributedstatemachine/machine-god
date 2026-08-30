use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{FileType, Mode, OFlags};

#[cfg(feature = "ai-gateway-http")]
use crate::{
    CopyFileTool, CreateFolderTool, DeleteFileTool, EditFileTool, FileInfoTool, GlobFilesTool,
    GrepFilesTool, InstallSkillTool, ListFilesTool, OpenFileTool, ReadFileTool, RenameFileTool,
    SemanticSearchTool, SkillTool, WriteFileTool,
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
    pub(crate) install_skill: InstallSkillTool,
    pub(crate) list_files: ListFilesTool,
    pub(crate) open_file: OpenFileTool,
    pub(crate) read_file: ReadFileTool,
    pub(crate) rename_file: RenameFileTool,
    pub(crate) semantic_search: SemanticSearchTool,
    pub(crate) skill: SkillTool,
    pub(crate) terminal_root: OwnedFd,
    pub(crate) vision_root: OwnedFd,
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
        let install_skill_root = clone_descriptor(&self.descriptor)?;
        let list_files_root = clone_descriptor(&self.descriptor)?;
        let open_file_root = clone_descriptor(&self.descriptor)?;
        let read_file_root = clone_descriptor(&self.descriptor)?;
        let rename_file_root = clone_descriptor(&self.descriptor)?;
        let semantic_search_root = clone_descriptor(&self.descriptor)?;
        let skill_root = clone_descriptor(&self.descriptor)?;
        let terminal_root = clone_descriptor(&self.descriptor)?;
        let vision_root = clone_descriptor(&self.descriptor)?;
        let write_file_root = clone_descriptor(&self.descriptor)?;
        Ok(WorkspaceTools {
            copy_file: CopyFileTool::from_root_descriptor(copy_file_root),
            create_folder: CreateFolderTool::from_root_descriptor(create_folder_root),
            delete_file: DeleteFileTool::from_root_descriptor(delete_file_root),
            edit_file: EditFileTool::from_root_descriptor(edit_file_root),
            file_info: FileInfoTool::from_root_descriptor(file_info_root),
            glob_files: GlobFilesTool::from_root_descriptor(self.descriptor),
            grep_files: GrepFilesTool::from_root_descriptor(grep_files_root),
            install_skill: InstallSkillTool::from_root_descriptor(install_skill_root),
            list_files: ListFilesTool::from_root_descriptor(list_files_root),
            open_file: OpenFileTool::from_root_descriptor(open_file_root),
            read_file: ReadFileTool::from_root_descriptor(read_file_root),
            rename_file: RenameFileTool::from_root_descriptor(rename_file_root),
            semantic_search: SemanticSearchTool::from_root_descriptor(semantic_search_root),
            skill: SkillTool::from_root_descriptor(skill_root),
            terminal_root,
            vision_root,
            write_file: WriteFileTool::from_root_descriptor(write_file_root),
        })
    }

    pub(crate) const fn descriptor(&self) -> &OwnedFd {
        &self.descriptor
    }
}

#[cfg(all(test, feature = "ai-gateway-http"))]
mod tests {
    use std::cell::Cell;

    use super::{WorkspaceRoot, WorkspaceRootError};

    #[test]
    fn workspace_composition_uses_exactly_sixteen_identity_preserving_clones() {
        let root = WorkspaceRoot::open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|_| panic!("open workspace root for clone evidence"));
        let original_metadata = rustix::fs::fstat(root.descriptor()).unwrap();
        let original_identity = (
            i128::from(original_metadata.st_dev),
            i128::from(original_metadata.st_ino),
        );
        let mut clone_identities = Vec::new();

        let tools = root
            .into_tools_with_clone(|descriptor| {
                let clone = descriptor.try_clone().map_err(|_| WorkspaceRootError)?;
                let clone_metadata = rustix::fs::fstat(&clone).unwrap();
                clone_identities.push((
                    i128::from(clone_metadata.st_dev),
                    i128::from(clone_metadata.st_ino),
                ));
                Ok(clone)
            })
            .unwrap_or_else(|_| panic!("compose workspace tools for clone evidence"));

        assert_eq!(clone_identities, vec![original_identity; 16]);
        let terminal_metadata = rustix::fs::fstat(&tools.terminal_root).unwrap();
        assert_eq!(
            (
                i128::from(terminal_metadata.st_dev),
                i128::from(terminal_metadata.st_ino),
            ),
            original_identity
        );
        let vision_metadata = rustix::fs::fstat(&tools.vision_root).unwrap();
        assert_eq!(
            (
                i128::from(vision_metadata.st_dev),
                i128::from(vision_metadata.st_ino),
            ),
            original_identity
        );
    }

    #[test]
    fn every_descriptor_clone_failure_aborts_workspace_composition() {
        for failing_attempt in 1..=16 {
            let root = WorkspaceRoot::open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
                .unwrap_or_else(|_| panic!("open workspace root for clone failure evidence"));
            let attempts = Cell::new(0);

            let result = root.into_tools_with_clone(|descriptor| {
                let attempt = attempts.get() + 1;
                attempts.set(attempt);
                if attempt == failing_attempt {
                    return Err(WorkspaceRootError);
                }
                descriptor.try_clone().map_err(|_| WorkspaceRootError)
            });

            assert!(result.is_err());
            assert_eq!(attempts.get(), failing_attempt);
        }
    }
}
