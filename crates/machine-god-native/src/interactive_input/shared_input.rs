//! Shared-stdin acquisition without mutating its open-file description.

use super::{NativeInteractiveInputChunk, NativeInteractiveInputError as Error, Outcome, Shared};
use crate::background_process::TmuxChild as InputChild;
use rustix::fs::{FileType, Mode, OFlags};
use std::fmt;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

mod wire;
pub use wire::run_interactive_input_helper;

/// Exact private CLI dispatch; not ordinary command or provider configuration.
#[doc(hidden)]
pub const INTERACTIVE_INPUT_HELPER_ARGUMENT: &str = "--machine-god-interactive-input-helper";
const MAX_PROGRAM_BYTES: usize = 4096;

/// Explicit executable spelling and retained file authority for the fixed helper.
/// The caller reserves the helper installation against replacement during use.
#[derive(Clone)]
pub struct NativeInteractiveInputHelper {
    program: Arc<PathBuf>,
    executable: Arc<File>,
    #[cfg(test)]
    arguments: Option<Arc<Vec<std::ffi::OsString>>>,
    #[cfg(test)]
    observations: Option<Arc<tests::ChildObservations>>,
}
impl fmt::Debug for NativeInteractiveInputHelper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeInteractiveInputHelper")
            .finish_non_exhaustive()
    }
}
impl NativeInteractiveInputHelper {
    /// Inert binding: validates bounded absolute spelling before copying, but
    /// does not inspect, open, or execute the program. A worker validates the
    /// retained file against that spelling immediately before helper spawn.
    ///
    /// # Errors
    /// Rejects unbounded, nonabsolute, NUL-containing or parent-relative paths.
    pub fn new(program: &Path, executable: File) -> Result<Self, Error> {
        if program.as_os_str().len() > MAX_PROGRAM_BYTES
            || !program.is_absolute()
            || program.as_os_str().as_bytes().contains(&0)
            || program
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        {
            return Err(Error::InvalidDescriptor);
        }
        Ok(Self {
            program: Arc::new(program.to_path_buf()),
            executable: Arc::new(executable),
            #[cfg(test)]
            arguments: None,
            #[cfg(test)]
            observations: None,
        })
    }

    fn command(&self) -> Result<Command, Error> {
        let retained = rustix::fs::fstat(&*self.executable).map_err(|_| Error::Unavailable)?;
        let named = rustix::fs::open(
            &**self.program,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| Error::Unavailable)?;
        let current = rustix::fs::fstat(named).map_err(|_| Error::Unavailable)?;
        if !same_identity(&retained, &current)
            || FileType::from_raw_mode(retained.st_mode) != FileType::RegularFile
            || retained.st_mode & 0o111 == 0
            || retained.st_size != current.st_size
            || retained.st_mtime != current.st_mtime
            || retained.st_mtime_nsec != current.st_mtime_nsec
            || retained.st_ctime != current.st_ctime
            || retained.st_ctime_nsec != current.st_ctime_nsec
        {
            return Err(Error::Unavailable);
        }
        let mut command = Command::new(&**self.program);
        command.env_clear();
        #[cfg(test)]
        if let Some(arguments) = &self.arguments {
            command.args(arguments.iter());
            return Ok(command);
        }
        command.arg(INTERACTIVE_INPUT_HELPER_ARGUMENT);
        Ok(command)
    }
}

pub(super) enum AcquiredInput {
    Direct(File),
    Helper(PipeHelper),
}

pub(super) fn acquire(
    input: File,
    helper: NativeInteractiveInputHelper,
    shared: &Shared,
) -> Result<AcquiredInput, Error> {
    if shared.cancelled() {
        return Err(Error::Cancelled);
    }
    let original = rustix::fs::fstat(&input).map_err(|_| Error::InvalidDescriptor)?;
    let flags = rustix::fs::fcntl_getfl(&input).map_err(|_| Error::InvalidDescriptor)?;
    if flags.contains(OFlags::WRONLY) {
        return Err(Error::InvalidDescriptor);
    }
    match FileType::from_raw_mode(original.st_mode) {
        FileType::CharacterDevice => {
            let settings =
                rustix::termios::tcgetattr(&input).map_err(|_| Error::InvalidDescriptor)?;
            // Reopening a PTY master clone node can allocate a different
            // terminal despite an identical filesystem device identity.
            if rustix::pty::ptsname(&input, Vec::with_capacity(MAX_PROGRAM_BYTES + 1)).is_ok() {
                return Err(Error::InvalidDescriptor);
            }
            // Both supported platforms bound terminal paths by PATH_MAX <=4096.
            // Start with sufficient capacity rather than incrementally growing it.
            let name = rustix::termios::ttyname(&input, Vec::with_capacity(MAX_PROGRAM_BYTES + 1))
                .map_err(|_| Error::Unavailable)?;
            let path = Path::new(std::ffi::OsStr::from_bytes(name.to_bytes()));
            let fresh = reopen_terminal(&input, path, &original, &settings, flags)?;
            Ok(AcquiredInput::Direct(fresh))
        }
        FileType::Fifo if flags.contains(OFlags::NONBLOCK) => Ok(AcquiredInput::Direct(input)),
        FileType::Fifo => PipeHelper::start(input, helper, shared).map(AcquiredInput::Helper),
        _ => Err(Error::InvalidDescriptor),
    }
}

fn same_identity(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && left.st_rdev == right.st_rdev
        && left.st_mode == right.st_mode
}

fn same_settings(left: &rustix::termios::Termios, right: &rustix::termios::Termios) -> bool {
    left.input_modes == right.input_modes && left.output_modes == right.output_modes
        && left.control_modes == right.control_modes && left.local_modes == right.local_modes
        && left.input_speed() == right.input_speed() && left.output_speed() == right.output_speed()
        // SpecialCodes has no equality/iteration API. Its fixed NCCS-sized
        // representation contains only byte-valued settings, never input text.
        && format!("{:?}", left.special_codes) == format!("{:?}", right.special_codes)
}

fn reopen_terminal(
    input: &File,
    path: &Path,
    original: &rustix::fs::Stat,
    settings: &rustix::termios::Termios,
    flags: OFlags,
) -> Result<File, Error> {
    if path.as_os_str().len() > MAX_PROGRAM_BYTES
        || !path.is_absolute()
        || path.starts_with("/dev/fd")
        || path.starts_with("/proc")
        || [
            Path::new("/dev/stdin"),
            Path::new("/dev/stdout"),
            Path::new("/dev/stderr"),
        ]
        .contains(&path)
    {
        return Err(Error::Unavailable);
    }
    let fresh = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|_| Error::Unavailable)?;
    let reopened = rustix::fs::fstat(&fresh).map_err(|_| Error::Unavailable)?;
    let current = rustix::fs::fstat(input).map_err(|_| Error::Unavailable)?;
    if !same_identity(original, &reopened)
        || !same_identity(original, &current)
        || rustix::fs::fcntl_getfl(input).map_err(|_| Error::Unavailable)? != flags
        || !same_settings(
            settings,
            &rustix::termios::tcgetattr(&fresh).map_err(|_| Error::Unavailable)?,
        )
        || !same_settings(
            settings,
            &rustix::termios::tcgetattr(input).map_err(|_| Error::Unavailable)?,
        )
    {
        return Err(Error::Unavailable);
    }
    Ok(fresh.into())
}

pub(super) struct PipeHelper {
    channel: Option<UnixStream>,
    child: InputChild,
}
impl PipeHelper {
    fn start(
        input: File,
        helper: NativeInteractiveInputHelper,
        shared: &Shared,
    ) -> Result<Self, Error> {
        let mut command = helper.command()?;
        if shared.cancelled() {
            return Err(Error::Cancelled);
        }
        let (parent, peer) = UnixStream::pair().map_err(|_| Error::Unavailable)?;
        parent
            .set_nonblocking(true)
            .map_err(|_| Error::Unavailable)?;
        command
            .stdin(Stdio::from(input))
            .stdout(Stdio::null())
            .stderr(Stdio::from(OwnedFd::from(peer)));
        let mut child = InputChild::spawn(&mut command).map_err(|_| Error::Unavailable)?;
        #[cfg(test)]
        if let Some(observations) = &helper.observations {
            observations
                .pid
                .store(child.id(), std::sync::atomic::Ordering::Release);
            if let Some(deferred) = &observations.deferred {
                child.defer_reap_for_test(Arc::clone(deferred));
            }
        }
        child.retain_until_reaped(Box::new(helper));
        drop(command);
        let mut owned = Self {
            channel: Some(parent),
            child,
        };
        if let Err(error) =
            wire::handshake(owned.channel.as_ref().ok_or(Error::Unavailable)?, shared)
        {
            return Err(owned.settle(Err(error)).unwrap_err());
        }
        Ok(owned)
    }

    pub(super) fn drive(mut self, shared: &Shared) -> Outcome {
        let result = self.drive_inner(shared);
        self.settle(result)
    }

    fn drive_inner(&mut self, shared: &Shared) -> Outcome {
        loop {
            shared.wait_for_demand()?;
            let channel = self.channel.as_ref().ok_or(Error::Unavailable)?;
            let Some(chunk) = wire::next_chunk(channel, shared)? else {
                return Ok(());
            };
            shared.publish_chunk(chunk)?;
        }
    }

    fn settle(&mut self, result: Outcome) -> Outcome {
        self.channel.take();
        self.child.abort().map_err(|_| Error::Unavailable)?;
        result
    }
}
impl Drop for PipeHelper {
    fn drop(&mut self) {
        // Close the private channel before the existing exact-child guard
        // kills/reaps or quarantines. Its permit retains this input scope.
        self.channel.take();
    }
}

#[cfg(test)]
mod tests;
