use super::{
    MAX_NATIVE_SKILLS_STARTUP_IO_ATTEMPTS, MAX_NATIVE_SKILLS_STARTUP_PATH_BYTES,
    MAX_NATIVE_SKILLS_STARTUP_PATH_ENTRIES, NativeSkillsStartupError as Error,
};
use crate::skills_catalog::{MAX_NATIVE_SKILL_PATH_BYTES, MAX_NATIVE_SKILL_PATH_COMPONENTS};
use crate::skills_git_runner::{SystemNativeSkillGitRunner, allowed_environment_key};
use crate::skills_roots::NativeSkillDirectoryAuthority;
use crate::{NativeEnvironment, NativeReferenceHostTerminalOptions};
use machine_god_core::CancellationToken;
use rustix::fs::{FileType, Mode, OFlags};
use std::{
    fs::{self, File},
    io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub(super) struct Budget<'a> {
    cancellation: &'a CancellationToken,
    attempts: usize,
}
impl<'a> Budget<'a> {
    pub(super) const fn new(cancellation: &'a CancellationToken) -> Self {
        Self {
            cancellation,
            attempts: 0,
        }
    }
    pub(super) fn check(&self) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
    pub(super) fn call<T>(
        &mut self,
        operation: impl FnOnce() -> io::Result<T>,
    ) -> Result<T, Error> {
        self.check()?;
        if self.attempts == MAX_NATIVE_SKILLS_STARTUP_IO_ATTEMPTS {
            return Err(Error::ResourceLimit);
        }
        self.attempts += 1;
        let result = operation();
        self.check()?;
        result.map_err(|_| Error::Unavailable)
    }
}

pub(super) fn validate_environment(
    environment: &NativeEnvironment,
    terminal: &NativeReferenceHostTerminalOptions,
) -> Result<(), Error> {
    if terminal
        .environment()
        .iter()
        .any(|(name, value)| name == "HOME" && Some(value.as_os_str()) != environment.home())
    {
        return Err(Error::InvalidEnvironment);
    }
    Ok(())
}

fn valid_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|text| !text.contains('\0') && text.len() <= MAX_NATIVE_SKILL_PATH_BYTES)
        && path.components().count() <= MAX_NATIVE_SKILL_PATH_COMPONENTS
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

pub(super) fn home(
    environment: &NativeEnvironment,
    budget: &mut Budget<'_>,
) -> Result<Option<NativeSkillDirectoryAuthority>, Error> {
    let Some(value) = environment.home() else {
        return Ok(None);
    };
    let selected = Path::new(value);
    if !valid_absolute(selected) {
        return Err(Error::InvalidHome);
    }
    let canonical = budget
        .call(|| fs::canonicalize(selected))
        .map_err(home_error)?;
    if !valid_absolute(&canonical) {
        return Err(Error::InvalidHome);
    }
    let directory = budget
        .call(|| {
            rustix::fs::open(
                &canonical,
                OFlags::RDONLY
                    | OFlags::DIRECTORY
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(Into::into)
        })
        .map_err(home_error)?;
    let metadata = budget
        .call(|| rustix::fs::fstat(&directory).map_err(Into::into))
        .map_err(home_error)?;
    let selected_metadata = budget.call(|| fs::metadata(selected)).map_err(home_error)?;
    if !FileType::from_raw_mode(metadata.st_mode).is_dir()
        || !selected_metadata.is_dir()
        || i128::from(metadata.st_dev) != i128::from(selected_metadata.dev())
        || i128::from(metadata.st_ino) != i128::from(selected_metadata.ino())
    {
        return Err(Error::InvalidHome);
    }
    NativeSkillDirectoryAuthority::from_directory(Arc::new(File::from(directory)), canonical)
        .map(Some)
        .map_err(|_| Error::InvalidHome)
}

fn home_error(error: Error) -> Error {
    if error == Error::Unavailable {
        Error::InvalidHome
    } else {
        error
    }
}

pub(super) fn git(
    terminal: &NativeReferenceHostTerminalOptions,
    budget: &mut Budget<'_>,
) -> Result<Option<SystemNativeSkillGitRunner>, Error> {
    let Some(program) = git_program(terminal, budget)? else {
        return Ok(None);
    };
    let environment = terminal
        .environment()
        .iter()
        .filter(|(name, _)| allowed_environment_key(name))
        .cloned()
        .collect();
    let runner = SystemNativeSkillGitRunner::new(
        program,
        terminal.helper_program().to_owned(),
        vec![crate::TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
        environment,
    )
    .map_err(|_| Error::InvalidEnvironment)?;
    #[cfg(target_os = "macos")]
    let runner = runner
        .with_process_inventory_service(
            terminal.helper_program().to_owned(),
            vec![crate::PROCESS_INVENTORY_SERVICE_ARGUMENT.into()],
        )
        .map_err(|_| Error::InvalidEnvironment)?;
    Ok(Some(runner))
}

fn git_program(
    terminal: &NativeReferenceHostTerminalOptions,
    budget: &mut Budget<'_>,
) -> Result<Option<PathBuf>, Error> {
    let Some((_, path)) = terminal
        .environment()
        .iter()
        .find(|(name, _)| name == "PATH")
    else {
        return Ok(None);
    };
    if path.len() > MAX_NATIVE_SKILLS_STARTUP_PATH_BYTES {
        return Err(Error::ResourceLimit);
    }
    // Validate the entire selection before the first lookup, including entries
    // after a potential match. Empty/relative entries never imply ambient cwd.
    let mut entries = Vec::new();
    for directory in std::env::split_paths(path) {
        if entries.len() == MAX_NATIVE_SKILLS_STARTUP_PATH_ENTRIES {
            return Err(Error::ResourceLimit);
        }
        let candidate = directory.join("git");
        if !valid_absolute(&directory) || !valid_absolute(&candidate) {
            return Err(Error::InvalidPath);
        }
        entries.push(candidate);
    }
    for candidate in entries {
        let metadata = budget.call(|| match fs::metadata(&candidate) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::NotADirectory
                        | io::ErrorKind::PermissionDenied
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        })?;
        let Some(metadata) = metadata else {
            continue;
        };
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }
        let canonical = budget.call(|| fs::canonicalize(&candidate))?;
        if !valid_absolute(&canonical) {
            return Err(Error::InvalidPath);
        }
        let confirmed = budget.call(|| fs::metadata(&canonical))?;
        if metadata.dev() != confirmed.dev()
            || metadata.ino() != confirmed.ino()
            || !confirmed.is_file()
            || confirmed.permissions().mode() & 0o111 == 0
        {
            return Err(Error::Unavailable);
        }
        return Ok(Some(canonical));
    }
    Ok(None)
}
