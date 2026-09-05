//! Owned, nonblocking native pseudoterminal transport.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::ffi::OsString;
use std::fmt;
use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use machine_god_core::CancellationToken;
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{FileType, Mode, OFlags};
use rustix::termios::Winsize;

use crate::background_input::{
    BackgroundInputReceipt, BackgroundInputStatus, MAX_BACKGROUND_INPUT_BYTES,
};
use crate::background_process::{
    BackgroundProcessExit, BackgroundProcessSignal, OwnedBackgroundProcess, TerminalChildGuard,
    TerminalClosePhase, ValidatedBackgroundEnvironment,
};

const MAGIC: &[u8; 8] = b"MGPTY\0\0\x01";
const READY: u8 = 0xa7;
const COMMIT: u8 = 0x5b;
const MAX_FRAME: usize = 320 * 1024;
const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_ARGUMENTS: usize = 256;
const MAX_READ: usize = 64 * 1024;
const START_TIMEOUT: Duration = Duration::from_secs(2);
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
const fn error(kind: TerminalPtyErrorKind) -> TerminalPtyError {
    TerminalPtyError { kind }
}
fn process_error(_: impl fmt::Debug) -> TerminalPtyError {
    error(TerminalPtyErrorKind::Process)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalPtyDimensions {
    pub(crate) rows: u16,
    pub(crate) columns: u16,
}
impl TerminalPtyDimensions {
    fn validate(self) -> Result<Self, TerminalPtyError> {
        if self.rows == 0 || self.columns == 0 {
            Err(error(TerminalPtyErrorKind::InvalidRequest))
        } else {
            Ok(self)
        }
    }
    fn winsize(self) -> Winsize {
        Winsize {
            ws_row: self.rows,
            ws_col: self.columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
}

pub(crate) struct TerminalPtyRequest {
    program: String,
    arguments: Vec<String>,
    environment: ValidatedBackgroundEnvironment,
    cwd: OwnedFd,
    dimensions: TerminalPtyDimensions,
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
        })
    }
    fn frame(&self) -> Result<Vec<u8>, TerminalPtyError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.dimensions.rows.to_be_bytes());
        bytes.extend_from_slice(&self.dimensions.columns.to_be_bytes());
        append_bytes(&mut bytes, self.program.as_bytes())?;
        append_length(&mut bytes, self.arguments.len())?;
        for argument in &self.arguments {
            append_bytes(&mut bytes, argument.as_bytes())?;
        }
        append_length(&mut bytes, self.environment.entries().len())?;
        for (key, value) in self.environment.entries() {
            append_bytes(&mut bytes, key.as_bytes())?;
            append_bytes(&mut bytes, value.as_bytes())?;
        }
        if bytes.len() > MAX_FRAME {
            return Err(error(TerminalPtyErrorKind::InvalidRequest));
        }
        Ok(bytes)
    }
}

pub(crate) struct TerminalPtyHelper {
    program: PathBuf,
    arguments: Vec<OsString>,
}
impl TerminalPtyHelper {
    pub(crate) fn new(
        program: PathBuf,
        arguments: Vec<OsString>,
    ) -> Result<Self, TerminalPtyError> {
        if !program.is_absolute()
            || program.as_os_str().as_bytes().contains(&0)
            || arguments.len() > 16
            || arguments
                .iter()
                .any(|value| value.as_bytes().contains(&0) || value.len() > 4096)
        {
            return Err(error(TerminalPtyErrorKind::InvalidRequest));
        }
        Ok(Self { program, arguments })
    }
}

pub(crate) struct PreparedTerminalPty {
    process: Option<OwnedBackgroundProcess>,
    master: Option<OwnedFd>,
    gate: Option<UnixStream>,
    permit: Option<PtyPermit>,
}

impl PreparedTerminalPty {
    pub(crate) fn prepare(
        helper: &TerminalPtyHelper,
        request: TerminalPtyRequest,
        cancellation: &CancellationToken,
    ) -> Result<Self, TerminalPtyError> {
        if cancellation.is_cancelled() {
            return Err(error(TerminalPtyErrorKind::Cancelled));
        }
        let permit = PtyPermit::acquire()?;
        let frame = request.frame()?;
        #[cfg(target_os = "macos")]
        machine_god_terminal_sys::ProcessIdentity::verify_signal_support()
            .map_err(process_error)?;
        let (master, slave) = open_pty(request.dimensions)?;
        let (mut gate, child_gate) = UnixStream::pair().map_err(process_error)?;
        gate.set_nonblocking(true).map_err(process_error)?;
        let mut guard = TerminalChildGuard::reserve(cancellation).map_err(process_error)?;
        let mut command = Command::new(&helper.program);
        command
            .args(&helper.arguments)
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("MACHINE_GOD_PTY_HELPER", "1")
            .stdin(Stdio::from(OwnedFd::from(child_gate)))
            .stdout(Stdio::from(slave))
            .stderr(Stdio::from(request.cwd));
        guard.spawn(&mut command).map_err(process_error)?;
        let deadline = Instant::now() + START_TIMEOUT;
        write_gate(&mut gate, &frame, deadline, cancellation)?;
        let mut ready = [0];
        read_gate(&mut gate, &mut ready, deadline, cancellation)?;
        if ready != [READY] {
            return Err(error(TerminalPtyErrorKind::Process));
        }
        let process = guard.into_session().map_err(process_error)?;
        Ok(Self {
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
        write_gate(
            gate,
            &[COMMIT],
            Instant::now() + START_TIMEOUT,
            cancellation,
        )?;
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
            pid: process.pid(),
            process: Some(process),
            master: Some(master),
            observed: None,
            read_closed: false,
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
    pid: NonZeroU32,
    process: Option<OwnedBackgroundProcess>,
    master: Option<OwnedFd>,
    observed: Option<TerminalPtyStatus>,
    read_closed: bool,
    write_closed: bool,
    output_incomplete: bool,
    permit: Option<PtyPermit>,
}
impl TerminalPty {
    pub(crate) const fn pid(&self) -> NonZeroU32 {
        self.pid
    }
    pub(crate) fn read(&mut self, buffer: &mut [u8]) -> Result<TerminalPtyRead, TerminalPtyError> {
        if buffer.is_empty() || buffer.len() > MAX_READ {
            return Err(error(TerminalPtyErrorKind::InvalidRequest));
        }
        if self.read_closed {
            return Ok(TerminalPtyRead {
                bytes_read: 0,
                closed: true,
            });
        }
        let Some(master) = self.master.as_ref() else {
            return Err(error(TerminalPtyErrorKind::Closed));
        };
        for _ in 0..32 {
            match rustix::io::read(master, &mut *buffer) {
                Ok(0) | Err(rustix::io::Errno::IO) => {
                    self.read_closed = true;
                    return Ok(TerminalPtyRead {
                        bytes_read: 0,
                        closed: true,
                    });
                }
                Ok(bytes_read) => {
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
    pub(crate) fn write(
        &mut self,
        bytes: &[u8],
    ) -> Result<BackgroundInputReceipt, TerminalPtyError> {
        if bytes.is_empty() || bytes.len() > MAX_BACKGROUND_INPUT_BYTES {
            return Err(error(TerminalPtyErrorKind::InvalidRequest));
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
impl Drop for TerminalPty {
    fn drop(&mut self) {
        let _ = self.close(true);
        drop(self.master.take());
        drop(self.process.take());
    }
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

pub(crate) fn run_terminal_pty_helper() -> Result<(), TerminalPtyError> {
    let input = std::io::stdin();
    let output = std::io::stdout();
    let cwd = std::io::stderr();
    validate_directory(&cwd)?;
    let flags = rustix::fs::fcntl_getfl(input.as_fd()).map_err(process_error)?;
    rustix::fs::fcntl_setfl(input.as_fd(), flags | OFlags::NONBLOCK).map_err(process_error)?;
    let cancellation = CancellationToken::new();
    let deadline = Instant::now() + START_TIMEOUT;
    let mut io = DescriptorIo(input.as_fd());
    let frame = read_frame(&mut io, deadline, &cancellation)?;
    rustix::process::fchdir(&cwd).map_err(process_error)?;
    rustix::process::setsid().map_err(process_error)?;
    rustix::process::ioctl_tiocsctty(&output).map_err(process_error)?;
    rustix::termios::tcsetwinsize(&output, frame.dimensions.winsize()).map_err(process_error)?;
    write_gate(&mut io, &[READY], deadline, &cancellation)?;
    let mut commit = [0];
    read_gate(&mut io, &mut commit, deadline, &cancellation)?;
    if commit != [COMMIT] {
        return Err(error(TerminalPtyErrorKind::Process));
    }
    let slave_in = rustix::io::fcntl_dupfd_cloexec(&output, 3).map_err(process_error)?;
    let slave_out = rustix::io::fcntl_dupfd_cloexec(&output, 3).map_err(process_error)?;
    let slave_err = rustix::io::fcntl_dupfd_cloexec(&output, 3).map_err(process_error)?;
    let mut shell = Command::new(frame.program);
    shell
        .args(frame.arguments)
        .env_clear()
        .envs(
            frame
                .environment
                .entries()
                .iter()
                .map(|(key, value)| (key, value)),
        )
        .stdin(Stdio::from(slave_in))
        .stdout(Stdio::from(slave_out))
        .stderr(Stdio::from(slave_err));
    Err(process_error(shell.exec()))
}

struct DescriptorIo<'a>(rustix::fd::BorrowedFd<'a>);
impl Read for DescriptorIo<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        rustix::io::read(self.0, bytes).map_err(Into::into)
    }
}
impl Write for DescriptorIo<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        rustix::io::write(self.0, bytes).map_err(Into::into)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct LaunchFrame {
    program: String,
    arguments: Vec<String>,
    environment: ValidatedBackgroundEnvironment,
    dimensions: TerminalPtyDimensions,
}
fn read_frame(
    input: &mut impl Read,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<LaunchFrame, TerminalPtyError> {
    let mut magic = [0; 8];
    read_gate(input, &mut magic, deadline, cancellation)?;
    if &magic != MAGIC {
        return Err(error(TerminalPtyErrorKind::InvalidRequest));
    }
    let mut dimensions = [0; 4];
    read_gate(input, &mut dimensions, deadline, cancellation)?;
    let dimensions = TerminalPtyDimensions {
        rows: u16::from_be_bytes([dimensions[0], dimensions[1]]),
        columns: u16::from_be_bytes([dimensions[2], dimensions[3]]),
    }
    .validate()?;
    let mut budget = MAX_FRAME - 12;
    let program = String::from_utf8(read_bytes(
        input,
        4096,
        &mut budget,
        deadline,
        cancellation,
    )?)
    .map_err(process_error)?;
    let count = read_length(input, MAX_ARGUMENTS, &mut budget, deadline, cancellation)?;
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        arguments.push(
            String::from_utf8(read_bytes(
                input,
                MAX_ARGUMENT_BYTES,
                &mut budget,
                deadline,
                cancellation,
            )?)
            .map_err(process_error)?,
        );
    }
    validate_program_arguments(&program, &arguments)?;
    let count = read_length(input, 512, &mut budget, deadline, cancellation)?;
    let mut environment = Vec::with_capacity(count);
    for _ in 0..count {
        let key = OsString::from_vec(read_bytes(
            input,
            1024,
            &mut budget,
            deadline,
            cancellation,
        )?);
        let value = OsString::from_vec(read_bytes(
            input,
            16 * 1024,
            &mut budget,
            deadline,
            cancellation,
        )?);
        environment.push((key, value));
    }
    let environment = ValidatedBackgroundEnvironment::new(environment).map_err(process_error)?;
    Ok(LaunchFrame {
        program,
        arguments,
        environment,
        dimensions,
    })
}
fn validate_program_arguments(program: &str, arguments: &[String]) -> Result<(), TerminalPtyError> {
    if !program.starts_with('/')
        || program.len() > 4096
        || program.as_bytes().contains(&0)
        || arguments.len() > MAX_ARGUMENTS
        || arguments.iter().any(|value| value.as_bytes().contains(&0))
        || arguments
            .iter()
            .map(String::len)
            .try_fold(0_usize, usize::checked_add)
            .is_none_or(|total| total > MAX_ARGUMENT_BYTES)
    {
        return Err(error(TerminalPtyErrorKind::InvalidRequest));
    }
    Ok(())
}
fn validate_directory(fd: &impl AsFd) -> Result<(), TerminalPtyError> {
    let metadata = rustix::fs::fstat(fd).map_err(process_error)?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::Directory {
        return Err(error(TerminalPtyErrorKind::InvalidRequest));
    }
    Ok(())
}
fn append_length(bytes: &mut Vec<u8>, length: usize) -> Result<(), TerminalPtyError> {
    bytes.extend_from_slice(&u32::try_from(length).map_err(process_error)?.to_be_bytes());
    Ok(())
}
fn append_bytes(bytes: &mut Vec<u8>, data: &[u8]) -> Result<(), TerminalPtyError> {
    append_length(bytes, data.len())?;
    bytes.extend_from_slice(data);
    Ok(())
}
fn read_length(
    input: &mut impl Read,
    maximum: usize,
    budget: &mut usize,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<usize, TerminalPtyError> {
    *budget = budget
        .checked_sub(4)
        .ok_or_else(|| error(TerminalPtyErrorKind::InvalidRequest))?;
    let mut bytes = [0; 4];
    read_gate(input, &mut bytes, deadline, cancellation)?;
    let length = usize::try_from(u32::from_be_bytes(bytes)).map_err(process_error)?;
    if length > maximum {
        return Err(error(TerminalPtyErrorKind::InvalidRequest));
    }
    Ok(length)
}
fn read_bytes(
    input: &mut impl Read,
    maximum: usize,
    budget: &mut usize,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, TerminalPtyError> {
    let length = read_length(input, maximum, budget, deadline, cancellation)?;
    *budget = budget
        .checked_sub(length)
        .ok_or_else(|| error(TerminalPtyErrorKind::InvalidRequest))?;
    let mut bytes = vec![0; length];
    read_gate(input, &mut bytes, deadline, cancellation)?;
    Ok(bytes)
}
fn write_gate(
    output: &mut impl Write,
    mut bytes: &[u8],
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), TerminalPtyError> {
    for _ in 0..65536 {
        if bytes.is_empty() {
            return Ok(());
        }
        check_deadline(deadline, cancellation)?;
        match output.write(bytes) {
            Ok(0) => return Err(error(TerminalPtyErrorKind::Process)),
            Ok(count) => bytes = &bytes[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(other) => return Err(process_error(other)),
        }
    }
    Err(error(TerminalPtyErrorKind::Process))
}
fn read_gate(
    input: &mut impl Read,
    mut bytes: &mut [u8],
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), TerminalPtyError> {
    for _ in 0..65536 {
        if bytes.is_empty() {
            return Ok(());
        }
        check_deadline(deadline, cancellation)?;
        match input.read(bytes) {
            Ok(0) => return Err(error(TerminalPtyErrorKind::Process)),
            Ok(count) => bytes = &mut bytes[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(other) => return Err(process_error(other)),
        }
    }
    Err(error(TerminalPtyErrorKind::Process))
}
fn check_deadline(
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), TerminalPtyError> {
    if cancellation.is_cancelled() {
        Err(error(TerminalPtyErrorKind::Cancelled))
    } else if Instant::now() >= deadline {
        Err(error(TerminalPtyErrorKind::Process))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
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
        let recovered = TerminalHistory::recover(
            TerminalJournal::open_existing(storage.fd(), &id, limits).unwrap(),
        )
        .unwrap();
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
    fn read_until(pty: &mut TerminalPty, marker: &[u8]) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        while !bytes.windows(marker.len()).any(|part| part == marker) {
            assert!(
                Instant::now() < deadline,
                "PTY marker missing: {}",
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
        pty.write(b"/bin/sleep 30\n").unwrap();
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
