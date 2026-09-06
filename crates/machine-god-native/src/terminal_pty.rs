//! Owned, nonblocking native pseudoterminal transport.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::ffi::OsString;
use std::fmt;
#[cfg(test)]
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
#[cfg(test)]
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use machine_god_core::CancellationToken;
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{Mode, OFlags};

use crate::background_input::{
    BackgroundInputReceipt, BackgroundInputStatus, MAX_BACKGROUND_INPUT_BYTES,
};
use crate::background_process::{
    BackgroundProcessExit, BackgroundProcessSignal, OwnedBackgroundProcess, TerminalChildGuard,
    TerminalClosePhase, ValidatedBackgroundEnvironment,
};

#[cfg(test)]
use crate::background_process::{
    MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES, MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES,
};
use crate::terminal_helper::{
    COMMIT, DescriptorIo, LaunchFrame, MAX_STARTUP_TIMEOUT, PTY_DEADLINE_ENV, READY,
    TerminalHelperError, TerminalHelperErrorKind, check_deadline, encode_helper_deadline,
    read_gate, validate_program_arguments, validate_pty_directory as validate_directory,
    write_gate,
};
#[cfg(test)]
use crate::terminal_helper::{
    MAX_ARGUMENT_BYTES, MAX_ARGUMENTS, MAX_ARGUMENTS_BYTES, MAX_FRAME, MAX_PROGRAM_BYTES,
    START_TIMEOUT, read_frame, run_terminal_pty_helper,
};
pub(crate) use crate::terminal_helper::{TerminalPtyDimensions, TerminalPtyHelper};

const MAX_READ: usize = 64 * 1024;
// A slave can close just before its retained child's exit becomes waitable.
// Allow observation to converge without sleeping or treating EOF as an exit.
const EOF_STATUS_GRACE: Duration = Duration::from_millis(100);
static LIVE_PTYS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct PtyPermit;
impl PtyPermit {
    fn acquire() -> Result<Self, TerminalPtyError> {
        use std::sync::atomic::Ordering;
        LIVE_PTYS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 16).then_some(count + 1)
            })
            .map_err(|_| error(TerminalPtyErrorKind::Capacity))?;
        Ok(Self)
    }
}
impl Drop for PtyPermit {
    fn drop(&mut self) {
        LIVE_PTYS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalPtyErrorKind {
    InvalidRequest,
    Cancelled,
    Timeout,
    Process,
    Closed,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalPtyError {
    kind: TerminalPtyErrorKind,
}
impl TerminalPtyError {
    pub(crate) const fn kind(self) -> TerminalPtyErrorKind {
        self.kind
    }
}
impl fmt::Display for TerminalPtyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal pseudoterminal operation failed")
    }
}
impl std::error::Error for TerminalPtyError {}
impl From<TerminalHelperError> for TerminalPtyError {
    fn from(failure: TerminalHelperError) -> Self {
        error(match failure.kind {
            TerminalHelperErrorKind::InvalidRequest => TerminalPtyErrorKind::InvalidRequest,
            TerminalHelperErrorKind::Cancelled => TerminalPtyErrorKind::Cancelled,
            TerminalHelperErrorKind::Timeout => TerminalPtyErrorKind::Timeout,
            TerminalHelperErrorKind::Process | TerminalHelperErrorKind::Protocol => {
                TerminalPtyErrorKind::Process
            }
        })
    }
}
const fn error(kind: TerminalPtyErrorKind) -> TerminalPtyError {
    TerminalPtyError { kind }
}
fn process_error(_: impl fmt::Debug) -> TerminalPtyError {
    error(TerminalPtyErrorKind::Process)
}

pub(crate) struct TerminalPtyRequest {
    program: String,
    arguments: Vec<String>,
    environment: ValidatedBackgroundEnvironment,
    cwd: OwnedFd,
    dimensions: TerminalPtyDimensions,
    startup_source: Option<String>,
}
impl TerminalPtyRequest {
    pub(crate) fn new(
        program: String,
        arguments: Vec<String>,
        environment: Vec<(OsString, OsString)>,
        cwd: OwnedFd,
        dimensions: TerminalPtyDimensions,
    ) -> Result<Self, TerminalPtyError> {
        validate_program_arguments(&program, &arguments)?;
        validate_directory(&cwd)?;
        let environment = ValidatedBackgroundEnvironment::new(environment)
            .map_err(|_| error(TerminalPtyErrorKind::InvalidRequest))?;
        Ok(Self {
            program,
            arguments,
            environment,
            cwd,
            dimensions: dimensions.validate()?,
            startup_source: None,
        })
    }
    pub(crate) fn with_startup_source(mut self, source: String) -> Result<Self, TerminalPtyError> {
        // Each physical line remains below both canonical input-line ceilings.
        // The host owns the bounded suffix until the committed shell can read it.
        if source.is_empty()
            || source.len() > 32 * 1024
            || source.contains('\0')
            || !source.ends_with('\n')
            || source.contains('\r')
            || source.split_inclusive('\n').any(|line| line.len() > 512)
        {
            return Err(error(TerminalPtyErrorKind::InvalidRequest));
        }
        self.startup_source = Some(source);
        Ok(self)
    }
    pub(crate) fn frame(&self) -> Result<Vec<u8>, TerminalPtyError> {
        Ok(LaunchFrame::encode(
            &self.program,
            &self.arguments,
            &self.environment,
            self.dimensions,
        )?)
    }
}

fn encode_pty_deadline(deadline: Instant) -> Result<String, TerminalPtyError> {
    Ok(encode_helper_deadline(deadline, MAX_STARTUP_TIMEOUT)?)
}

pub(crate) struct PreparedTerminalPty {
    startup_source: Option<crate::terminal_helper::TerminalStartupInput>,
    deadline: Instant,
    process: Option<OwnedBackgroundProcess>,
    master: Option<OwnedFd>,
    gate: Option<UnixStream>,
    permit: Option<PtyPermit>,
}

impl PreparedTerminalPty {
    #[cfg(test)]
    pub(crate) fn prepare(
        helper: &TerminalPtyHelper,
        request: TerminalPtyRequest,
        cancellation: &CancellationToken,
    ) -> Result<Self, TerminalPtyError> {
        Self::prepare_until(
            helper,
            request,
            Instant::now() + START_TIMEOUT,
            cancellation,
        )
    }

    pub(crate) fn prepare_until(
        helper: &TerminalPtyHelper,
        request: TerminalPtyRequest,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self, TerminalPtyError> {
        check_deadline(deadline, cancellation)?;
        let helper_deadline = encode_pty_deadline(deadline)?;
        let permit = PtyPermit::acquire()?;
        let frame = request.frame()?;
        #[cfg(target_os = "macos")]
        machine_god_terminal_sys::ProcessIdentity::verify_signal_support()
            .map_err(process_error)?;
        let (master, slave) = open_pty(request.dimensions)?;
        if request.startup_source.is_some() {
            set_echo(&slave, false)?;
        }
        let (mut gate, child_gate) = UnixStream::pair().map_err(process_error)?;
        gate.set_nonblocking(true).map_err(process_error)?;
        let mut guard = TerminalChildGuard::reserve(cancellation).map_err(process_error)?;
        let mut command = Command::new(helper.program());
        command
            .args(helper.arguments())
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("MACHINE_GOD_PTY_HELPER", "1")
            .env(PTY_DEADLINE_ENV, helper_deadline)
            .stdin(Stdio::from(OwnedFd::from(child_gate)))
            .stdout(Stdio::from(slave))
            .stderr(Stdio::from(request.cwd));
        guard.spawn(&mut command).map_err(process_error)?;
        write_gate(&mut gate, &frame, deadline, cancellation)?;
        let mut ready = [0];
        read_gate(&mut gate, &mut ready, deadline, cancellation)?;
        if ready != [READY] {
            return Err(error(TerminalPtyErrorKind::Process));
        }
        let startup_source = request
            .startup_source
            .as_deref()
            .map(crate::terminal_helper::TerminalStartupInput::new)
            .transpose()?;
        if let Some(source) = &startup_source {
            write_gate(
                &mut DescriptorIo(master.as_fd()),
                source.initial(),
                deadline,
                cancellation,
            )?;
        }
        let process = guard.into_session().map_err(process_error)?;
        check_deadline(deadline, cancellation)?;
        Ok(Self {
            startup_source: startup_source.filter(|source| !source.complete()),
            deadline,
            process: Some(process),
            master: Some(master),
            gate: Some(gate),
            permit: Some(permit),
        })
    }

    pub(crate) fn commit(
        mut self,
        cancellation: &CancellationToken,
    ) -> Result<TerminalPty, TerminalPtyError> {
        let gate = self
            .gate
            .as_mut()
            .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?;
        write_gate(gate, &[COMMIT], self.deadline, cancellation)?;
        // Cancellation cannot relabel or discard the committed process.
        let mut process = self
            .process
            .take()
            .ok_or_else(|| error(TerminalPtyErrorKind::Process))?;
        process
            .activate_signal_controller()
            .map_err(process_error)?;
        let master = self
            .master
            .take()
            .ok_or_else(|| error(TerminalPtyErrorKind::Process))?;
        Ok(TerminalPty {
            startup_source: std::mem::take(&mut self.startup_source),
            startup_deadline: self.deadline,
            #[cfg(test)]
            pid: process.pid(),
            process: Some(process),
            master: Some(master),
            observed: None,
            read_closed: false,
            eof_deadline: None,
            write_closed: false,
            output_incomplete: false,
            permit: self.permit.take(),
        })
    }
}

impl Drop for PreparedTerminalPty {
    fn drop(&mut self) {
        // Abort the private protocol before the helper owner performs cleanup.
        drop(self.gate.take());
        drop(self.master.take());
        drop(self.process.take());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalPtyStatus {
    Running,
    Exited(i32),
    Signalled(i32),
}
impl From<BackgroundProcessExit> for TerminalPtyStatus {
    fn from(status: BackgroundProcessExit) -> Self {
        match status {
            BackgroundProcessExit::Exited(code) => Self::Exited(code),
            BackgroundProcessExit::Signaled(signal) => Self::Signalled(signal),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalPtyRead {
    pub(crate) bytes_read: usize,
    pub(crate) closed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalPtyClose {
    pub(crate) status: TerminalPtyStatus,
    pub(crate) output_incomplete: bool,
}

pub(crate) struct TerminalPty {
    startup_source: Option<crate::terminal_helper::TerminalStartupInput>,
    startup_deadline: Instant,
    #[cfg(test)]
    pid: NonZeroU32,
    process: Option<OwnedBackgroundProcess>,
    master: Option<OwnedFd>,
    observed: Option<TerminalPtyStatus>,
    read_closed: bool,
    eof_deadline: Option<Instant>,
    write_closed: bool,
    output_incomplete: bool,
    permit: Option<PtyPermit>,
}
impl TerminalPty {
    fn flush_startup_source(&mut self) -> Result<(), TerminalPtyError> {
        let Some(source) = self.startup_source.as_mut() else {
            return Ok(());
        };
        if Instant::now() >= self.startup_deadline {
            return Err(error(TerminalPtyErrorKind::Timeout));
        }
        let master = self
            .master
            .as_ref()
            .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?;
        let Some(bytes) = source.pending() else {
            return Ok(());
        };
        match rustix::io::write(master, bytes) {
            Ok(count) => source.advance(count)?,
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => {}
            Err(failure) => return Err(process_error(failure)),
        }
        if source.complete() {
            self.startup_source = None;
        }
        Ok(())
    }
    pub(crate) fn restore_startup_echo(&self) -> Result<(), TerminalPtyError> {
        set_echo(
            self.master
                .as_ref()
                .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?,
            true,
        )
    }
    #[cfg(test)]
    pub(crate) const fn pid(&self) -> NonZeroU32 {
        self.pid
    }
    pub(crate) fn read(&mut self, buffer: &mut [u8]) -> Result<TerminalPtyRead, TerminalPtyError> {
        if buffer.is_empty() || buffer.len() > MAX_READ {
            return Err(error(TerminalPtyErrorKind::InvalidRequest));
        }
        self.flush_startup_source()?;
        if self.read_closed {
            return self.observe_eof();
        }
        let Some(master) = self.master.as_ref() else {
            return Err(error(TerminalPtyErrorKind::Closed));
        };
        for _ in 0..32 {
            match rustix::io::read(master, &mut *buffer) {
                Ok(0) | Err(rustix::io::Errno::IO) => {
                    self.read_closed = true;
                    self.write_closed = true;
                    self.eof_deadline = Some(Instant::now() + EOF_STATUS_GRACE);
                    return self.observe_eof();
                }
                Ok(bytes_read) => {
                    if let Some(source) = self.startup_source.as_mut() {
                        source.observe(&buffer[..bytes_read]);
                    }
                    return Ok(TerminalPtyRead {
                        bytes_read,
                        closed: false,
                    });
                }
                Err(rustix::io::Errno::INTR) => {}
                Err(rustix::io::Errno::AGAIN) => {
                    return Ok(TerminalPtyRead {
                        bytes_read: 0,
                        closed: false,
                    });
                }
                Err(other) => return Err(process_error(other)),
            }
        }
        Ok(TerminalPtyRead {
            bytes_read: 0,
            closed: false,
        })
    }
    fn observe_eof(&mut self) -> Result<TerminalPtyRead, TerminalPtyError> {
        // Native close temporarily moves the process capability into its
        // teardown routine. Its drain needs physical EOF, not another status
        // lookup through the now-empty slot.
        if self.process.is_none() || self.eof_deadline.is_none() {
            return Ok(TerminalPtyRead {
                bytes_read: 0,
                closed: true,
            });
        }
        let status = self.status()?;
        Ok(eof_read(status, self.eof_deadline, Instant::now()))
    }
    pub(crate) fn write(
        &mut self,
        bytes: &[u8],
    ) -> Result<BackgroundInputReceipt, TerminalPtyError> {
        if bytes.is_empty() || bytes.len() > MAX_BACKGROUND_INPUT_BYTES {
            return Err(error(TerminalPtyErrorKind::InvalidRequest));
        }
        self.flush_startup_source()?;
        if self.startup_source.is_some() {
            return Ok(BackgroundInputReceipt::new(
                0,
                false,
                BackgroundInputStatus::Backpressure,
            ));
        }
        if self.write_closed {
            return Ok(BackgroundInputReceipt::new(
                0,
                true,
                BackgroundInputStatus::Closed,
            ));
        }
        let master = self
            .master
            .as_ref()
            .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?;
        let mut accepted = 0;
        for _ in 0..32 {
            if accepted == bytes.len() {
                break;
            }
            match rustix::io::write(master, &bytes[accepted..]) {
                Ok(0) | Err(rustix::io::Errno::AGAIN) => break,
                Ok(count) => accepted += count,
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => {
                    self.write_closed = true;
                    return Ok(BackgroundInputReceipt::new(
                        accepted,
                        true,
                        if matches!(error, rustix::io::Errno::PIPE | rustix::io::Errno::IO) {
                            BackgroundInputStatus::Closed
                        } else {
                            BackgroundInputStatus::Failed
                        },
                    ));
                }
            }
        }
        Ok(BackgroundInputReceipt::new(
            accepted,
            false,
            if accepted == bytes.len() {
                BackgroundInputStatus::Written
            } else {
                BackgroundInputStatus::Backpressure
            },
        ))
    }
    pub(crate) fn resize(
        &mut self,
        dimensions: TerminalPtyDimensions,
    ) -> Result<(), TerminalPtyError> {
        let dimensions = dimensions.validate()?;
        let master = self
            .master
            .as_ref()
            .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?;
        rustix::termios::tcsetwinsize(master, dimensions.winsize()).map_err(process_error)
    }
    pub(crate) fn status(&mut self) -> Result<TerminalPtyStatus, TerminalPtyError> {
        if let Some(status) = self.observed {
            return Ok(status);
        }
        let process = self
            .process
            .as_mut()
            .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?;
        if let Some(status) = process.terminal_poll().map_err(process_error)? {
            let status = status.into();
            self.observed = Some(status);
            Ok(status)
        } else {
            Ok(TerminalPtyStatus::Running)
        }
    }
    pub(crate) fn signal(
        &mut self,
        signal: BackgroundProcessSignal,
    ) -> Result<(), TerminalPtyError> {
        #[cfg(target_os = "macos")]
        {
            if self.status()? != TerminalPtyStatus::Running {
                return Err(error(TerminalPtyErrorKind::Closed));
            }
            self.signal_foreground(signal)
        }
        #[cfg(target_os = "linux")]
        self.process
            .as_ref()
            .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?
            .terminal_signal(signal)
            .map_err(process_error)
    }

    #[cfg(target_os = "macos")]
    fn signal_foreground(
        &mut self,
        signal: BackgroundProcessSignal,
    ) -> Result<(), TerminalPtyError> {
        let signal = match signal {
            BackgroundProcessSignal::Interrupt => rustix::process::Signal::INT,
            BackgroundProcessSignal::Terminate => rustix::process::Signal::TERM,
            BackgroundProcessSignal::Kill => rustix::process::Signal::KILL,
            BackgroundProcessSignal::Hangup => rustix::process::Signal::HUP,
            BackgroundProcessSignal::Quit => rustix::process::Signal::QUIT,
        };
        let master = self
            .master
            .as_ref()
            .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?;
        // TIOCSIG can flush the tty's unread output even if a later read sees
        // EOF. Conservatively retain that evidence gap through explicit signals
        // and close rather than claiming a complete raw tail.
        self.output_incomplete = true;
        machine_god_terminal_sys::signal_terminal_foreground(master.as_fd(), signal)
            .map_err(process_error)
    }
    pub(crate) fn close(&mut self, force: bool) -> Result<TerminalPtyStatus, TerminalPtyError> {
        self.close_with_output(force, |_| {})
            .map(|closed| closed.status)
    }

    pub(crate) fn close_with_output(
        &mut self,
        force: bool,
        mut output: impl FnMut(&[u8]),
    ) -> Result<TerminalPtyClose, TerminalPtyError> {
        self.startup_source = None;
        if self.process.is_none() {
            return self
                .observed
                .map(|status| TerminalPtyClose {
                    status,
                    output_incomplete: self.output_incomplete,
                })
                .ok_or_else(|| error(TerminalPtyErrorKind::Closed));
        }
        self.write_closed = true;
        let before = self.status()?;
        let mut drain_budget = 128;
        let mut phase_failed = self
            .drain_close_output(&mut drain_budget, &mut output)
            .is_err();
        let mut process = self
            .process
            .take()
            .ok_or_else(|| error(TerminalPtyErrorKind::Closed))?;
        let closed = process.terminal_close(force, |phase| match phase {
            TerminalClosePhase::Graceful | TerminalClosePhase::Force => {
                #[cfg(target_os = "macos")]
                {
                    let signal = if matches!(phase, TerminalClosePhase::Graceful) {
                        BackgroundProcessSignal::Terminate
                    } else {
                        BackgroundProcessSignal::Kill
                    };
                    // An exited session leader can still have a foreground
                    // job. The owned master, not a persisted PID, authorizes
                    // this delivery; the kernel resolves membership atomically.
                    if self.signal_foreground(signal).is_err()
                        && before == TerminalPtyStatus::Running
                    {
                        phase_failed = true;
                    }
                }
            }
            TerminalClosePhase::Close => {
                phase_failed |= self
                    .drain_close_output(&mut drain_budget, &mut output)
                    .is_err();
                self.output_incomplete |= !self.read_closed;
                drop(self.master.take());
                self.read_closed = true;
            }
        });
        // Even an early lifecycle error must close the master before Drop's
        // fallback reap. Retain the child on failure so cleanup can be retried.
        drop(self.master.take());
        self.read_closed = true;
        let closed = match closed {
            Ok(closed) => closed,
            Err(error) => {
                self.output_incomplete = true;
                self.process = Some(process);
                return Err(process_error(error));
            }
        };
        drop(process);
        let status = if before == TerminalPtyStatus::Running {
            closed.into()
        } else {
            before
        };
        self.observed = Some(status);
        drop(self.permit.take());
        if phase_failed {
            return Err(error(TerminalPtyErrorKind::Process));
        }
        Ok(TerminalPtyClose {
            status,
            output_incomplete: self.output_incomplete,
        })
    }

    fn drain_close_output(
        &mut self,
        budget: &mut usize,
        output: &mut impl FnMut(&[u8]),
    ) -> Result<(), TerminalPtyError> {
        let mut buffer = [0; 4096];
        while *budget != 0 {
            *budget -= 1;
            let read = self.read(&mut buffer)?;
            if read.bytes_read != 0 {
                output(&buffer[..read.bytes_read]);
            }
            if read.closed || read.bytes_read == 0 {
                break;
            }
        }
        Ok(())
    }
}

fn eof_read(status: TerminalPtyStatus, deadline: Option<Instant>, now: Instant) -> TerminalPtyRead {
    TerminalPtyRead {
        bytes_read: 0,
        closed: status != TerminalPtyStatus::Running || deadline.is_none_or(|end| now >= end),
    }
}
impl Drop for TerminalPty {
    fn drop(&mut self) {
        let _ = self.close(true);
        drop(self.master.take());
        drop(self.process.take());
    }
}

fn set_echo(fd: &impl AsFd, enabled: bool) -> Result<(), TerminalPtyError> {
    let mut termios = rustix::termios::tcgetattr(fd).map_err(process_error)?;
    termios
        .local_modes
        .set(rustix::termios::LocalModes::ECHO, enabled);
    rustix::termios::tcsetattr(fd, rustix::termios::OptionalActions::Now, &termios)
        .map_err(process_error)
}

fn open_pty(dimensions: TerminalPtyDimensions) -> Result<(OwnedFd, OwnedFd), TerminalPtyError> {
    // Atomic CLOEXEC is required even on macOS, where posix_openpt's public
    // flag set lacks it; open the same native clone device with open(2).
    let master = rustix::fs::open(
        "/dev/ptmx",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(process_error)?;
    rustix::pty::grantpt(&master).map_err(process_error)?;
    rustix::pty::unlockpt(&master).map_err(process_error)?;
    #[cfg(target_os = "linux")]
    let slave = rustix::pty::ioctl_tiocgptpeer(
        &master,
        rustix::pty::OpenptFlags::RDWR
            | rustix::pty::OpenptFlags::NOCTTY
            | rustix::pty::OpenptFlags::CLOEXEC,
    )
    .map_err(process_error)?;
    #[cfg(target_os = "macos")]
    let slave = {
        let name = rustix::pty::ptsname(&master, Vec::new()).map_err(process_error)?;
        rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(process_error)?
    };
    rustix::termios::tcsetwinsize(&slave, dimensions.winsize()).map_err(process_error)?;
    Ok((master, slave))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    #[test]
    fn eof_waits_only_for_the_original_bounded_exit_observation_window() {
        let first = Instant::now();
        let deadline = first + EOF_STATUS_GRACE;
        for elapsed in [0, 1, 50, 99] {
            let read = eof_read(
                TerminalPtyStatus::Running,
                Some(deadline),
                first + Duration::from_millis(elapsed),
            );
            assert_eq!(read.bytes_read, 0);
            assert!(!read.closed);
        }
        for elapsed in [100, 101, 1000] {
            assert!(
                eof_read(
                    TerminalPtyStatus::Running,
                    Some(deadline),
                    first + Duration::from_millis(elapsed),
                )
                .closed
            );
        }
        for status in [
            TerminalPtyStatus::Exited(7),
            TerminalPtyStatus::Signalled(15),
        ] {
            assert!(eof_read(status, Some(deadline), first).closed);
        }
        assert!(eof_read(TerminalPtyStatus::Running, None, first).closed);
    }
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "machine-god-pty-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn fd(&self) -> OwnedFd {
            rustix::fs::open(
                &self.0,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .unwrap()
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn helper() -> TerminalPtyHelper {
        #[cfg(target_os = "linux")]
        {
            static SUBREAPER: std::sync::Once = std::sync::Once::new();
            SUBREAPER.call_once(|| {
                rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1)).unwrap()
            });
        }
        if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            let program = PathBuf::from(program);
            assert!(
                program.is_absolute(),
                "release helper path must be absolute"
            );
            return TerminalPtyHelper::new(
                program,
                vec![crate::terminal_helper::TERMINAL_PTY_HELPER_ARGUMENT.into()],
            )
            .unwrap();
        }
        TerminalPtyHelper::new(
            std::env::current_exe().unwrap(),
            vec![
                "--exact".into(),
                "terminal_pty::tests::helper_entry".into(),
                "--test-threads=1".into(),
                "--quiet".into(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn durable_history_resize_matches_the_real_pty_and_survives_recovery() {
        use crate::terminal_history::TerminalHistory;
        use crate::terminal_journal::{TerminalJournal, TerminalJournalLimits};
        use machine_god_core::{TerminalDimensions, TerminalSessionId};
        use std::os::unix::fs::PermissionsExt;

        let cwd = Directory::new();
        let storage = Directory::new();
        std::fs::set_permissions(&storage.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        let id = TerminalSessionId::new("pty-history").unwrap();
        let limits = TerminalJournalLimits::default();
        let journal = TerminalJournal::create(storage.fd(), id.clone(), limits).unwrap();
        let mut history =
            TerminalHistory::create(journal, &TerminalDimensions::new(24, 80).unwrap()).unwrap();
        // The shell does not check dimensions until the parent has committed
        // both the native resize and the replacement screen checkpoint.
        let mut pty = start(
            &cwd,
            &[
                "-c",
                "stty -echo; printf READY; read answer; stty size; printf FINISHED",
            ],
        );
        let ready = read_until(&mut pty, b"READY");
        history.append(&ready).unwrap();
        let dimensions = TerminalDimensions::new(7, 31).unwrap();
        history
            .resize(&dimensions, |size| {
                pty.resize(TerminalPtyDimensions {
                    rows: size.rows(),
                    columns: size.columns(),
                })
                .map_err(|_| ())
            })
            .unwrap();
        assert_eq!(pty.write(b"continue\n").unwrap().bytes_written(), 9);
        let output = read_until(&mut pty, b"FINISHED");
        assert!(output.windows(b"7 31".len()).any(|bytes| bytes == b"7 31"));
        history.append(&output).unwrap();
        let expected = history.screen().unwrap();
        assert_eq!(expected.dimensions, dimensions);
        // Recover from the resize checkpoint plus later native output, not
        // from a final checkpoint that could conceal a replay defect.
        let source = history.latest();
        drop(history);
        let deadline = Instant::now() + Duration::from_secs(2);
        let recovered = loop {
            match TerminalJournal::open_existing(storage.fd(), &id, limits) {
                Err(crate::terminal_journal::TerminalJournalError::Busy)
                    if Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                result => break TerminalHistory::recover(result.unwrap()).unwrap(),
            }
        };
        assert_eq!(recovered.latest(), source);
        assert_eq!(recovered.screen().unwrap(), expected);
        pty.close(true).unwrap();
    }

    fn request(directory: &Directory, args: &[&str]) -> TerminalPtyRequest {
        TerminalPtyRequest::new(
            "/bin/sh".into(),
            args.iter().map(|arg| (*arg).into()).collect(),
            vec![
                ("LANG".into(), "C".into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TERM".into(), "xterm-256color".into()),
            ],
            directory.fd(),
            TerminalPtyDimensions {
                rows: 24,
                columns: 80,
            },
        )
        .unwrap()
    }

    #[test]
    fn owned_session_driver_composes_real_input_resize_monitor_and_exit() {
        use crate::terminal_history::TerminalHistory;
        use crate::terminal_input::{TerminalInputProgress, TerminalWriterId};
        use crate::terminal_journal::{TerminalJournal, TerminalJournalLimits};
        use crate::terminal_monitor::{TerminalMonitorActivation, TerminalProcessOutcome};
        use crate::terminal_session::TerminalSession;
        use crate::terminal_session_record::test_metadata as meta;
        use machine_god_core::{
            BackgroundOutputOwner, SessionId, SessionIncarnationId, TerminalCursor,
            TerminalDimensions, TerminalEventQuery, TerminalLifecycle, TerminalMonitorCondition,
            TerminalMonitorDefinition, TerminalMonitorLifetime, TerminalMonitorOperation,
            TerminalNotifySchedule, TerminalSessionId,
        };
        use std::num::NonZeroU64;
        use std::os::unix::fs::PermissionsExt;

        let cwd = Directory::new();
        let storage = Directory::new();
        std::fs::set_permissions(&storage.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        let id = TerminalSessionId::new("driver-pty").unwrap();
        let owner = BackgroundOutputOwner::new(
            SessionId::new("driver-session").unwrap(),
            SessionIncarnationId::new("driver-incarnation").unwrap(),
        );
        let writer = TerminalWriterId::new(NonZeroU64::new(1).unwrap());
        let history = TerminalHistory::create(
            TerminalJournal::create(storage.fd(), id.clone(), TerminalJournalLimits::default())
                .unwrap(),
            &TerminalDimensions::new(24, 80).unwrap(),
        )
        .unwrap();
        let pty = start(
            &cwd,
            &[
                "-c",
                "stty -echo; printf READY; read answer; stty size; printf FINISHED; exit 23",
            ],
        );
        let mut session = TerminalSession::new(pty, history, owner.clone(), id, meta(), 0).unwrap();
        let monitor = session
            .monitor(
                &owner,
                TerminalMonitorOperation::Add {
                    definition: TerminalMonitorDefinition {
                        condition: TerminalMonitorCondition::OutputContains {
                            pattern: "7 31".into(),
                        },
                        check_schedule: None,
                        notify: TerminalNotifySchedule::OnMatch,
                        lifetime: TerminalMonitorLifetime::UntilSessionEnd,
                    },
                },
                TerminalMonitorActivation::default(),
                0,
            )
            .unwrap()
            .monitor_id;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = Vec::new();
        let mut operation = None;
        for now in 1..=2500 {
            assert!(Instant::now() < deadline, "session driver did not finish");
            let step = session.pump(now).unwrap();
            observed.extend(step.output);
            if operation.is_none() && observed.windows(5).any(|bytes| bytes == b"READY") {
                // This fixture's private handshake proves the script is waiting
                // for input. Production readiness uses its control channel.
                operation = Some(release_driver_fixture(&mut session, &owner, writer, now));
            }
            if session.context().lifecycle == TerminalLifecycle::Exited {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Exited);
        assert_eq!(session.outcome(), Some(TerminalProcessOutcome::Exited(23)));
        assert_eq!(
            session
                .write_receipt(&owner, writer, operation.unwrap())
                .unwrap()
                .progress,
            TerminalInputProgress::Complete
        );
        let raw = session
            .read(&owner, &TerminalCursor::new(1, 0).unwrap(), 4096)
            .unwrap()
            .bytes;
        assert!(raw.windows(4).any(|bytes| bytes == b"7 31"));
        assert!(raw.windows(8).any(|bytes| bytes == b"FINISHED"));
        assert!(
            session
                .events(
                    &owner,
                    &TerminalEventQuery {
                        after_event_id: 0,
                        acknowledge_event_id: None,
                        max_events: 256
                    }
                )
                .unwrap()
                .iter()
                .any(|event| event.monitor_id == monitor)
        );
    }

    fn release_driver_fixture(
        session: &mut crate::terminal_session::TerminalSession<TerminalPty>,
        owner: &machine_god_core::BackgroundOutputOwner,
        writer: crate::terminal_input::TerminalWriterId,
        now: i64,
    ) -> std::num::NonZeroU64 {
        use machine_god_core::{
            TerminalDimensions, TerminalWriteLeaseIntent, TerminalWritePayload,
            TerminalWriteRequest,
        };
        session.shell_ready(now).unwrap();
        session
            .resize(owner, &TerminalDimensions::new(7, 31).unwrap(), now)
            .unwrap();
        session
            .write(
                owner,
                writer,
                &TerminalWriteRequest {
                    lease: TerminalWriteLeaseIntent::Acquire,
                    payload: None,
                },
                false,
            )
            .unwrap();
        session
            .write(
                owner,
                writer,
                &TerminalWriteRequest {
                    lease: TerminalWriteLeaseIntent::Use,
                    payload: Some(TerminalWritePayload::Text {
                        text: "continue\n".into(),
                    }),
                },
                false,
            )
            .unwrap()
            .operation_id
            .unwrap()
    }
    fn start(directory: &Directory, args: &[&str]) -> TerminalPty {
        PreparedTerminalPty::prepare(
            &helper(),
            request(directory, args),
            &CancellationToken::new(),
        )
        .unwrap()
        .commit(&CancellationToken::new())
        .unwrap()
    }
    #[test]
    fn expired_eof_observation_cannot_hide_a_retained_running_child() {
        let directory = Directory::new();
        let mut pty = start(&directory, &["-c", "exec /bin/sleep 10"]);
        // Inject the already-observed EOF state: kernels differ in whether
        // closing all slave descriptors alone produces EOF before leader exit.
        pty.read_closed = true;
        pty.write_closed = true;
        pty.eof_deadline = Some(Instant::now());
        let mut buffer = [0; 64];
        assert!(pty.read(&mut buffer).unwrap().closed);
        assert_eq!(pty.write(b"ignored").unwrap().bytes_written(), 0);
        assert_eq!(pty.status().unwrap(), TerminalPtyStatus::Running);
        assert_ne!(pty.close(true).unwrap(), TerminalPtyStatus::Running);
        assert!(pty.read(&mut buffer).unwrap().closed);
    }
    fn read_until(pty: &mut TerminalPty, marker: &[u8]) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        while !bytes.windows(marker.len()).any(|part| part == marker) {
            assert!(
                Instant::now() < deadline,
                "PTY marker {:?} missing: {}",
                String::from_utf8_lossy(marker),
                String::from_utf8_lossy(&bytes)
            );
            let read = pty.read(&mut buffer).unwrap();
            bytes.extend_from_slice(&buffer[..read.bytes_read]);
            assert!(bytes.len() <= MAX_READ);
            assert!(
                !read.closed || bytes.windows(marker.len()).any(|part| part == marker),
                "PTY closed before marker: {}",
                String::from_utf8_lossy(&bytes)
            );
            if read.bytes_read == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        bytes
    }
    #[test]
    fn helper_entry() {
        if std::env::var_os("MACHINE_GOD_PTY_HELPER").is_some() {
            run_terminal_pty_helper().unwrap();
            unreachable!();
        }
    }

    #[test]
    fn full_command_boundary_executes_as_one_argument_and_reaps() {
        let directory = Directory::new();
        assert_eq!(MAX_ARGUMENT_BYTES, 64 * 1024);
        for padding in ["x", "\u{1}", "\"\\雪"] {
            let mut command = String::from("printf boundary; #");
            command.push_str(&padding.repeat((MAX_ARGUMENT_BYTES - command.len()) / padding.len()));
            command.push_str(&"x".repeat(MAX_ARGUMENT_BYTES - command.len()));
            let mut pty = start(&directory, &["-c", &command]);
            let pid =
                rustix::process::Pid::from_raw(i32::try_from(pty.pid().get()).unwrap()).unwrap();
            read_until(&mut pty, b"boundary");
            let deadline = Instant::now() + Duration::from_secs(2);
            while pty.status().unwrap() == TerminalPtyStatus::Running {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(pty.close(false).unwrap(), TerminalPtyStatus::Exited(0));
            assert_eq!(
                rustix::process::test_kill_process(pid),
                Err(rustix::io::Errno::SRCH)
            );
            command.push('x');
            assert!(validate_program_arguments("/bin/sh", &["-c".into(), command]).is_err());
        }
    }

    #[test]
    fn maximum_frame_roundtrips_all_bounded_fields_and_rejects_aggregate_overflow() {
        let directory = Directory::new();
        let mut request = request(&directory, &[]);
        request.program = format!("/{}", "p".repeat(MAX_PROGRAM_BYTES - 1));
        request.arguments = vec![String::new(); MAX_ARGUMENTS];
        request.arguments[0] = "c".repeat(MAX_ARGUMENT_BYTES);
        request.arguments[1] = "a".repeat(MAX_ARGUMENTS_BYTES - MAX_ARGUMENT_BYTES);
        let environment = (0..MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES)
            .map(|index| {
                let key = format!("K{index:03}");
                let value = "v".repeat(
                    MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES
                        / MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES
                        - key.len(),
                );
                (key.into(), value.into())
            })
            .collect();
        request.environment = ValidatedBackgroundEnvironment::new(environment).unwrap();
        validate_program_arguments(&request.program, &request.arguments).unwrap();
        let frame = request.frame().unwrap();
        assert_eq!(frame.len(), MAX_FRAME);
        let decoded = read_frame(
            &mut frame.as_slice(),
            Instant::now() + START_TIMEOUT,
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(decoded.program, request.program);
        assert_eq!(decoded.arguments, request.arguments);
        assert_eq!(decoded.environment.entries(), request.environment.entries());
        request.arguments[1].push('x');
        assert!(validate_program_arguments(&request.program, &request.arguments).is_err());
        assert!(request.frame().is_err());
    }

    #[test]
    fn actual_controlling_tty_dimensions_environment_and_owned_cwd() {
        let directory = Directory::new();
        let mut pty = start(
            &directory,
            &[
                "-c",
                "test -t 0 && test -t 1 && test -t 2 && test -c /dev/tty && stty size && printf 'TERM=%s\\n' \"$TERM\" && pwd && printf finished",
            ],
        );
        let output = read_until(&mut pty, b"finished");
        assert!(output.windows(5).any(|part| part == b"24 80"));
        assert!(String::from_utf8_lossy(&output).contains("TERM=xterm-256color"));
        assert!(
            String::from_utf8_lossy(&output)
                .contains(directory.0.file_name().unwrap().to_str().unwrap())
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while pty.status().unwrap() == TerminalPtyStatus::Running {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(pty.status().unwrap(), TerminalPtyStatus::Exited(0));
        assert_eq!(pty.close(false).unwrap(), TerminalPtyStatus::Exited(0));
    }

    #[test]
    fn partial_line_input_and_real_resize_are_visible_to_the_shell() {
        let directory = Directory::new();
        let mut pty = start(
            &directory,
            &[
                "-c",
                "stty -echo; printf ready; IFS= read -r line; printf 'line=%s\\n' \"$line\"; stty size; printf finished",
            ],
        );
        read_until(&mut pty, b"ready");
        assert_eq!(pty.write(b"first ").unwrap().bytes_written(), 6);
        pty.resize(TerminalPtyDimensions {
            rows: 41,
            columns: 103,
        })
        .unwrap();
        assert_eq!(pty.write("雪\n".as_bytes()).unwrap().bytes_written(), 4);
        let bytes = read_until(&mut pty, b"finished");
        assert!(String::from_utf8_lossy(&bytes).contains("line=first 雪"));
        assert!(String::from_utf8_lossy(&bytes).contains("41 103"));
        pty.close(true).unwrap();
    }

    #[test]
    fn control_c_targets_the_interactive_foreground_job_and_shell_survives() {
        let directory = Directory::new();
        let mut pty = start(&directory, &["-i"]);
        pty.write(b"stty -echo; printf '%s%s\\n' ready _marker\n")
            .unwrap();
        read_until(&mut pty, b"ready_marker");
        pty.write(b"/bin/sh -c 'printf child_active; exec /bin/sleep 30'\n")
            .unwrap();
        read_until(&mut pty, b"child_active");
        let deadline = Instant::now() + Duration::from_secs(2);
        while rustix::termios::tcgetpgrp(pty.master.as_ref().unwrap())
            .unwrap()
            .as_raw_nonzero()
            .get()
            .cast_unsigned()
            == pty.pid().get()
        {
            assert!(
                Instant::now() < deadline,
                "interactive foreground job was not installed"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        pty.write(b"\x03").unwrap();
        // Terminal input processing may flush bytes queued behind Ctrl-C.
        // Wait for the shell to regain foreground ownership before sending
        // the next command, rather than racing that line-discipline flush.
        let deadline = Instant::now() + Duration::from_secs(2);
        while rustix::termios::tcgetpgrp(pty.master.as_ref().unwrap())
            .unwrap()
            .as_raw_nonzero()
            .get()
            .cast_unsigned()
            != pty.pid().get()
        {
            assert!(
                Instant::now() < deadline,
                "shell did not regain the foreground after Ctrl-C"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        pty.write(b"printf foreground_survived\\n\n").unwrap();
        read_until(&mut pty, b"foreground_survived");
        assert_eq!(pty.status().unwrap(), TerminalPtyStatus::Running);
        pty.write(b"exit\n").unwrap();
        pty.close(false).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn owned_ioctl_interrupts_foreground_job_without_terminating_shell() {
        let directory = Directory::new();
        let mut pty = start(&directory, &["-i"]);
        pty.write(b"stty -echo; printf '%s%s\\n' shell _ready\n")
            .unwrap();
        read_until(&mut pty, b"shell_ready");
        pty.write(b"/bin/sh -c 'printf job_ready; exec /bin/sleep 30'\n")
            .unwrap();
        read_until(&mut pty, b"job_ready");
        pty.signal(BackgroundProcessSignal::Interrupt).unwrap();
        pty.write(b"printf '%s%s\\n' still _interactive\n").unwrap();
        read_until(&mut pty, b"still_interactive");
        assert_eq!(pty.status().unwrap(), TerminalPtyStatus::Running);
        pty.close(true).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn close_reaps_separate_foreground_and_background_jobs_ignoring_hup_and_term() {
        use machine_god_terminal_sys::ProcessIdentity;
        for force in [true, false] {
            let directory = Directory::new();
            let mut pty = start(&directory, &["-i"]);
            pty.write(b"stty -echo; printf '%s%s\\n' shell _ready\n")
                .unwrap();
            read_until(&mut pty, b"shell_ready");
            pty.write(b"/bin/sh -c 'trap \"\" HUP TERM; printf %s $$ > background.pid; printf background_ready; while :; do /bin/sleep 30; done' &\n").unwrap();
            read_until(&mut pty, b"background_ready");
            pty.write(b"/bin/sh -c 'trap \"\" HUP TERM; printf %s $$ > foreground.pid; printf foreground_ready; while :; do /bin/sleep 30; done'\n").unwrap();
            read_until(&mut pty, b"foreground_ready");
            let identity = |file: &str| {
                ProcessIdentity::capture(
                    NonZeroU32::new(
                        std::fs::read_to_string(directory.0.join(file))
                            .unwrap()
                            .parse()
                            .unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap()
            };
            let background = identity("background.pid");
            let foreground = identity("foreground.pid");
            for job in [background, foreground] {
                let pid = rustix::process::Pid::from_raw(i32::try_from(job.pid().get()).unwrap())
                    .unwrap();
                assert_ne!(
                    rustix::process::getpgid(Some(pid))
                        .unwrap()
                        .as_raw_nonzero()
                        .get()
                        .cast_unsigned(),
                    pty.pid().get()
                );
            }
            // A separate owned Child in another session/group must be untouched.
            let mut bystander = Command::new("/bin/sleep")
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let bystander_identity =
                ProcessIdentity::capture(NonZeroU32::new(bystander.id()).unwrap()).unwrap();
            let result = pty.close(force);
            let untouched = bystander.try_wait().unwrap().is_none();
            let _ = bystander.kill();
            let _ = bystander.wait();
            // Clean up exact test-owned incarnations even on a failed assertion.
            let background_alive = background.exists().unwrap();
            let foreground_alive = foreground.exists().unwrap();
            let _ = background.signal(rustix::process::Signal::KILL);
            let _ = foreground.signal(rustix::process::Signal::KILL);
            assert!(result.is_ok(), "close({force}) failed: {result:?}");
            assert!(!background_alive && !foreground_alive);
            assert!(untouched, "close touched unrelated {bystander_identity:?}");
        }
    }

    #[test]
    fn prepared_drop_and_cancelled_commit_never_execute_command() {
        let directory = Directory::new();
        let command = "printf forbidden > forbidden";
        let prepared = PreparedTerminalPty::prepare(
            &helper(),
            request(&directory, &["-c", command]),
            &CancellationToken::new(),
        )
        .unwrap();
        drop(prepared);
        assert!(!directory.0.join("forbidden").exists());
        let prepared = PreparedTerminalPty::prepare(
            &helper(),
            request(&directory, &["-c", command]),
            &CancellationToken::new(),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            prepared.commit(&cancellation).err().unwrap().kind(),
            TerminalPtyErrorKind::Cancelled
        );
        assert!(!directory.0.join("forbidden").exists());
    }

    #[test]
    fn output_flood_remains_bounded_and_force_close_reaps_shell() {
        let directory = Directory::new();
        let mut pty = start(
            &directory,
            &[
                "-c",
                "trap '' TERM HUP; while :; do printf '0123456789abcdef'; done",
            ],
        );
        let pid = pty.pid();
        let mut buffer = [0; 4096];
        let deadline = Instant::now() + Duration::from_secs(2);
        while pty.read(&mut buffer).unwrap().bytes_read == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let started = Instant::now();
        pty.close(true).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(matches!(
            rustix::process::waitid(
                rustix::process::WaitId::Pid(
                    rustix::process::Pid::from_raw(i32::try_from(pid.get()).unwrap()).unwrap()
                ),
                rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG
            ),
            Err(rustix::io::Errno::CHILD)
        ));
    }

    #[test]
    fn queued_startup_suffix_obeys_deadline_and_close_discards_it() {
        for expired in [false, true] {
            let directory = Directory::new();
            let source = if expired {
                format!(
                    "{}0123456789abcdef0123456789abcdef\n/bin/sleep 30\nprintf BAD > executed\n",
                    crate::terminal_helper::PACED_STARTUP_PREFIX
                )
            } else {
                format!("{}printf BAD > executed\n", ":\n".repeat(1024))
            };
            let prepared = PreparedTerminalPty::prepare(
                &helper(),
                request(&directory, &["-i"])
                    .with_startup_source(source)
                    .unwrap(),
                &CancellationToken::new(),
            )
            .unwrap();
            let mut pty = prepared.commit(&CancellationToken::new()).unwrap();
            assert!(pty.startup_source.is_some());
            if expired {
                pty.startup_deadline = Instant::now();
                assert!(
                    matches!(pty.read(&mut [0; 128]), Err(error) if error.kind() == TerminalPtyErrorKind::Timeout)
                );
            }
            pty.close(true).unwrap();
            assert!(pty.startup_source.is_none());
            assert!(!directory.0.join("executed").exists());
        }
    }

    #[test]
    fn startup_source_has_bounded_physical_lines_and_total_bytes() {
        let directory = Directory::new();
        for source in [
            String::new(),
            "unterminated".into(),
            "bad\0\n".into(),
            "bad\r\n".into(),
            format!("{}\n", "x".repeat(512)),
            "x\n".repeat(16 * 1024 + 1),
        ] {
            assert!(
                request(&directory, &["-i"])
                    .with_startup_source(source)
                    .is_err()
            );
        }
        let source = "x\n".repeat(16 * 1024);
        assert!(
            request(&directory, &["-i"])
                .with_startup_source(source)
                .is_ok()
        );
    }

    #[test]
    fn invalid_frames_and_requests_are_bounded_and_fail_closed() {
        let directory = Directory::new();
        let request = request(&directory, &["-c", "true"]);
        let frame = request.frame().unwrap();
        for length in 0..frame.len() {
            assert!(
                read_frame(
                    &mut &frame[..length],
                    Instant::now() + START_TIMEOUT,
                    &CancellationToken::new()
                )
                .is_err()
            );
        }
        let mut trailing = frame.clone();
        trailing[0] = 0;
        assert!(
            read_frame(
                &mut trailing.as_slice(),
                Instant::now() + START_TIMEOUT,
                &CancellationToken::new()
            )
            .is_err()
        );
        assert!(
            TerminalPtyDimensions {
                rows: 0,
                columns: 80
            }
            .validate()
            .is_err()
        );
        assert!(validate_program_arguments("relative", &[]).is_err());
        assert!(
            validate_program_arguments("/bin/sh", &["x".repeat(MAX_ARGUMENT_BYTES + 1)]).is_err()
        );
        assert!(Path::new(&request.program).is_absolute());
    }

    #[test]
    fn raw_terminal_query_bytes_and_binary_responses_are_not_interpreted() {
        let directory = Directory::new();
        let mut pty = start(
            &directory,
            &[
                "-c",
                "stty raw -echo; printf '\\033[6n'; dd bs=1 count=8 2>/dev/null > received; printf finished",
            ],
        );
        let output = read_until(&mut pty, b"\x1b[6n");
        assert!(output.ends_with(b"\x1b[6n"));
        let response = b"\x1b[4;9R\0\xff";
        assert_eq!(pty.write(response).unwrap().bytes_written(), response.len());
        read_until(&mut pty, b"finished");
        assert_eq!(
            std::fs::read(directory.0.join("received")).unwrap(),
            response
        );
        pty.close(true).unwrap();
    }

    #[test]
    fn graceful_close_has_800ms_budget_and_reports_unobserved_tail() {
        let directory = Directory::new();
        let mut pty = start(
            &directory,
            &["-c", "trap '' HUP TERM; printf ready; while :; do :; done"],
        );
        read_until(&mut pty, b"ready");
        let started = Instant::now();
        let closed = pty.close_with_output(false, |_| {}).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(800));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(closed.output_incomplete);
        assert_eq!(closed.status, TerminalPtyStatus::Signalled(9));
    }

    #[test]
    fn explicit_owned_signal_kills_shell_and_final_drain_preserves_bytes() {
        let directory = Directory::new();
        let mut pty = start(
            &directory,
            &["-c", "printf before_signal; while :; do :; done"],
        );
        read_until(&mut pty, b"before_signal");
        pty.signal(BackgroundProcessSignal::Kill).unwrap();
        let mut tail = Vec::new();
        let closed = pty
            .close_with_output(true, |bytes| tail.extend_from_slice(bytes))
            .unwrap();
        assert_eq!(closed.status, TerminalPtyStatus::Signalled(9));
        assert!(tail.len() <= 512 * 1024);
        assert_eq!(pty.close(false).unwrap(), closed.status);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn close_captures_reparented_same_session_job_after_shell_exit() {
        let directory = Directory::new();
        let mut pty = start(
            &directory,
            &[
                "-c",
                "(trap '' HUP TERM; while :; do :; done) & printf ready; exit 0",
            ],
        );
        read_until(&mut pty, b"ready");
        let deadline = Instant::now() + Duration::from_secs(2);
        while pty.status().unwrap() == TerminalPtyStatus::Running {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(pty.close(true).unwrap(), TerminalPtyStatus::Exited(0));
    }
}
