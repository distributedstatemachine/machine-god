//! Non-output startup receipts and explicitly acknowledged initial commands.
//!
//! Preparation is blocking and belongs on the host's owned worker. After commit,
//! the backend stays in the session while its separate control handle performs
//! bounded nonblocking protocol steps. The owner persists each event before ACK.

use std::ffi::OsString;
use std::fmt;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use machine_god_core::{CancellationToken, TerminalDimensions, TerminalSignal};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{AtFlags, FileType, Mode, OFlags};

use crate::background_input::{
    BackgroundInputReceipt, BackgroundInputStatus, MAX_BACKGROUND_INPUT_BYTES,
};
use crate::terminal_pty::{
    PreparedTerminalPty, TerminalPty, TerminalPtyClose, TerminalPtyDimensions, TerminalPtyHelper,
    TerminalPtyRead, TerminalPtyRequest, TerminalPtyStatus,
};
use crate::terminal_session::TerminalSessionBackend;
use crate::terminal_shell::TerminalShell;

const BLOCKED: u8 = 0;
const RELEASED: u8 = 1;
const FAILED: u8 = 2;
const FRAME_BYTES: usize = 34;
pub(crate) const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
// Single quoting expands each byte by at most four. There are at most two
// marker invocations, each with 8 KiB combined executable/argv bytes and a
// bounded socket-directory path; 4 KiB covers quotes, separators and protocol
// environment names. Command bytes occur exactly once in the bootstrap.
const MAX_BOOTSTRAP_BYTES: usize =
    4 * machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES + 2 * 4 * (8192 + 104) + 4096;
pub(crate) const STARTUP_MARKER_ARGUMENT: &str = "--machine-god-terminal-startup-marker";
const MAX_SOCKET_PATH_BYTES: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalStartupError {
    InvalidRequest,
    Cancelled,
    Timeout,
    Protocol,
    Process,
    Closed,
}
impl fmt::Display for TerminalStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal shell startup failed")
    }
}
impl std::error::Error for TerminalStartupError {}
type Result<T> = std::result::Result<T, TerminalStartupError>;
fn process_error(_: impl fmt::Debug) -> TerminalStartupError {
    TerminalStartupError::Process
}

pub(crate) struct TerminalStartupRequest {
    pub(crate) shell: TerminalShell,
    pub(crate) command: Option<String>,
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) cwd: OwnedFd,
    /// Retained owner-only directory, with its exact canonical short path.
    pub(crate) artifacts: OwnedFd,
    pub(crate) artifact_path: PathBuf,
    pub(crate) marker_helper: TerminalPtyHelper,
    pub(crate) dimensions: TerminalPtyDimensions,
    pub(crate) timeout: Duration,
}

pub(crate) struct PreparedTerminalStartup {
    pty: PreparedTerminalPty,
    nonce: [u8; 32],
    has_command: bool,
    timeout: Duration,
    listener: UnixListener,
    artifacts: StartupArtifacts,
}
impl PreparedTerminalStartup {
    pub(crate) fn prepare(
        helper: &TerminalPtyHelper,
        request: TerminalStartupRequest,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        if cancellation.is_cancelled() {
            return Err(TerminalStartupError::Cancelled);
        }
        if request.timeout.is_zero()
            || request.timeout > MAX_STARTUP_TIMEOUT
            || request.command.as_ref().is_some_and(|command| {
                command.is_empty()
                    || command.len() > machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES
                    || command.contains('\0')
            })
        {
            return Err(TerminalStartupError::InvalidRequest);
        }
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(process_error)?;
        let mut nonce = [0; 32];
        for (index, byte) in random.into_iter().enumerate() {
            nonce[index * 2] = b"0123456789abcdef"[usize::from(byte >> 4)];
            nonce[index * 2 + 1] = b"0123456789abcdef"[usize::from(byte & 15)];
        }
        let mut artifacts = StartupArtifacts::new(request.artifacts, request.artifact_path, nonce)?;
        let bootstrap = bootstrap_script(
            &artifacts,
            &request.marker_helper,
            request.command.as_deref(),
        )?;
        let program = request
            .shell
            .program()
            .to_str()
            .ok_or(TerminalStartupError::InvalidRequest)?
            .to_owned();
        let source = format!(
            ". {}\n",
            quote(
                artifacts
                    .script_path()
                    .to_str()
                    .ok_or(TerminalStartupError::InvalidRequest)?
            )
        );
        if source.len() > 512 {
            return Err(TerminalStartupError::InvalidRequest);
        }
        let has_command = request.command.is_some();
        let mut arguments = request.shell.interactive_arguments();
        if has_command {
            arguments.extend(["-c".into(), source.clone()]);
        }
        let mut pty_request = TerminalPtyRequest::new(
            program,
            arguments,
            request.environment,
            request.cwd,
            request.dimensions,
        )
        .map_err(process_error)?;
        if !has_command {
            pty_request = pty_request
                .with_startup_source(source)
                .map_err(process_error)?;
        }
        // All semantic, directory, source, argv and environment checks precede
        // artifact publication. No shell runs before the explicit PTY COMMIT.
        let listener = artifacts.publish(&bootstrap)?;
        let pty =
            PreparedTerminalPty::prepare(helper, pty_request, cancellation).map_err(|error| {
                if error.kind() == crate::terminal_pty::TerminalPtyErrorKind::Cancelled {
                    TerminalStartupError::Cancelled
                } else {
                    process_error(error)
                }
            })?;
        Ok(Self {
            pty,
            nonce,
            has_command,
            timeout: request.timeout,
            listener,
            artifacts,
        })
    }

    pub(crate) fn commit(
        self,
        cancellation: &CancellationToken,
    ) -> Result<(TerminalStartupBackend, TerminalStartupControl)> {
        let pty = self.pty.commit(cancellation).map_err(|error| {
            if error.kind() == crate::terminal_pty::TerminalPtyErrorKind::Cancelled {
                TerminalStartupError::Cancelled
            } else {
                process_error(error)
            }
        })?;
        let latch = Arc::new(AtomicU8::new(BLOCKED));
        let artifacts = Arc::new(Mutex::new(ArtifactRetirement {
            artifacts: Some(self.artifacts),
            failed: false,
        }));
        let backend = TerminalStartupBackend {
            artifacts: Arc::clone(&artifacts),
            pty,
            latch: Arc::clone(&latch),
            echo_disabled: !self.has_command,
        };
        let control = TerminalStartupControl {
            listener: Some(self.listener),
            channel: None,
            artifacts: Some(artifacts),
            nonce: self.nonce,
            has_command: self.has_command,
            phase: Phase::Shell,
            bytes: [0; FRAME_BYTES],
            used: 0,
            ack_used: 0,
            deadline: Instant::now() + self.timeout,
            latch,
        };
        Ok((backend, control))
    }
}

/// Owns the child and PTY. No startup-control handle grants process authority.
pub(crate) struct TerminalStartupBackend {
    pty: TerminalPty,
    latch: Arc<AtomicU8>,
    echo_disabled: bool,
    artifacts: Arc<Mutex<ArtifactRetirement>>,
}
impl TerminalStartupBackend {
    pub(crate) fn pid(&self) -> std::num::NonZeroU32 {
        self.pty.pid()
    }
    /// Owner hook, called for a validated `ShellReady` before its ACK. This does
    /// not release an initial command or grant ordinary input admission.
    pub(crate) fn restore_startup_echo(&mut self) -> Result<()> {
        if self.echo_disabled {
            self.pty.restore_startup_echo().map_err(process_error)?;
            self.echo_disabled = false;
        }
        Ok(())
    }
}
impl TerminalSessionBackend for TerminalStartupBackend {
    fn restore_startup_echo(&mut self) -> std::result::Result<(), ()> {
        self.restore_startup_echo().map_err(|_| ())
    }
    fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
        self.pty.read(buffer).map_err(|_| ())
    }
    fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
        if bytes.is_empty() || bytes.len() > MAX_BACKGROUND_INPUT_BYTES {
            return Err(());
        }
        match self.latch.load(Ordering::Acquire) {
            RELEASED => self.pty.write(bytes).map_err(|_| ()),
            BLOCKED => Ok(BackgroundInputReceipt::new(
                0,
                false,
                BackgroundInputStatus::Backpressure,
            )),
            _ => Ok(BackgroundInputReceipt::new(
                0,
                true,
                BackgroundInputStatus::Closed,
            )),
        }
    }
    fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
        // The owner's existing lost-session path quiesces the child and drains
        // output under its persistence authority. Do not discard that tail here.
        if self.latch.load(Ordering::Acquire) == FAILED {
            return Err(());
        }
        self.pty.status().map_err(|_| ())
    }
    fn resize(&mut self, dimensions: &TerminalDimensions) -> std::result::Result<(), ()> {
        TerminalSessionBackend::resize(&mut self.pty, dimensions)
    }
    fn signal(&mut self, signal: TerminalSignal) -> std::result::Result<(), ()> {
        TerminalSessionBackend::signal(&mut self.pty, signal)
    }
    fn signal_may_discard_output(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn close(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
    ) -> std::result::Result<TerminalPtyClose, ()> {
        self.latch.store(FAILED, Ordering::Release);
        let closed = self.pty.close_with_output(force, output).map_err(|_| ());
        self.artifacts
            .lock()
            .map_err(|_| ())?
            .cleanup()
            .map_err(|_| ())?;
        closed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalStartupEvent {
    ShellReady,
    CommandStarted,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Shell,
    ShellAcknowledgement,
    Command,
    CommandAcknowledgement,
    Complete,
    Failed,
}

/// The owner advances this channel independently of output pumping. Its Drop
/// closes startup admission; it never reconstructs or owns process authority.
pub(crate) struct TerminalStartupControl {
    listener: Option<UnixListener>,
    channel: Option<UnixStream>,
    artifacts: Option<Arc<Mutex<ArtifactRetirement>>>,
    nonce: [u8; 32],
    has_command: bool,
    phase: Phase,
    bytes: [u8; FRAME_BYTES],
    used: usize,
    ack_used: usize,
    deadline: Instant,
    latch: Arc<AtomicU8>,
}
impl TerminalStartupControl {
    pub(crate) fn is_complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub(crate) fn poll(
        &mut self,
        now: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Option<TerminalStartupEvent>> {
        self.check(now, cancellation)?;
        let expected = match self.phase {
            Phase::Shell => b'R',
            Phase::Command => b'C',
            Phase::ShellAcknowledgement | Phase::CommandAcknowledgement | Phase::Complete => {
                return Ok(None);
            }
            Phase::Failed => return Err(TerminalStartupError::Closed),
        };
        if self.channel.is_none() {
            match self
                .listener
                .as_ref()
                .ok_or(TerminalStartupError::Closed)?
                .accept()
            {
                Ok((channel, _)) => {
                    if channel.set_nonblocking(true).is_err() {
                        return self.fail(TerminalStartupError::Protocol);
                    }
                    self.channel = Some(channel);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    return Ok(None);
                }
                Err(_) => return self.fail(TerminalStartupError::Protocol),
            }
        }
        for _ in 0..4 {
            let read = self
                .channel
                .as_mut()
                .ok_or(TerminalStartupError::Closed)?
                .read(&mut self.bytes[self.used..]);
            match read {
                Ok(0) => return self.fail(TerminalStartupError::Protocol),
                Ok(count) => {
                    self.used += count;
                    if self.used == FRAME_BYTES {
                        if self.bytes[..32] != self.nonce
                            || self.bytes[32] != expected
                            || self.bytes[33] != b'\n'
                        {
                            return self.fail(TerminalStartupError::Protocol);
                        }
                        self.used = 0;
                        self.phase = if expected == b'R' {
                            Phase::ShellAcknowledgement
                        } else {
                            Phase::CommandAcknowledgement
                        };
                        return Ok(Some(if expected == b'R' {
                            TerminalStartupEvent::ShellReady
                        } else {
                            TerminalStartupEvent::CommandStarted
                        }));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return self.fail(TerminalStartupError::Protocol),
            }
        }
        Ok(None)
    }

    /// Call only after publishing `ShellReady` and restoring startup echo through
    /// the backend hook. False means bounded socket backpressure; retry later.
    pub(crate) fn acknowledge_shell_ready(
        &mut self,
        now: Instant,
        cancellation: &CancellationToken,
    ) -> Result<bool> {
        self.acknowledge(Phase::ShellAcknowledgement, *b"R\n", now, cancellation)
    }
    /// Call only after publishing `CommandStarted` and committing its authorization.
    pub(crate) fn release_command(
        &mut self,
        now: Instant,
        cancellation: &CancellationToken,
    ) -> Result<bool> {
        self.acknowledge(Phase::CommandAcknowledgement, *b"C\n", now, cancellation)
    }
    fn acknowledge(
        &mut self,
        phase: Phase,
        bytes: [u8; 2],
        now: Instant,
        cancellation: &CancellationToken,
    ) -> Result<bool> {
        self.check(now, cancellation)?;
        if self.phase != phase {
            return Err(TerminalStartupError::InvalidRequest);
        }
        for _ in 0..4 {
            let write = self
                .channel
                .as_mut()
                .ok_or(TerminalStartupError::Closed)?
                .write(&bytes[self.ack_used..]);
            match write {
                Ok(0) => return self.fail(TerminalStartupError::Protocol),
                Ok(count) => {
                    self.ack_used += count;
                    if self.ack_used == bytes.len() {
                        self.ack_used = 0;
                        self.channel.take();
                        if phase == Phase::ShellAcknowledgement && self.has_command {
                            self.phase = Phase::Command;
                        } else {
                            self.phase = Phase::Complete;
                            self.latch.store(RELEASED, Ordering::Release);
                            self.listener.take();
                        }
                        return Ok(true);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return self.fail(TerminalStartupError::Protocol),
            }
        }
        Ok(false)
    }
    fn check(&mut self, now: Instant, cancellation: &CancellationToken) -> Result<()> {
        if self.phase == Phase::Complete {
            return Ok(());
        }
        if self.phase == Phase::Failed || self.latch.load(Ordering::Acquire) == FAILED {
            return self.fail(TerminalStartupError::Closed);
        }
        if cancellation.is_cancelled() {
            return self.fail(TerminalStartupError::Cancelled);
        }
        if now >= self.deadline {
            return self.fail(TerminalStartupError::Timeout);
        }
        Ok(())
    }
    fn fail<T>(&mut self, error: TerminalStartupError) -> Result<T> {
        self.phase = Phase::Failed;
        self.latch.store(FAILED, Ordering::Release);
        self.channel.take();
        self.listener.take();
        Err(error)
    }
    pub(crate) fn cleanup_failed(&self) -> bool {
        self.artifacts
            .as_ref()
            .is_some_and(|artifacts| artifacts.lock().map_or(true, |artifacts| artifacts.failed))
    }
    /// Run on the owning worker. Retirement is independent of an already
    /// committed ACK; backend close retains and retries the same obligation.
    pub(crate) fn retry_cleanup(&mut self) -> Result<()> {
        if !matches!(self.phase, Phase::Complete | Phase::Failed) {
            return Err(TerminalStartupError::InvalidRequest);
        }
        if let Some(artifacts) = self.artifacts.as_ref() {
            artifacts.lock().map_err(process_error)?.cleanup()?;
        }
        Ok(())
    }
}
impl Drop for TerminalStartupControl {
    fn drop(&mut self) {
        if self.phase != Phase::Complete {
            self.latch.store(FAILED, Ordering::Release);
        }
        self.channel.take();
        self.listener.take();
    }
}

impl Drop for TerminalStartupBackend {
    fn drop(&mut self) {
        self.latch.store(FAILED, Ordering::Release);
        let _ = self.pty.close_with_output(true, &mut |_: &[u8]| {});
        if let Ok(mut artifacts) = self.artifacts.lock() {
            let _ = artifacts.cleanup();
        }
    }
}
struct ArtifactRetirement {
    artifacts: Option<StartupArtifacts>,
    failed: bool,
}
impl ArtifactRetirement {
    fn cleanup(&mut self) -> Result<()> {
        if let Some(artifacts) = self.artifacts.as_mut()
            && let Err(error) = artifacts.cleanup()
        {
            self.failed = true;
            return Err(error);
        }
        self.artifacts.take();
        self.failed = false;
        Ok(())
    }
}
struct StartupArtifacts {
    directory: OwnedFd,
    path: PathBuf,
    identity: String,
    nonce: [u8; 32],
    script_name: String,
    socket_name: String,
    script_identity: Option<String>,
    socket_identity: Option<String>,
}
fn identity(stat: &rustix::fs::Stat) -> String {
    format!("{}:{}", stat.st_dev, stat.st_ino)
}
fn validate_directory(directory: &impl AsFd, path: &Path) -> Result<String> {
    let stat = rustix::fs::fstat(directory).map_err(process_error)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_mode & 0o077 != 0
        || stat.st_uid != rustix::process::getuid().as_raw()
    {
        return Err(TerminalStartupError::InvalidRequest);
    }
    let reopened = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(process_error)?;
    if identity(&stat) != identity(&rustix::fs::fstat(reopened).map_err(process_error)?) {
        return Err(TerminalStartupError::InvalidRequest);
    }
    Ok(identity(&stat))
}
impl StartupArtifacts {
    fn new(directory: OwnedFd, path: PathBuf, nonce: [u8; 32]) -> Result<Self> {
        let text = path.to_str().ok_or(TerminalStartupError::InvalidRequest)?;
        if !path.is_absolute()
            || text.chars().any(char::is_control)
            || std::fs::canonicalize(&path).map_err(process_error)? != path
        {
            return Err(TerminalStartupError::InvalidRequest);
        }
        let nonce_text = std::str::from_utf8(&nonce).map_err(process_error)?;
        let script_name = format!("b-{nonce_text}");
        let socket_name = format!("s-{nonce_text}");
        if path.join(&socket_name).as_os_str().as_bytes().len() > MAX_SOCKET_PATH_BYTES {
            return Err(TerminalStartupError::InvalidRequest);
        }
        let identity = validate_directory(&directory, &path)?;
        Ok(Self {
            directory,
            path,
            identity,
            nonce,
            script_name,
            socket_name,
            script_identity: None,
            socket_identity: None,
        })
    }
    fn script_path(&self) -> PathBuf {
        self.path.join(&self.script_name)
    }
    fn publish(&mut self, bootstrap: &str) -> Result<UnixListener> {
        if validate_directory(&self.directory, &self.path)? != self.identity {
            return Err(TerminalStartupError::InvalidRequest);
        }
        let writer = rustix::fs::openat(
            &self.directory,
            self.script_name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(process_error)?;
        self.script_identity = Some(identity(
            &rustix::fs::fstat(&writer).map_err(process_error)?,
        ));
        std::fs::File::from(writer)
            .write_all(bootstrap.as_bytes())
            .map_err(process_error)?;
        let listener =
            UnixListener::bind(self.path.join(&self.socket_name)).map_err(process_error)?;
        let socket = rustix::fs::statat(
            &self.directory,
            self.socket_name.as_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(process_error)?;
        if FileType::from_raw_mode(socket.st_mode) != FileType::Socket {
            return Err(TerminalStartupError::InvalidRequest);
        }
        self.socket_identity = Some(identity(&socket));
        if validate_directory(&self.directory, &self.path)? != self.identity {
            return Err(TerminalStartupError::InvalidRequest);
        }
        listener.set_nonblocking(true).map_err(process_error)?;
        Ok(listener)
    }
    fn cleanup(&mut self) -> Result<()> {
        for (name, expected) in [
            (&self.script_name, &mut self.script_identity),
            (&self.socket_name, &mut self.socket_identity),
        ] {
            let Some(expected_identity) = expected.as_ref() else {
                continue;
            };
            match rustix::fs::statat(&self.directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) if identity(&stat) == *expected_identity => {
                    rustix::fs::unlinkat(&self.directory, name.as_str(), AtFlags::empty())
                        .map_err(process_error)?;
                }
                Err(rustix::io::Errno::NOENT) => {}
                _ => return Err(TerminalStartupError::Process),
            }
            *expected = None;
        }
        Ok(())
    }
}
impl Drop for StartupArtifacts {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn bootstrap_script(
    artifacts: &StartupArtifacts,
    helper: &TerminalPtyHelper,
    command: Option<&str>,
) -> Result<String> {
    let program = helper
        .program()
        .to_str()
        .ok_or(TerminalStartupError::InvalidRequest)?;
    if program.len() > 4096
        || helper
            .arguments()
            .iter()
            .map(|arg| arg.as_bytes().len())
            .sum::<usize>()
            > 4096
        || command.is_some_and(|command| {
            command.is_empty()
                || command.len() > machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES
                || command.contains('\0')
        })
    {
        return Err(TerminalStartupError::InvalidRequest);
    }
    let mut invocation = quote(program);
    for argument in helper.arguments() {
        invocation.push(' ');
        invocation.push_str(&quote(
            argument
                .to_str()
                .ok_or(TerminalStartupError::InvalidRequest)?,
        ));
    }
    let nonce = std::str::from_utf8(&artifacts.nonce).map_err(process_error)?;
    let directory = artifacts
        .path
        .to_str()
        .ok_or(TerminalStartupError::InvalidRequest)?;
    let mut output = String::from("set +x; ");
    for marker in if command.is_some() {
        &["R", "C"][..]
    } else {
        &["R"][..]
    } {
        write!(output, "builtin command /usr/bin/env -i LANG=C MACHINE_GOD_STARTUP_MARKER=1 MACHINE_GOD_STARTUP_DIRECTORY={} MACHINE_GOD_STARTUP_ID={} MACHINE_GOD_STARTUP_NONCE={} MACHINE_GOD_STARTUP_KIND={} {} || exit 125; ", quote(directory), quote(&artifacts.identity), quote(nonce), quote(marker), invocation).map_err(process_error)?;
    }
    if let Some(command) = command {
        output.push_str("builtin eval -- ");
        output.push_str(&quote(command));
        output.push_str("; _machine_god_status=$?; exit \"$_machine_god_status\"\n");
    } else {
        output.push('\n');
    }
    if output.len() > MAX_BOOTSTRAP_BYTES {
        return Err(TerminalStartupError::InvalidRequest);
    }
    Ok(output)
}

/// Private executable mode only: its isolated process may change cwd, never
/// the multithreaded owner. It holds no PTY/process cleanup authority.
pub(crate) fn run_terminal_startup_marker() -> Result<()> {
    let value = |key: &str, limit: usize| -> Result<String> {
        let value = std::env::var(key).map_err(process_error)?;
        if value.is_empty() || value.len() > limit || value.contains('\0') {
            return Err(TerminalStartupError::InvalidRequest);
        }
        Ok(value)
    };
    if value("MACHINE_GOD_STARTUP_MARKER", 1)? != "1" {
        return Err(TerminalStartupError::InvalidRequest);
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
        return Err(TerminalStartupError::InvalidRequest);
    }
    let directory = rustix::fs::open(
        &path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(process_error)?;
    if validate_directory(&directory, &path)? != expected {
        return Err(TerminalStartupError::InvalidRequest);
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
        return Err(TerminalStartupError::Protocol);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU64;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = PathBuf::from("/tmp").join(format!(
                "machine-god-startup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn fd(&self) -> OwnedFd {
            rustix::fs::open(
                &self.0,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap()
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn marker_helper_entry() {
        if std::env::var("MACHINE_GOD_STARTUP_MARKER").as_deref() == Ok("1") {
            run_terminal_startup_marker().unwrap();
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
    fn request(
        cwd: &Directory,
        artifacts: &Directory,
        shell: &str,
        clean: bool,
        command: Option<String>,
    ) -> TerminalStartupRequest {
        TerminalStartupRequest {
            shell: TerminalShell::from_executable(Path::new(shell), clean).unwrap(),
            command,
            environment: vec![
                ("HOME".into(), cwd.0.as_os_str().to_owned()),
                ("LANG".into(), "C".into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TERM".into(), "xterm-256color".into()),
            ],
            cwd: cwd.fd(),
            artifacts: artifacts.fd(),
            artifact_path: std::fs::canonicalize(&artifacts.0).unwrap(),
            marker_helper: TerminalPtyHelper::new(
                std::env::current_exe().unwrap(),
                vec![
                    "--exact".into(),
                    "terminal_startup::tests::marker_helper_entry".into(),
                    "--test-threads=1".into(),
                    "--quiet".into(),
                ],
            )
            .unwrap(),
            dimensions: TerminalPtyDimensions {
                rows: 24,
                columns: 80,
            },
            timeout: DEFAULT_STARTUP_TIMEOUT,
        }
    }
    fn start(request: TerminalStartupRequest) -> (TerminalStartupBackend, TerminalStartupControl) {
        PreparedTerminalStartup::prepare(&helper(), request, &CancellationToken::new())
            .unwrap()
            .commit(&CancellationToken::new())
            .unwrap()
    }
    fn event(
        backend: &mut TerminalStartupBackend,
        control: &mut TerminalStartupControl,
        expected: TerminalStartupEvent,
        output: &mut Vec<u8>,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut chunk = [0; 4096];
            let read = backend.read(&mut chunk).unwrap();
            output.extend_from_slice(&chunk[..read.bytes_read]);
            assert!(output.len() < 1024 * 1024);
            match control.poll(Instant::now(), &CancellationToken::new()) {
                Ok(Some(observed)) => {
                    assert_eq!(observed, expected);
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    std::thread::sleep(Duration::from_millis(50));
                    let mut chunk = [0; 4096];
                    let read = backend.read(&mut chunk).unwrap();
                    output.extend_from_slice(&chunk[..read.bytes_read]);
                    panic!(
                        "startup error {error:?}; status {:?}; used {}; output {}",
                        backend.pty.status(),
                        control.used,
                        String::from_utf8_lossy(output)
                    );
                }
            }
            assert!(
                Instant::now() < deadline,
                "startup marker missing: {}",
                String::from_utf8_lossy(output)
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn shell_ack(backend: &mut TerminalStartupBackend, control: &mut TerminalStartupControl) {
        TerminalSessionBackend::restore_startup_echo(backend).unwrap();
        assert!(
            control
                .acknowledge_shell_ready(Instant::now(), &CancellationToken::new())
                .unwrap()
        );
    }
    fn finish(backend: &mut TerminalStartupBackend, expected: i32, output: &mut Vec<u8>) {
        let pid =
            rustix::process::Pid::from_raw(i32::try_from(backend.pid().get()).unwrap()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut chunk = [0; 4096];
            let read = backend.read(&mut chunk).unwrap();
            output.extend_from_slice(&chunk[..read.bytes_read]);
            assert!(output.len() < 1024 * 1024);
            if backend.status().unwrap() != TerminalPtyStatus::Running {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let closed = backend
            .close(false, &mut |bytes| output.extend_from_slice(bytes))
            .unwrap();
        assert_eq!(closed.status, TerminalPtyStatus::Exited(expected));
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
    }

    #[test]
    fn initial_command_waits_for_both_trusted_receipts_and_owner_release() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(shell).exists() {
                continue;
            }
            for clean in [true, false] {
                let cwd = Directory::new();
                let artifacts = Directory::new();
                let profile = if shell.ends_with("bash") {
                    ".bash_profile"
                } else {
                    ".zprofile"
                };
                std::fs::write(
                    cwd.0.join(profile),
                    "export FROM_PROFILE=user; printf '%s\\n' PROFILE_OUTPUT\nfor fd in 3 4 5 6 7 8 9; do eval \"exec $fd>&-\"; done\n",
                )
                .unwrap();
                let expected = if clean { "unset" } else { "user" };
                let command = format!(
                    "test \"${{FROM_PROFILE-unset}}\" = {expected} || exit 7; printf command > executed; exit 23"
                );
                let (mut backend, mut control) =
                    start(request(&cwd, &artifacts, shell, clean, Some(command)));
                assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 2);
                let mut output = Vec::new();
                assert_eq!(
                    backend.write(b"printf CORRUPT\n").unwrap().status(),
                    BackgroundInputStatus::Backpressure
                );
                assert_eq!(
                    control.release_command(Instant::now(), &CancellationToken::new()),
                    Err(TerminalStartupError::InvalidRequest)
                );
                event(
                    &mut backend,
                    &mut control,
                    TerminalStartupEvent::ShellReady,
                    &mut output,
                );
                assert!(!cwd.0.join("executed").exists());
                assert_eq!(
                    control
                        .poll(Instant::now(), &CancellationToken::new())
                        .unwrap(),
                    None
                );
                shell_ack(&mut backend, &mut control);
                event(
                    &mut backend,
                    &mut control,
                    TerminalStartupEvent::CommandStarted,
                    &mut output,
                );
                assert!(!cwd.0.join("executed").exists());
                assert_eq!(
                    backend.write(b"printf CORRUPT\n").unwrap().bytes_written(),
                    0
                );
                assert!(
                    control
                        .release_command(Instant::now(), &CancellationToken::new())
                        .unwrap()
                );
                assert!(control.is_complete());
                control.retry_cleanup().unwrap();
                assert!(!control.cleanup_failed());
                assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
                finish(&mut backend, 23, &mut output);
                assert_eq!(
                    std::fs::read_to_string(cwd.0.join("executed")).unwrap(),
                    "command"
                );
                assert_eq!(
                    String::from_utf8_lossy(&output).contains("PROFILE_OUTPUT"),
                    !clean
                );
            }
        }
    }

    #[test]
    fn commandless_shell_queues_only_bootstrap_and_preserves_user_input_after_ack() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(shell).exists() {
                continue;
            }
            for clean in [true, false] {
                let cwd = Directory::new();
                let profile = if shell.ends_with("bash") {
                    ".bash_profile"
                } else {
                    ".zprofile"
                };
                std::fs::write(cwd.0.join(profile), "export FROM_PROFILE=user; for fd in 3 4 5 6 7 8 9; do eval \"exec $fd>&-\"; done\n").unwrap();
                let artifacts = Directory::new();
                let (mut backend, mut control) =
                    start(request(&cwd, &artifacts, shell, clean, None));
                let mut output = Vec::new();
                assert_eq!(backend.write(b"bad input\n").unwrap().bytes_written(), 0);
                event(
                    &mut backend,
                    &mut control,
                    TerminalStartupEvent::ShellReady,
                    &mut output,
                );
                assert!(!String::from_utf8_lossy(&output).contains("_machine_god_ack"));
                shell_ack(&mut backend, &mut control);
                assert!(control.is_complete());
                let input = format!(
                    "test \"${{FROM_PROFILE-unset}}\" = {} || exit 7; printf 'user input' > received; exit 0\n",
                    if clean { "unset" } else { "user" }
                );
                let input = input.as_bytes();
                assert_eq!(backend.write(input).unwrap().bytes_written(), input.len());
                finish(&mut backend, 0, &mut output);
                assert_eq!(
                    std::fs::read_to_string(cwd.0.join("received")).unwrap(),
                    "user input"
                );
                assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
            }
        }
    }

    #[test]
    fn full_command_quoting_unicode_and_control_bytes_survive_bootstrap() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let mut command = String::from("printf '%s' \"literal ' \\\" 雪\" > executed; #");
        let padding = "'\\\"雪\u{1}";
        command.push_str(&padding.repeat(
            (machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES - command.len()) / padding.len(),
        ));
        command.push_str(
            &"x".repeat(machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES - command.len()),
        );
        let (mut backend, mut control) =
            start(request(&cwd, &artifacts, "/bin/bash", true, Some(command)));
        let mut output = Vec::new();
        event(
            &mut backend,
            &mut control,
            TerminalStartupEvent::ShellReady,
            &mut output,
        );
        shell_ack(&mut backend, &mut control);
        event(
            &mut backend,
            &mut control,
            TerminalStartupEvent::CommandStarted,
            &mut output,
        );
        assert!(
            control
                .release_command(Instant::now(), &CancellationToken::new())
                .unwrap()
        );
        finish(&mut backend, 0, &mut output);
        assert_eq!(
            std::fs::read_to_string(cwd.0.join("executed")).unwrap(),
            "literal ' \" 雪"
        );
    }

    #[test]
    fn output_cannot_establish_readiness_and_timeout_quiesces_owned_backend() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        std::fs::write(
            cwd.0.join(".bash_profile"),
            "printf 'shell-ready command-started\\n'; while :; do :; done\n",
        )
        .unwrap();
        let (mut backend, mut control) = start(request(
            &cwd,
            &artifacts,
            "/bin/bash",
            false,
            Some("printf BAD > executed".into()),
        ));
        let pid =
            rustix::process::Pid::from_raw(i32::try_from(backend.pid().get()).unwrap()).unwrap();
        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !String::from_utf8_lossy(&output).contains("shell-ready") {
            let mut chunk = [0; 4096];
            let read = backend.read(&mut chunk).unwrap();
            output.extend_from_slice(&chunk[..read.bytes_read]);
            assert_eq!(
                control
                    .poll(Instant::now(), &CancellationToken::new())
                    .unwrap(),
                None
            );
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            control.poll(control.deadline, &CancellationToken::new()),
            Err(TerminalStartupError::Timeout)
        );
        assert!(backend.status().is_err());
        backend.close(true, &mut |_| {}).unwrap();
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
        assert!(!cwd.0.join("executed").exists());
    }

    #[test]
    fn preparation_abort_cancellation_and_control_drop_never_release_command() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let prepared = PreparedTerminalStartup::prepare(
            &helper(),
            request(
                &cwd,
                &artifacts,
                "/bin/bash",
                true,
                Some("printf BAD > executed".into()),
            ),
            &CancellationToken::new(),
        )
        .unwrap();
        drop(prepared);
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
        assert!(!cwd.0.join("executed").exists());
        for cancel in [true, false] {
            let (mut backend, mut control) = start(request(
                &cwd,
                &artifacts,
                "/bin/bash",
                true,
                Some("printf BAD > executed".into()),
            ));
            let pid = rustix::process::Pid::from_raw(i32::try_from(backend.pid().get()).unwrap())
                .unwrap();
            let mut output = Vec::new();
            event(
                &mut backend,
                &mut control,
                TerminalStartupEvent::ShellReady,
                &mut output,
            );
            if cancel {
                let cancellation = CancellationToken::new();
                cancellation.cancel();
                assert_eq!(
                    control.acknowledge_shell_ready(Instant::now(), &cancellation),
                    Err(TerminalStartupError::Cancelled)
                );
            }
            drop(control);
            assert!(backend.status().is_err());
            drop(backend);
            assert_eq!(
                rustix::process::test_kill_process(pid),
                Err(rustix::io::Errno::SRCH)
            );
            assert!(!cwd.0.join("executed").exists());
        }
    }

    #[test]
    fn control_parser_rejects_forged_out_of_order_and_truncated_frames() {
        for payload in [
            vec![b'x'; FRAME_BYTES],
            [b"0123456789abcdef0123456789abcdef".as_slice(), b"C\n"].concat(),
            vec![b'x'; 3],
        ] {
            let (channel, mut peer) = UnixStream::pair().unwrap();
            channel.set_nonblocking(true).unwrap();
            let mut control = TerminalStartupControl {
                listener: None,
                artifacts: None,
                channel: Some(channel),
                nonce: *b"0123456789abcdef0123456789abcdef",
                has_command: true,
                phase: Phase::Shell,
                bytes: [0; FRAME_BYTES],
                used: 0,
                ack_used: 0,
                deadline: Instant::now() + DEFAULT_STARTUP_TIMEOUT,
                latch: Arc::new(AtomicU8::new(BLOCKED)),
            };
            peer.write_all(&payload).unwrap();
            drop(peer);
            assert_eq!(
                control.poll(Instant::now(), &CancellationToken::new()),
                Err(TerminalStartupError::Protocol)
            );
            assert_eq!(control.latch.load(Ordering::Acquire), FAILED);
            assert!(control.channel.is_none());
        }
    }

    #[test]
    fn invalid_command_and_artifact_identity_reject_before_publication() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let other = Directory::new();
        for mode in 0..4 {
            let mut request = request(&cwd, &artifacts, "/bin/bash", true, None);
            match mode {
                0 => {
                    request.command =
                        Some("x".repeat(machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES + 1));
                }
                1 => request.artifact_path = std::fs::canonicalize(&other.0).unwrap(),
                2 => request.timeout = Duration::ZERO,
                _ => {
                    let long = artifacts.0.join("x".repeat(80));
                    std::fs::create_dir(&long).unwrap();
                    std::fs::set_permissions(&long, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    request.artifact_path = std::fs::canonicalize(&long).unwrap();
                    request.artifacts = rustix::fs::open(
                        &long,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .unwrap();
                }
            }
            assert!(matches!(
                PreparedTerminalStartup::prepare(&helper(), request, &CancellationToken::new()),
                Err(TerminalStartupError::InvalidRequest)
            ));
            assert_eq!(
                std::fs::read_dir(&artifacts.0).unwrap().count(),
                usize::from(mode == 3)
            );
            assert_eq!(std::fs::read_dir(&other.0).unwrap().count(), 0);
        }
    }

    #[test]
    fn bootstrap_encoder_bound_includes_worst_case_quoting() {
        let directory = Directory::new();
        let artifacts = StartupArtifacts::new(
            directory.fd(),
            std::fs::canonicalize(&directory.0).unwrap(),
            *b"0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let helper = TerminalPtyHelper::new(
            PathBuf::from(format!("/{}", "'".repeat(4095))),
            vec!["'".repeat(4096).into()],
        )
        .unwrap();
        let script = bootstrap_script(
            &artifacts,
            &helper,
            Some(&"'".repeat(machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES)),
        )
        .unwrap();
        assert!(script.len() <= MAX_BOOTSTRAP_BYTES);
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn backend_retains_failed_artifact_retirement_after_control_drop() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let (mut backend, mut control) = start(request(&cwd, &artifacts, "/bin/bash", true, None));
        let mut output = Vec::new();
        event(
            &mut backend,
            &mut control,
            TerminalStartupEvent::ShellReady,
            &mut output,
        );
        shell_ack(&mut backend, &mut control);
        let script_name = backend
            .artifacts
            .lock()
            .unwrap()
            .artifacts
            .as_ref()
            .unwrap()
            .script_name
            .clone();
        let script = artifacts.0.join(script_name);
        let retained = artifacts.0.join("retained-bootstrap");
        std::fs::rename(&script, &retained).unwrap();
        std::fs::write(&script, "unrelated replacement").unwrap();
        assert!(control.retry_cleanup().is_err());
        assert!(control.cleanup_failed());
        assert!(control.is_complete());
        drop(control);
        assert!(backend.close(true, &mut |_| {}).is_err());
        assert_eq!(
            std::fs::read_to_string(&script).unwrap(),
            "unrelated replacement"
        );
        std::fs::remove_file(&script).unwrap();
        std::fs::rename(&retained, &script).unwrap();
        backend.close(true, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
    }

    #[test]
    fn cancelled_commit_and_command_phase_timeout_collect_without_execution() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let prepared = PreparedTerminalStartup::prepare(
            &helper(),
            request(
                &cwd,
                &artifacts,
                "/bin/bash",
                true,
                Some("printf BAD > executed".into()),
            ),
            &CancellationToken::new(),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            prepared.commit(&cancellation),
            Err(TerminalStartupError::Cancelled)
        ));
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
        let (mut backend, mut control) = start(request(
            &cwd,
            &artifacts,
            "/bin/bash",
            true,
            Some("printf BAD > executed".into()),
        ));
        let mut output = Vec::new();
        event(
            &mut backend,
            &mut control,
            TerminalStartupEvent::ShellReady,
            &mut output,
        );
        shell_ack(&mut backend, &mut control);
        event(
            &mut backend,
            &mut control,
            TerminalStartupEvent::CommandStarted,
            &mut output,
        );
        assert_eq!(
            control.release_command(control.deadline, &CancellationToken::new()),
            Err(TerminalStartupError::Timeout)
        );
        let pid =
            rustix::process::Pid::from_raw(i32::try_from(backend.pid().get()).unwrap()).unwrap();
        backend.close(true, &mut |_| {}).unwrap();
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
        assert!(!cwd.0.join("executed").exists());
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
    }
}
