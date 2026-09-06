//! Standalone private tmux helper and bounded shared wire protocol.
//! No session, registry, runtime, or terminal transport ownership dependency.

use crate::terminal_helper::{
    COMMIT, READY, read_frame, read_gate, startup_directory_identity, validate_startup_directory,
    write_gate,
};
use machine_god_core::CancellationToken;
use rustix::fd::OwnedFd;
use rustix::fs::{Mode, OFlags};
use std::ffi::OsString;
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) const PROOF_BYTES: usize = 36;
pub(crate) const CAPTURE_CHUNK: usize = 16 * 1024;
pub(crate) const PAUSE: Duration = Duration::from_millis(2);
pub(crate) const MAX_HELPER_FRAME: usize = 12 * 1024;

type RelativeCommand = (PathBuf, String, PathBuf, Vec<OsString>);

/// The child opens and validates the exact directory before changing cwd and
/// exec; a path swap between the host check and spawn never selects authority.
#[cfg(any(test, feature = "ai-gateway-http"))]
pub(crate) fn relative_command(
    helper: &crate::terminal_helper::TerminalPtyHelper,
    directory: &OwnedFd,
    path: &Path,
    executable: &Path,
    arguments: &[OsString],
) -> Result<Command> {
    let identity = validate_startup_directory(directory, path).map_err(gate_error)?;
    let frame =
        serde_json::to_string(&(path, identity, executable, arguments)).map_err(process_error)?;
    if frame.len() > 64 * 1024 {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    let mut command = Command::new(helper.program());
    command
        .args(helper.arguments())
        .args(["exec", &frame, "-", "-"]);
    Ok(command)
}

fn run_relative_command(frame: &std::ffi::OsStr) -> Result<()> {
    if frame.as_bytes().len() > 64 * 1024 {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    let (path, expected, executable, arguments): RelativeCommand =
        serde_json::from_slice(frame.as_bytes()).map_err(process_error)?;
    if !path.is_absolute()
        || path.as_os_str().as_bytes().len() > crate::terminal_helper::MAX_STARTUP_PATH_BYTES
        || !executable.is_absolute()
        || executable.as_os_str().as_bytes().len() > 4096
    {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    let directory = rustix::fs::open(
        &path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(process_error)?;
    if validate_startup_directory(&directory, &path).map_err(gate_error)? != expected {
        return Err(TerminalTmuxLaunchError::Identity);
    }
    rustix::process::fchdir(&directory).map_err(process_error)?;
    let mut command = Command::new(executable);
    command.args(arguments);
    #[cfg(test)]
    if std::env::var("MG_TMUX_KIND").as_deref() == Ok("exec") {
        // The unit harness writes a preamble before entering this function.
        // Its launcher hides that preamble and retains the real output pipe
        // on stderr; the production CLI does not need this fixture adapter.
        command.stdout(Stdio::from(
            rustix::io::fcntl_dupfd_cloexec(std::io::stderr(), 3).map_err(process_error)?,
        ));
    }
    Err(process_error(command.exec()))
}

/// Private, bounded helper dispatch; never part of ordinary CLI configuration.
#[doc(hidden)]
pub const TERMINAL_TMUX_HELPER_ARGUMENT: &str = "--machine-god-terminal-tmux-helper";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalTmuxLaunchError {
    Invalid,
    Cancelled,
    Timeout,
    Identity,
    Process,
    Protocol,
    Cleanup,
}
impl fmt::Display for TerminalTmuxLaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal tmux launch failed")
    }
}
impl std::error::Error for TerminalTmuxLaunchError {}
pub(crate) type Result<T> = std::result::Result<T, TerminalTmuxLaunchError>;
pub(crate) fn process_error(_: impl fmt::Debug) -> TerminalTmuxLaunchError {
    TerminalTmuxLaunchError::Process
}
pub(crate) fn gate_error(
    error: crate::terminal_helper::TerminalHelperError,
) -> TerminalTmuxLaunchError {
    use crate::terminal_helper::TerminalHelperErrorKind;
    match error.kind {
        TerminalHelperErrorKind::Cancelled => TerminalTmuxLaunchError::Cancelled,
        TerminalHelperErrorKind::Timeout => TerminalTmuxLaunchError::Timeout,
        _ => TerminalTmuxLaunchError::Protocol,
    }
}

fn connect_relative(directory: &OwnedFd, path: &Path) -> Result<UnixStream> {
    let cwd = rustix::fs::open(
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(process_error)?;
    rustix::process::fchdir(directory).map_err(process_error)?;
    let connected = UnixStream::connect(path.file_name().ok_or(TerminalTmuxLaunchError::Invalid)?);
    rustix::process::fchdir(&cwd).map_err(process_error)?;
    connected.map_err(process_error)
}

fn read_helper_frame(
    channel: &mut UnixStream,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<Vec<OsString>> {
    let mut length = [0; 4];
    read_gate(channel, &mut length, deadline, cancellation).map_err(gate_error)?;
    let length = usize::try_from(u32::from_be_bytes(length)).map_err(process_error)?;
    if !(4..=MAX_HELPER_FRAME).contains(&length) {
        return Err(TerminalTmuxLaunchError::Protocol);
    }
    let mut payload = vec![0; length];
    read_gate(channel, &mut payload, deadline, cancellation).map_err(gate_error)?;
    let mut remaining = payload.as_slice();
    let count = read_length(&mut remaining)?;
    if !(5..=261).contains(&count) {
        return Err(TerminalTmuxLaunchError::Protocol);
    }
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        let length = read_length(&mut remaining)?;
        let bytes = remaining
            .get(..length)
            .ok_or(TerminalTmuxLaunchError::Protocol)?;
        if bytes.contains(&0) {
            return Err(TerminalTmuxLaunchError::Protocol);
        }
        arguments.push(OsString::from_vec(bytes.to_vec()));
        remaining = &remaining[length..];
    }
    if !remaining.is_empty() || !Path::new(&arguments[0]).is_absolute() {
        return Err(TerminalTmuxLaunchError::Protocol);
    }
    Ok(arguments)
}
fn read_length(bytes: &mut &[u8]) -> Result<usize> {
    let length = u32::from_be_bytes(
        bytes
            .get(..4)
            .ok_or(TerminalTmuxLaunchError::Protocol)?
            .try_into()
            .map_err(process_error)?,
    );
    *bytes = &bytes[4..];
    usize::try_from(length).map_err(process_error)
}

/// Private CLI entrypoint; the host passes exactly four trailing arguments.
/// The helper executes no model-provided argv before its authenticated COMMIT.
///
/// # Errors
/// Returns a fixed failure for malformed private input, broken authentication,
/// expired startup or native process/capture failure.
#[doc(hidden)]
#[allow(
    clippy::too_many_lines,
    reason = "Private launch phases keep the gated child guard live through every fallible handshake."
)]
pub fn run_terminal_tmux_helper(arguments: &[OsString]) -> Result<()> {
    if arguments.len() != 4 {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    let kind = arguments[0]
        .to_str()
        .ok_or(TerminalTmuxLaunchError::Invalid)?;
    if kind == "exec" {
        if arguments[2] != "-" || arguments[3] != "-" {
            return Err(TerminalTmuxLaunchError::Invalid);
        }
        return run_relative_command(&arguments[1]);
    }
    if kind == "bind" {
        return crate::terminal_helper::run_startup_bind(
            Path::new(&arguments[1]),
            arguments[2]
                .to_str()
                .ok_or(TerminalTmuxLaunchError::Invalid)?,
            arguments[3]
                .to_str()
                .ok_or(TerminalTmuxLaunchError::Invalid)?,
        )
        .map_err(gate_error);
    }
    if !matches!(kind, "pane" | "capture" | "shell") {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    let socket = PathBuf::from(&arguments[1]);
    let nonce: [u8; 32] = arguments[2].as_bytes().try_into().map_err(process_error)?;
    if !nonce
        .iter()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        || !socket.is_absolute()
        || socket.as_os_str().as_bytes().len() > crate::terminal_helper::MAX_STARTUP_PATH_BYTES + 35
    {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    if kind == "shell" {
        return run_shell_child();
    }
    let directory_path = socket.parent().ok_or(TerminalTmuxLaunchError::Invalid)?;
    let directory = rustix::fs::open(
        directory_path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(process_error)?;
    validate_startup_directory(&directory, directory_path).map_err(gate_error)?;
    let deadline = Instant::now() + crate::terminal_helper::MAX_STARTUP_TIMEOUT;
    let cancellation = CancellationToken::new();
    let mut channel = connect_relative(&directory, &socket)?;
    channel.set_nonblocking(true).map_err(process_error)?;
    let mut proof = [0; PROOF_BYTES];
    proof[..32].copy_from_slice(&nonce);
    proof[32..].copy_from_slice(&std::process::id().to_be_bytes());
    write_gate(&mut channel, &proof, deadline, &cancellation).map_err(gate_error)?;
    let mut ready = [0];
    read_gate(&mut channel, &mut ready, deadline, &cancellation).map_err(gate_error)?;
    if ready != [READY] {
        return Err(TerminalTmuxLaunchError::Protocol);
    }
    if kind == "capture" {
        let mut completion_nonce = [0; 32];
        read_gate(&mut channel, &mut completion_nonce, deadline, &cancellation)
            .map_err(gate_error)?;
        let mut completion = connect_relative(&directory, &socket)?;
        completion.set_nonblocking(true).map_err(process_error)?;
        proof[..32].copy_from_slice(&completion_nonce);
        write_gate(&mut completion, &proof, deadline, &cancellation).map_err(gate_error)?;
        read_gate(&mut completion, &mut ready, deadline, &cancellation).map_err(gate_error)?;
        if ready != [READY] {
            return Err(TerminalTmuxLaunchError::Protocol);
        }
        let result = copy_capture(&mut channel);
        drop(channel);
        write_gate(
            &mut completion,
            &[b'C', u8::from(result.is_err())],
            Instant::now() + Duration::from_secs(2),
            &cancellation,
        )
        .map_err(gate_error)?;
        return result;
    }
    let cwd = rustix::fs::open(
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(process_error)?;
    if arguments[3]
        != OsString::from(startup_directory_identity(
            &rustix::fs::fstat(&cwd).map_err(process_error)?,
        ))
    {
        return Err(TerminalTmuxLaunchError::Identity);
    }
    let mut echo = [0];
    read_gate(&mut channel, &mut echo, deadline, &cancellation).map_err(gate_error)?;
    match echo {
        [0] => {}
        [1] => set_echo(std::io::stdin(), false)?,
        _ => return Err(TerminalTmuxLaunchError::Protocol),
    }
    let child_frame = {
        let mut recording = RecordedFrame {
            source: &mut channel,
            bytes: Vec::new(),
        };
        read_frame(&mut recording, deadline, &cancellation).map_err(gate_error)?;
        recording.bytes
    };
    let child_arguments = read_helper_frame(&mut channel, deadline, &cancellation)?;
    let (mut child_gate, child_peer) = UnixStream::pair().map_err(process_error)?;
    child_gate.set_nonblocking(true).map_err(process_error)?;
    child_peer.set_nonblocking(true).map_err(process_error)?;
    let mut child = ProvisionalChild(Some(
        Command::new(&child_arguments[0])
            .args(&child_arguments[1..])
            .env_clear()
            .process_group(0)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::from(OwnedFd::from(child_peer)))
            .spawn()
            .map_err(process_error)?,
    ));
    // Reuse the exact validated launch frame for the child. Its original tty
    // stdin/stdout remain untouched; the private gate temporarily uses stderr.
    write_gate(&mut child_gate, &child_frame, deadline, &cancellation).map_err(gate_error)?;
    let mut child_ready = [0];
    read_gate(&mut child_gate, &mut child_ready, deadline, &cancellation).map_err(gate_error)?;
    if child_ready != [READY] {
        return Err(TerminalTmuxLaunchError::Protocol);
    }
    write_gate(&mut channel, &[READY], deadline, &cancellation).map_err(gate_error)?;
    let mut challenge = [0; 32];
    read_gate(&mut channel, &mut challenge, deadline, &cancellation).map_err(gate_error)?;
    for (byte, nonce) in challenge.iter_mut().zip(nonce) {
        *byte ^= nonce;
    }
    write_gate(&mut channel, &challenge, deadline, &cancellation).map_err(gate_error)?;
    let mut commit = [0];
    read_gate(&mut channel, &mut commit, deadline, &cancellation).map_err(gate_error)?;
    if commit != [COMMIT] {
        return Err(TerminalTmuxLaunchError::Protocol);
    }
    let pid = rustix::process::Pid::from_raw(
        i32::try_from(
            child
                .0
                .as_ref()
                .ok_or(TerminalTmuxLaunchError::Process)?
                .id(),
        )
        .map_err(process_error)?,
    )
    .ok_or(TerminalTmuxLaunchError::Process)?;
    rustix::termios::tcsetpgrp(std::io::stdin(), pid).map_err(process_error)?;
    write_gate(&mut child_gate, &[COMMIT], deadline, &cancellation).map_err(gate_error)?;
    drop(child_gate);
    supervise_child(&mut child, &mut channel)
}

fn copy_capture(channel: &mut UnixStream) -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut bytes = [0; CAPTURE_CHUNK];
    loop {
        let count = input.read(&mut bytes).map_err(process_error)?;
        if count == 0 {
            return Ok(());
        }
        // Never block a capture helper behind an unconsumed owner stream:
        // stock tmux otherwise accumulates an unbounded pipe-pane queue.
        // This bounds our buffer, not arbitrary stock-tmux scheduling/RSS.
        channel.write_all(&bytes[..count]).map_err(process_error)?;
    }
}

fn run_shell_child() -> Result<()> {
    use std::os::fd::AsFd;
    let deadline = Instant::now() + crate::terminal_helper::MAX_STARTUP_TIMEOUT;
    let cancellation = CancellationToken::new();
    let stderr = std::io::stderr();
    let mut gate = crate::terminal_helper::DescriptorIo(stderr.as_fd());
    let frame = read_frame(&mut gate, deadline, &cancellation).map_err(gate_error)?;
    write_gate(&mut gate, &[READY], deadline, &cancellation).map_err(gate_error)?;
    let mut commit = [0];
    read_gate(&mut gate, &mut commit, deadline, &cancellation).map_err(gate_error)?;
    if commit != [COMMIT] {
        return Err(TerminalTmuxLaunchError::Protocol);
    }
    let error_output =
        rustix::io::fcntl_dupfd_cloexec(std::io::stdout(), 3).map_err(process_error)?;
    let mut command = Command::new(frame.program);
    command
        .args(frame.arguments)
        .env_clear()
        .envs(frame.environment.entries().iter().cloned())
        .stderr(Stdio::from(error_output));
    Err(process_error(command.exec()))
}

fn supervise_child(child: &mut ProvisionalChild, channel: &mut UnixStream) -> Result<()> {
    use std::os::unix::process::ExitStatusExt;
    let status = loop {
        if let Some(status) = child
            .0
            .as_mut()
            .ok_or(TerminalTmuxLaunchError::Process)?
            .try_wait()
            .map_err(process_error)?
        {
            child.0.take();
            break status;
        }
        let mut byte = [0];
        match channel.read(&mut byte) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            _ => return Err(TerminalTmuxLaunchError::Protocol),
        }
        std::thread::sleep(PAUSE);
    };
    let outcome = match (status.code(), status.signal()) {
        (Some(code @ 0..=255), None) => [b'O', 0, u8::try_from(code).map_err(process_error)?],
        (None, Some(signal @ 1..=127)) => [b'O', 1, u8::try_from(signal).map_err(process_error)?],
        _ => return Err(TerminalTmuxLaunchError::Protocol),
    };
    write_gate(
        channel,
        &outcome,
        Instant::now() + Duration::from_secs(2),
        &CancellationToken::new(),
    )
    .map_err(gate_error)?;
    // The owner retains this exact session anchor until all non-helper jobs
    // are gone. Job completion and helper lifetime are deliberately separate.
    channel.set_nonblocking(false).map_err(process_error)?;
    let mut byte = [0];
    match channel.read(&mut byte) {
        Ok(0) => Ok(()),
        _ => Err(TerminalTmuxLaunchError::Protocol),
    }
}

struct ProvisionalChild(Option<Child>);
impl ProvisionalChild {
    fn terminate(&mut self) -> Result<()> {
        if let Some(child) = self.0.as_mut() {
            if child.try_wait().map_err(process_error)?.is_none() {
                child.kill().map_err(process_error)?;
            }
            child.wait().map_err(process_error)?;
            self.0.take();
        }
        Ok(())
    }
}
impl Drop for ProvisionalChild {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}
pub(crate) fn set_echo(fd: impl rustix::fd::AsFd, enabled: bool) -> Result<()> {
    let mut termios = rustix::termios::tcgetattr(&fd).map_err(process_error)?;
    termios
        .local_modes
        .set(rustix::termios::LocalModes::ECHO, enabled);
    rustix::termios::tcsetattr(fd, rustix::termios::OptionalActions::Now, &termios)
        .map_err(process_error)
}

struct RecordedFrame<'a> {
    source: &'a mut UnixStream,
    bytes: Vec<u8>,
}
impl Read for RecordedFrame<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let remaining = crate::terminal_helper::MAX_FRAME.saturating_sub(self.bytes.len());
        if remaining == 0 && !bytes.is_empty() {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let length = bytes.len().min(remaining);
        let count = self.source.read(&mut bytes[..length])?;
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
}
