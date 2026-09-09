#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::check_cancelled;
use super::{TerminalTapeRecordingDestination, TerminalTapeRecordingError};
use machine_god_core::CancellationToken;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::fmt::Write;
use std::fs::File;
use std::path::PathBuf;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use rustix::fs::{FileType, Mode, OFlags};

pub(super) fn validate_destination(
    destination: &TerminalTapeRecordingDestination,
) -> Result<(), TerminalTapeRecordingError> {
    match destination {
        TerminalTapeRecordingDestination::Automatic { state_path, .. } => {
            validate_path(state_path, false)
        }
        TerminalTapeRecordingDestination::Explicit(path) => {
            validate_path(path, true)?;
            let name = path
                .file_name()
                .ok_or(TerminalTapeRecordingError::InvalidRequest)?;
            if path
                .as_os_str()
                .as_encoded_bytes()
                .rsplit(|byte| *byte == b'/')
                .next()
                != Some(name.as_encoded_bytes())
            {
                return Err(TerminalTapeRecordingError::InvalidRequest);
            }
            Ok(())
        }
    }
}

pub(super) fn open(
    destination: TerminalTapeRecordingDestination,
    epoch_ms: i64,
    cancellation: &CancellationToken,
) -> Result<(File, PathBuf), TerminalTapeRecordingError> {
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (destination, epoch_ms, cancellation);
        Err(TerminalTapeRecordingError::UnsupportedPlatform)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        check_cancelled(cancellation)?;
        match destination {
            TerminalTapeRecordingDestination::Automatic { store, state_path } => {
                validate_path(&state_path, false)?;
                let root = store
                    .try_clone_root_descriptor()
                    .map_err(|_| TerminalTapeRecordingError::OpenFailed)?;
                check_cancelled(cancellation)?;
                match rustix::fs::mkdirat(&root, "recordings", Mode::RWXU) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(_) => return Err(TerminalTapeRecordingError::OpenFailed),
                }
                check_cancelled(cancellation)?;
                let directory =
                    rustix::fs::openat(&root, "recordings", directory_flags(), Mode::empty())
                        .map_err(|_| TerminalTapeRecordingError::OpenFailed)?;
                check_cancelled(cancellation)?;
                let metadata = rustix::fs::fstat(&directory)
                    .map_err(|_| TerminalTapeRecordingError::OpenFailed)?;
                if metadata.st_uid != rustix::process::geteuid().as_raw()
                    || metadata.st_mode & 0o077 != 0
                    || !FileType::from_raw_mode(metadata.st_mode).is_dir()
                {
                    return Err(TerminalTapeRecordingError::OpenFailed);
                }
                for _ in 0..8 {
                    check_cancelled(cancellation)?;
                    let mut random = [0; 12];
                    getrandom::fill(&mut random)
                        .map_err(|_| TerminalTapeRecordingError::OpenFailed)?;
                    let mut name = format!("machine-god-record-{epoch_ms}-");
                    for byte in random {
                        write!(name, "{byte:02x}").expect("string formatting");
                    }
                    name.push_str(".fxtape");
                    match create(directory.as_fd(), &name, cancellation) {
                        Ok(file) => return Ok((file, state_path.join("recordings").join(name))),
                        Err(TerminalTapeRecordingError::AlreadyExists) => {}
                        Err(error) => return Err(error),
                    }
                }
                Err(TerminalTapeRecordingError::AlreadyExists)
            }
            TerminalTapeRecordingDestination::Explicit(path) => {
                validate_path(&path, true)?;
                let name = path
                    .file_name()
                    .ok_or(TerminalTapeRecordingError::InvalidRequest)?;
                let parent = path
                    .parent()
                    .ok_or(TerminalTapeRecordingError::InvalidRequest)?;
                let directory = open_directory(parent, cancellation)?;
                let file = create(directory.as_fd(), name, cancellation)?;
                Ok((file, path))
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_path(
    path: &std::path::Path,
    allow_parent: bool,
) -> Result<(), TerminalTapeRecordingError> {
    use std::os::unix::ffi::OsStrExt;
    if !path.is_absolute()
        || path.as_os_str().as_bytes().len() > 4096
        || path.as_os_str().as_bytes().contains(&0)
        || !allow_parent
            && path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        Err(TerminalTapeRecordingError::InvalidRequest)
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn directory_flags() -> OFlags {
    OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_directory(
    path: &std::path::Path,
    cancellation: &CancellationToken,
) -> Result<OwnedFd, TerminalTapeRecordingError> {
    check_cancelled(cancellation)?;
    let mut directory = rustix::fs::open("/", directory_flags(), Mode::empty())
        .map_err(|_| TerminalTapeRecordingError::OpenFailed)?;
    check_cancelled(cancellation)?;
    for part in path.components() {
        let name = match part {
            std::path::Component::Normal(name) => name,
            std::path::Component::ParentDir => std::ffi::OsStr::new(".."),
            std::path::Component::RootDir | std::path::Component::CurDir => continue,
            std::path::Component::Prefix(_) => {
                return Err(TerminalTapeRecordingError::InvalidRequest);
            }
        };
        // Do not lexically cancel `directory/..`: the preceding directory must
        // exist and must not be a symlink, even when the next step leaves it.
        directory = rustix::fs::openat(&directory, name, directory_flags(), Mode::empty())
            .map_err(|_| TerminalTapeRecordingError::OpenFailed)?;
        check_cancelled(cancellation)?;
    }
    Ok(directory)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create(
    parent: BorrowedFd<'_>,
    name: impl rustix::path::Arg,
    cancellation: &CancellationToken,
) -> Result<File, TerminalTapeRecordingError> {
    check_cancelled(cancellation)?;
    let fd = rustix::fs::openat(
        parent,
        name,
        OFlags::WRONLY
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|error| {
        if error == rustix::io::Errno::EXIST {
            TerminalTapeRecordingError::AlreadyExists
        } else {
            TerminalTapeRecordingError::OpenFailed
        }
    })?;
    check_cancelled(cancellation)?;
    let metadata = rustix::fs::fstat(&fd).map_err(|_| TerminalTapeRecordingError::OpenFailed)?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file()
        || metadata.st_nlink != 1
        || metadata.st_uid != rustix::process::geteuid().as_raw()
        || metadata.st_mode & 0o077 != 0
    {
        return Err(TerminalTapeRecordingError::OpenFailed);
    }
    // This descriptor was exclusively created by this operation. Normalize its
    // private owner permissions even under a restrictive inherited umask.
    rustix::fs::fchmod(&fd, Mode::RUSR | Mode::WUSR)
        .map_err(|_| TerminalTapeRecordingError::OpenFailed)?;
    check_cancelled(cancellation)?;
    Ok(File::from(fd))
}
