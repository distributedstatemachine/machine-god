use super::{Admission, ClipboardInner, NativeClipboardError as Error, NativeClipboardExecutable};
use crate::background_process::TmuxChild;
use machine_god_core::CancellationToken;
use rustix::fs::{FileType, Mode, OFlags};
use std::process::{ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

impl NativeClipboardExecutable {
    fn command(&self) -> Result<Command, Error> {
        let retained = rustix::fs::fstat(&*self.retained).map_err(|_| Error::Unavailable)?;
        let named = rustix::fs::open(
            &**self.program,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| Error::Unavailable)?;
        let current = rustix::fs::fstat(named).map_err(|_| Error::Unavailable)?;
        if retained.st_dev != current.st_dev
            || retained.st_ino != current.st_ino
            || retained.st_rdev != current.st_rdev
            || retained.st_mode != current.st_mode
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
        #[cfg(test)]
        if let Some(arguments) = &self.arguments {
            command.args(arguments.iter());
            return Ok(command);
        }
        command.args(platform_arguments());
        Ok(command)
    }
}

fn platform_arguments() -> &'static [&'static str] {
    if cfg!(target_os = "linux") {
        &["-selection", "clipboard"]
    } else {
        &[]
    }
}

pub(super) fn copy(
    inner: &ClipboardInner,
    text: &str,
    cancel: &CancellationToken,
    abandoned: &CancellationToken,
    deadline: Instant,
    admission: Admission,
) -> Result<(), Error> {
    let operation = Operation {
        cancel,
        abandoned,
        deadline,
    };
    operation.check()?;
    let mut command = inner.executable.command()?;
    command
        .env_clear()
        .envs(inner.environment.iter().map(|(k, v)| (k, v)))
        .current_dir(&inner.working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    operation.check()?;
    let mut child = TmuxChild::spawn(&mut command).map_err(|_| Error::Unavailable)?;
    child.retain_until_reaped(Box::new((admission, inner.executable.clone())));
    #[cfg(test)]
    if let Some(probe) = &inner.probe {
        probe.observe(&mut child);
    }
    // Drop the command's owned duplicate descriptors before driving the pipe.
    drop(command);
    let (stdin, stdout, stderr) = child.take_pipes();
    drop((stdout, stderr));
    let stdin = stdin.ok_or(Error::WriteFailed)?;
    let flags = rustix::fs::fcntl_getfl(&stdin).map_err(|_| Error::WriteFailed)?;
    rustix::fs::fcntl_setfl(&stdin, flags | OFlags::NONBLOCK).map_err(|_| Error::WriteFailed)?;
    let result = operation.write(&stdin, text.as_bytes());
    // EOF is required even for empty payloads, and always precedes child cleanup.
    drop(stdin);
    result?;
    loop {
        operation.check()?;
        if let Some(status) = child.try_wait().map_err(|_| Error::Unavailable)? {
            return if status.code() == Some(0) {
                Ok(())
            } else {
                Err(Error::ExitFailed)
            };
        }
        operation.pause();
    }
}

struct Operation<'a> {
    cancel: &'a CancellationToken,
    abandoned: &'a CancellationToken,
    deadline: Instant,
}
impl Operation<'_> {
    fn check(&self) -> Result<(), Error> {
        if self.cancel.is_cancelled() || self.abandoned.is_cancelled() {
            Err(Error::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(Error::TimedOut)
        } else {
            Ok(())
        }
    }
    fn pause(&self) {
        std::thread::sleep(
            Duration::from_millis(2).min(self.deadline.saturating_duration_since(Instant::now())),
        );
    }
    fn write(&self, stdin: &ChildStdin, bytes: &[u8]) -> Result<(), Error> {
        let mut offset = 0;
        while offset < bytes.len() {
            self.check()?;
            let end = bytes.len().min(offset + 4096);
            match rustix::io::write(stdin, &bytes[offset..end]) {
                Ok(0) => return Err(Error::WriteFailed),
                Ok(count) => offset += count,
                Err(rustix::io::Errno::INTR) => {}
                Err(rustix::io::Errno::AGAIN) => self.pause(),
                Err(_) => return Err(Error::WriteFailed),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn fixed_platform_arguments() {
    assert_eq!(
        platform_arguments(),
        if cfg!(target_os = "linux") {
            &["-selection", "clipboard"][..]
        } else {
            &[]
        }
    );
}
