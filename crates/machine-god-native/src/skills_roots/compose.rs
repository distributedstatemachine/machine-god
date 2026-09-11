use super::{
    NativeSkillCatalog, NativeSkillDirectoryAuthority, NativeSkillLinkPolicy, NativeSkillRoot,
    NativeSkillRootsError as Error, NativeSkillSource as Source, Result, check,
};
use machine_god_core::CancellationToken;
use rustix::{
    fs::{AtFlags, FileType, Mode, OFlags, Stat},
    io::Errno,
};
use std::{fs::File, path::Path, sync::Arc};

const WORKSPACE: [(Source, &str); 6] = [
    (Source::WorkspaceShared, "skills"),
    (Source::WorkspaceOpencode, ".opencode/skills"),
    (Source::WorkspaceCodex, ".codex/skills"),
    (Source::WorkspaceClaude, ".claude/skills"),
    (Source::WorkspaceAgents, ".agents/skills"),
    (Source::WorkspaceClaw, ".claw/skills"),
];
const HOME: [(Source, &str); 6] = [
    (Source::GlobalFx, ".fx/skills"),
    (Source::GlobalOpencode, ".config/opencode/skills"),
    (Source::GlobalCodex, ".codex/skills"),
    (Source::GlobalClaude, ".claude/skills"),
    (Source::GlobalAgents, ".agents/skills"),
    (Source::GlobalClaw, ".claw/skills"),
];
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

#[derive(Default)]
struct Budget {
    attempts: usize,
}
impl Budget {
    fn call<T>(
        &mut self,
        cancellation: &CancellationToken,
        operation: impl FnOnce() -> std::result::Result<T, Errno>,
    ) -> Result<T> {
        check(cancellation)?;
        if self.attempts == super::MAX_NATIVE_SKILL_ROOT_IO_ATTEMPTS {
            return Err(Error::ResourceLimit);
        }
        self.attempts += 1;
        let result = operation();
        check(cancellation)?;
        result.map_err(|_| Error::Unavailable)
    }
    fn directory(&mut self, file: &File, cancellation: &CancellationToken) -> Result<[i128; 2]> {
        let observed = self.call(cancellation, || rustix::fs::fstat(file))?;
        if !FileType::from_raw_mode(observed.st_mode).is_dir() {
            return Err(Error::InvalidAuthority);
        }
        Ok(identity(&observed))
    }
}

pub(super) fn compose(
    workspace: Option<&NativeSkillDirectoryAuthority>,
    home: Option<&NativeSkillDirectoryAuthority>,
    managed: Option<NativeSkillRoot>,
    cancellation: &CancellationToken,
) -> Result<NativeSkillCatalog> {
    let mut budget = Budget::default();
    let home_identity = home
        .map(|home| budget.directory(&home.directory, cancellation))
        .transpose()?;
    let mut roots = Vec::new();
    if let Some(workspace) = workspace {
        workspace_roots(
            workspace,
            home,
            home_identity,
            &mut roots,
            &mut budget,
            cancellation,
        )?;
    }
    if let Some(managed) = managed {
        roots.push(managed);
    }
    if let Some(home) = home {
        append(&mut roots, &home.directory, &home.path, &HOME)?;
    }
    check(cancellation)?;
    NativeSkillCatalog::new(roots).map_err(Error::Catalog)
}

fn workspace_roots(
    workspace: &NativeSkillDirectoryAuthority,
    home: Option<&NativeSkillDirectoryAuthority>,
    home_identity: Option<[i128; 2]>,
    roots: &mut Vec<NativeSkillRoot>,
    budget: &mut Budget,
    cancellation: &CancellationToken,
) -> Result<()> {
    let mut directory = workspace.directory.clone();
    let mut path = workspace.path.clone();
    let mut seen = Vec::new();
    loop {
        check(cancellation)?;
        let current = budget.directory(&directory, cancellation)?;
        if Some(current) == home_identity {
            return Ok(());
        }
        if home.is_some_and(|home| path == home.path) {
            // Never climb through the captured home spelling into higher
            // directories when its observed identity no longer agrees.
            return Err(Error::ChangedAncestry);
        }
        if seen.len() == super::MAX_NATIVE_SKILL_WORKSPACE_LEVELS {
            return Err(Error::ResourceLimit);
        }
        if seen.contains(&current) {
            return Err(Error::ChangedAncestry);
        }
        seen.push(current);
        append(roots, &directory, &path, &WORKSPACE)?;

        let parent = budget.call(cancellation, || {
            rustix::fs::openat(&*directory, "..", DIRECTORY_FLAGS, Mode::empty())
        })?;
        let parent = Arc::new(File::from(parent));
        let parent_identity = budget.directory(&parent, cancellation)?;
        let Some(parent_path) = path.parent() else {
            return if parent_identity == current {
                Ok(())
            } else {
                Err(Error::ChangedAncestry)
            };
        };
        if parent_identity == current {
            return Err(Error::ChangedAncestry);
        }
        let name = path.file_name().ok_or(Error::ChangedAncestry)?;
        let child = budget
            .call(cancellation, || {
                rustix::fs::statat(&*parent, name, AtFlags::SYMLINK_NOFOLLOW)
            })
            .map_err(|error| {
                if error == Error::Unavailable {
                    Error::ChangedAncestry
                } else {
                    error
                }
            })?;
        let parent_again = budget.call(cancellation, || {
            rustix::fs::statat(&*directory, "..", AtFlags::SYMLINK_NOFOLLOW)
        })?;
        if identity(&child) != current || identity(&parent_again) != parent_identity {
            return Err(Error::ChangedAncestry);
        }
        path = parent_path.to_owned();
        directory = parent;
    }
}

fn append(
    roots: &mut Vec<NativeSkillRoot>,
    directory: &Arc<File>,
    path: &Path,
    specs: &[(Source, &str)],
) -> Result<()> {
    for (source, relative) in specs {
        roots.push(
            NativeSkillRoot::from_directory(
                directory.clone(),
                relative.into(),
                path.to_owned(),
                path.join(relative),
                *source,
                NativeSkillLinkPolicy::Contained,
            )
            .map_err(Error::Catalog)?,
        );
    }
    Ok(())
}

fn identity(observed: &Stat) -> [i128; 2] {
    [i128::from(observed.st_dev), i128::from(observed.st_ino)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metering_and_cancellation_bracket_calls() {
        let cancellation = CancellationToken::new();
        let mut budget = Budget::default();
        assert_eq!(
            budget.call(&cancellation, || {
                cancellation.cancel();
                Ok(())
            }),
            Err(Error::Cancelled)
        );
        assert_eq!(budget.attempts, 1);
        assert_eq!(
            budget.call(&cancellation, || panic!("cancelled work")),
            Err::<(), _>(Error::Cancelled)
        );
        let mut budget = Budget {
            attempts: super::super::MAX_NATIVE_SKILL_ROOT_IO_ATTEMPTS,
        };
        assert_eq!(
            budget.call(&CancellationToken::new(), || panic!("excess work")),
            Err::<(), _>(Error::ResourceLimit)
        );
    }
}
