//! Private single-process terminal helper entrypoints and shared wire codecs.
//! This module has no dependency on session, history, registry or runtime state.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::background_process::{
    MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES, MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES,
    MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES, MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES,
    ValidatedBackgroundEnvironment,
};
use machine_god_core::CancellationToken;
use rustix::fd::AsFd;
use rustix::fs::{FileType, Mode, OFlags};
use rustix::termios::Winsize;
use std::ffi::OsString;
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Exact private executable mode for the descriptor-bound PTY helper.
#[doc(hidden)]
pub const TERMINAL_PTY_HELPER_ARGUMENT: &str = "--machine-god-terminal-pty-helper";
/// Exact private executable mode for a shell startup acknowledgement marker.
#[doc(hidden)]
pub const TERMINAL_STARTUP_MARKER_ARGUMENT: &str = "--machine-god-terminal-startup-marker";

/// Fixed, data-free failure from a private terminal helper.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalHelperError {
    pub(crate) kind: TerminalHelperErrorKind,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalHelperErrorKind {
    InvalidRequest,
    Cancelled,
    Process,
    Protocol,
}
impl fmt::Display for TerminalHelperError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal helper operation failed")
    }
}
impl std::error::Error for TerminalHelperError {}
const fn error(kind: TerminalHelperErrorKind) -> TerminalHelperError {
    TerminalHelperError { kind }
}
fn process_error(_: impl fmt::Debug) -> TerminalHelperError {
    error(TerminalHelperErrorKind::Process)
}
pub(crate) const MAX_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const MAX_SOCKET_PATH_BYTES: usize = 100;

pub(crate) const MAGIC: &[u8; 8] = b"MGPTY\0\0\x01";
pub(crate) const READY: u8 = 0xa7;
pub(crate) const COMMIT: u8 = 0x5b;
pub(crate) const MAX_PROGRAM_BYTES: usize = 4096;
pub(crate) const MAX_ARGUMENT_BYTES: usize = machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES;
// Keep the former aggregate allowance for auxiliary argv (shell flags, etc.)
// independently of the full command, which remains one unmodified argv item.
pub(crate) const MAX_ARGUMENTS_BYTES: usize = MAX_ARGUMENT_BYTES + 32 * 1024;
pub(crate) const MAX_ARGUMENTS: usize = 256;
// Binary fields do not escape: magic, dimensions, program, argv and environment
// bytes, with a u32 length for each string and both collection counts.
pub(crate) const MAX_FRAME: usize = MAGIC.len()
    + 4
    + MAX_PROGRAM_BYTES
    + MAX_ARGUMENTS_BYTES
    + MAX_BACKGROUND_PROCESS_ENVIRONMENT_BYTES
    + 4 * (3 + MAX_ARGUMENTS + 2 * MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES);
pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(2);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalPtyDimensions {
    pub(crate) rows: u16,
    pub(crate) columns: u16,
}
impl TerminalPtyDimensions {
    pub(crate) fn validate(self) -> Result<Self, TerminalHelperError> {
        if self.rows == 0 || self.columns == 0 {
            Err(error(TerminalHelperErrorKind::InvalidRequest))
        } else {
            Ok(self)
        }
    }
    pub(crate) fn winsize(self) -> Winsize {
        Winsize {
            ws_row: self.rows,
            ws_col: self.columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
}

/// Executes the already authorized PTY launch frame; success replaces this process.
///
/// # Errors
/// Returns a fixed failure for invalid descriptors, frames, cancellation or launch failure.
#[doc(hidden)]
pub fn run_terminal_pty_helper() -> Result<(), TerminalHelperError> {
    let input = std::io::stdin();
    let output = std::io::stdout();
    let cwd = std::io::stderr();
    validate_pty_directory(&cwd)?;
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
        return Err(error(TerminalHelperErrorKind::Process));
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

pub(crate) struct DescriptorIo<'a>(pub(crate) rustix::fd::BorrowedFd<'a>);
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

pub(crate) struct LaunchFrame {
    pub(crate) program: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) environment: ValidatedBackgroundEnvironment,
    pub(crate) dimensions: TerminalPtyDimensions,
}
pub(crate) fn read_frame(
    input: &mut impl Read,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<LaunchFrame, TerminalHelperError> {
    let mut magic = [0; 8];
    read_gate(input, &mut magic, deadline, cancellation)?;
    if &magic != MAGIC {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
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
        MAX_PROGRAM_BYTES,
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
    let count = read_length(
        input,
        MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES,
        &mut budget,
        deadline,
        cancellation,
    )?;
    let mut environment = Vec::with_capacity(count);
    for _ in 0..count {
        let key = OsString::from_vec(read_bytes(
            input,
            MAX_BACKGROUND_PROCESS_ENVIRONMENT_KEY_BYTES,
            &mut budget,
            deadline,
            cancellation,
        )?);
        let value = OsString::from_vec(read_bytes(
            input,
            MAX_BACKGROUND_PROCESS_ENVIRONMENT_VALUE_BYTES,
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
pub(crate) fn validate_program_arguments(
    program: &str,
    arguments: &[String],
) -> Result<(), TerminalHelperError> {
    if !program.starts_with('/')
        || program.len() > MAX_PROGRAM_BYTES
        || program.as_bytes().contains(&0)
        || arguments.len() > MAX_ARGUMENTS
        || arguments
            .iter()
            .any(|value| value.len() > MAX_ARGUMENT_BYTES || value.as_bytes().contains(&0))
        || arguments
            .iter()
            .map(String::len)
            .try_fold(0_usize, usize::checked_add)
            .is_none_or(|total| total > MAX_ARGUMENTS_BYTES)
    {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
    }
    Ok(())
}

fn read_length(
    input: &mut impl Read,
    maximum: usize,
    budget: &mut usize,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<usize, TerminalHelperError> {
    *budget = budget
        .checked_sub(4)
        .ok_or_else(|| error(TerminalHelperErrorKind::InvalidRequest))?;
    let mut bytes = [0; 4];
    read_gate(input, &mut bytes, deadline, cancellation)?;
    let length = usize::try_from(u32::from_be_bytes(bytes)).map_err(process_error)?;
    if length > maximum {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
    }
    Ok(length)
}
fn read_bytes(
    input: &mut impl Read,
    maximum: usize,
    budget: &mut usize,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, TerminalHelperError> {
    let length = read_length(input, maximum, budget, deadline, cancellation)?;
    *budget = budget
        .checked_sub(length)
        .ok_or_else(|| error(TerminalHelperErrorKind::InvalidRequest))?;
    let mut bytes = vec![0; length];
    read_gate(input, &mut bytes, deadline, cancellation)?;
    Ok(bytes)
}
pub(crate) fn write_gate(
    output: &mut impl Write,
    mut bytes: &[u8],
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), TerminalHelperError> {
    for _ in 0..65536 {
        if bytes.is_empty() {
            return Ok(());
        }
        check_deadline(deadline, cancellation)?;
        match output.write(bytes) {
            Ok(0) => return Err(error(TerminalHelperErrorKind::Process)),
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
    Err(error(TerminalHelperErrorKind::Process))
}
pub(crate) fn read_gate(
    input: &mut impl Read,
    mut bytes: &mut [u8],
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), TerminalHelperError> {
    for _ in 0..65536 {
        if bytes.is_empty() {
            return Ok(());
        }
        check_deadline(deadline, cancellation)?;
        match input.read(bytes) {
            Ok(0) => return Err(error(TerminalHelperErrorKind::Process)),
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
    Err(error(TerminalHelperErrorKind::Process))
}
fn check_deadline(
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(), TerminalHelperError> {
    if cancellation.is_cancelled() {
        Err(error(TerminalHelperErrorKind::Cancelled))
    } else if Instant::now() >= deadline {
        Err(error(TerminalHelperErrorKind::Process))
    } else {
        Ok(())
    }
}

pub(crate) fn startup_directory_identity(stat: &rustix::fs::Stat) -> String {
    format!("{}:{}", stat.st_dev, stat.st_ino)
}
pub(crate) fn validate_startup_directory(
    directory: &impl AsFd,
    path: &Path,
) -> Result<String, TerminalHelperError> {
    let stat = rustix::fs::fstat(directory).map_err(process_error)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_mode & 0o077 != 0
        || stat.st_uid != rustix::process::getuid().as_raw()
    {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
    }
    let reopened = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(process_error)?;
    if startup_directory_identity(&stat)
        != startup_directory_identity(&rustix::fs::fstat(reopened).map_err(process_error)?)
    {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
    }
    Ok(startup_directory_identity(&stat))
}
/// Private executable mode only: its isolated process may change cwd, never
/// the multithreaded owner. It holds no PTY/process cleanup authority.
///
/// # Errors
/// Returns a fixed failure for malformed configuration, directory identity or acknowledgement.
#[doc(hidden)]
pub fn run_terminal_startup_marker() -> Result<(), TerminalHelperError> {
    let value = |key: &str, limit: usize| -> Result<String, TerminalHelperError> {
        let value = std::env::var(key).map_err(process_error)?;
        if value.is_empty() || value.len() > limit || value.contains('\0') {
            return Err(error(TerminalHelperErrorKind::InvalidRequest));
        }
        Ok(value)
    };
    if value("MACHINE_GOD_STARTUP_MARKER", 1)? != "1" {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
    }
    let path = PathBuf::from(value(
        "MACHINE_GOD_STARTUP_DIRECTORY",
        MAX_SOCKET_PATH_BYTES,
    )?);
    let expected = value("MACHINE_GOD_STARTUP_ID", 64)?;
    let nonce = value("MACHINE_GOD_STARTUP_NONCE", 32)?;
    let kind = value("MACHINE_GOD_STARTUP_KIND", 1)?;
    if !path.is_absolute()
        || nonce.len() != 32
        || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !matches!(kind.as_str(), "R" | "C")
    {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
    }
    let directory = rustix::fs::open(
        &path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(process_error)?;
    if validate_startup_directory(&directory, &path)? != expected {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
    }
    rustix::process::fchdir(&directory).map_err(process_error)?;
    let mut stream = UnixStream::connect(format!("s-{nonce}")).map_err(process_error)?;
    // The owner enforces the shorter request deadline and owns process-tree
    // cancellation; the marker never invents a competing readiness timeout.
    let timeout = Some(MAX_STARTUP_TIMEOUT);
    stream.set_read_timeout(timeout).map_err(process_error)?;
    stream.set_write_timeout(timeout).map_err(process_error)?;
    stream
        .write_all(format!("{nonce}{kind}\n").as_bytes())
        .map_err(process_error)?;
    let mut ack = [0; 2];
    stream.read_exact(&mut ack).map_err(process_error)?;
    if ack != [kind.as_bytes()[0], b'\n'] {
        return Err(error(TerminalHelperErrorKind::Protocol));
    }
    Ok(())
}

pub(crate) fn validate_pty_directory(fd: &impl AsFd) -> Result<(), TerminalHelperError> {
    let metadata = rustix::fs::fstat(fd).map_err(process_error)?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::Directory {
        return Err(error(TerminalHelperErrorKind::InvalidRequest));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_helper_entry() {
        match std::env::var("MACHINE_GOD_HELPER_TEST_CASE").as_deref() {
            Ok("marker") => assert!(run_terminal_startup_marker().is_err()),
            Ok("pty") => assert!(run_terminal_pty_helper().is_err()),
            _ => {}
        }
    }

    #[test]
    fn malformed_private_invocations_fail_without_a_runtime() {
        let cases = [
            ("pty", None),
            ("marker", Some(("MACHINE_GOD_STARTUP_MARKER", "0"))),
            (
                "marker",
                Some(("MACHINE_GOD_STARTUP_DIRECTORY", "relative")),
            ),
            ("marker", Some(("MACHINE_GOD_STARTUP_NONCE", "invalid"))),
            ("marker", Some(("MACHINE_GOD_STARTUP_KIND", "X"))),
            ("marker", Some(("MACHINE_GOD_STARTUP_ID", ""))),
        ];
        for (mode, invalid) in cases {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "terminal_helper::tests::malformed_helper_entry",
                    "--test-threads=1",
                    "--quiet",
                ])
                .env_clear()
                .env("MACHINE_GOD_HELPER_TEST_CASE", mode)
                .env("MACHINE_GOD_STARTUP_MARKER", "1")
                .env("MACHINE_GOD_STARTUP_DIRECTORY", "/")
                .env("MACHINE_GOD_STARTUP_ID", "0:0")
                .env(
                    "MACHINE_GOD_STARTUP_NONCE",
                    "0123456789abcdef0123456789abcdef",
                )
                .env("MACHINE_GOD_STARTUP_KIND", "R")
                .stdin(Stdio::null());
            if let Some((key, value)) = invalid {
                command.env(key, value);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "helper rejection failed: {mode}");
        }
    }

    #[test]
    fn helper_errors_are_fixed_and_data_free() {
        let failure = process_error("private path and environment must not be retained");
        assert_eq!(failure.to_string(), "terminal helper operation failed");
        assert!(!format!("{failure:?}").contains("private"));
    }
}
