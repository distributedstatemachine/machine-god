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
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use machine_god_core::{CancellationToken, TerminalDimensions, TerminalSignal};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};

use crate::background_input::{BackgroundInputReceipt, BackgroundInputStatus};
#[cfg(test)]
use crate::terminal_helper::run_terminal_startup_marker;
use crate::terminal_helper::{
    MAX_STARTUP_PATH_BYTES, MAX_STARTUP_TIMEOUT, TerminalHelperError, TerminalHelperErrorKind,
    startup_directory_identity as identity, validate_startup_directory as validate_directory,
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
#[cfg(test)]
pub(crate) const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
// Single quoting expands each byte by at most four. There are at most two
// marker invocations, each with 8 KiB combined executable/argv bytes and a
// bounded socket-directory path; 4 KiB covers quotes, separators and protocol
// environment names. Command bytes occur exactly once in the bootstrap.
const MAX_BOOTSTRAP_BYTES: usize = 4 * machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES
    + 2 * 4 * (8192 + MAX_STARTUP_PATH_BYTES)
    + 4096;

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
impl From<TerminalHelperError> for TerminalStartupError {
    fn from(error: TerminalHelperError) -> Self {
        match error.kind {
            TerminalHelperErrorKind::InvalidRequest => Self::InvalidRequest,
            TerminalHelperErrorKind::Cancelled => Self::Cancelled,
            TerminalHelperErrorKind::Timeout => Self::Timeout,
            TerminalHelperErrorKind::Protocol => Self::Protocol,
            TerminalHelperErrorKind::Process => Self::Process,
        }
    }
}
type Result<T> = std::result::Result<T, TerminalStartupError>;
fn process_error(_: impl fmt::Debug) -> TerminalStartupError {
    TerminalStartupError::Process
}

pub(crate) struct TerminalStartupRequest {
    pub(crate) shell: TerminalShell,
    pub(crate) command: Option<String>,
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) cwd: OwnedFd,
    /// Retained owner-only directory, with its exact canonical path.
    pub(crate) artifacts: OwnedFd,
    pub(crate) artifact_path: PathBuf,
    pub(crate) marker_helper: TerminalPtyHelper,
    pub(crate) dimensions: TerminalPtyDimensions,
    pub(crate) timeout: Duration,
}

pub(crate) struct PreparedTerminalStartup {
    pty: PreparedTerminalPty,
    bootstrap: PublishedTerminalBootstrap,
}

/// Transport-neutral bootstrap, validated before any artifact publication.
/// Native PTY and tmux use the same authenticated marker and durable ACK protocol.
pub(crate) struct PreparedTerminalBootstrap {
    program: String,
    arguments: Vec<String>,
    source: Option<String>,
    script: String,
    nonce: [u8; 32],
    deadline: Instant,
    artifacts: StartupArtifacts,
    marker: TerminalPtyHelper,
}

pub(crate) struct PublishedTerminalBootstrap {
    nonce: [u8; 32],
    has_command: bool,
    deadline: Instant,
    listener: UnixListener,
    artifacts: StartupArtifacts,
}
impl PreparedTerminalStartup {
    #[cfg(test)]
    pub(crate) fn prepare(
        helper: &TerminalPtyHelper,
        request: TerminalStartupRequest,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        let started = Instant::now();
        if cancellation.is_cancelled() {
            return Err(TerminalStartupError::Cancelled);
        }
        let deadline = started
            .checked_add(request.timeout)
            .ok_or(TerminalStartupError::InvalidRequest)?;
        Self::prepare_until(helper, request, deadline, cancellation)
    }

    /// Shares the caller's absolute worker deadline across preparation, commit
    /// and startup acknowledgements without granting fresh time at this seam.
    pub(crate) fn prepare_until(
        helper: &TerminalPtyHelper,
        request: TerminalStartupRequest,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        let started = Instant::now();
        if cancellation.is_cancelled() {
            return Err(TerminalStartupError::Cancelled);
        }
        if request.timeout.is_zero()
            || request.timeout > MAX_STARTUP_TIMEOUT
            || deadline.saturating_duration_since(started) > MAX_STARTUP_TIMEOUT
            || request.command.as_ref().is_some_and(|command| {
                command.is_empty()
                    || command.len() > machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES
                    || command.contains('\0')
            })
        {
            return Err(TerminalStartupError::InvalidRequest);
        }
        let deadline = deadline.min(started + request.timeout);
        let bootstrap = PreparedTerminalBootstrap::new(
            &request.shell,
            request.command.as_deref(),
            request.artifacts,
            request.artifact_path,
            &request.marker_helper,
            deadline,
            cancellation,
        )?;
        let mut pty_request = TerminalPtyRequest::new(
            bootstrap.program().to_owned(),
            bootstrap.arguments().to_vec(),
            request.environment,
            request.cwd,
            request.dimensions,
        )
        .map_err(process_error)?;
        if let Some(source) = bootstrap.startup_source() {
            pty_request = pty_request
                .with_startup_source(source.to_owned())
                .map_err(process_error)?;
        }
        // All semantic, directory, source, argv and environment checks precede
        // artifact publication. No shell runs before the explicit PTY COMMIT.
        let bootstrap = bootstrap.publish(cancellation)?;
        let pty = PreparedTerminalPty::prepare_until(helper, pty_request, deadline, cancellation)
            .map_err(|error| {
            if error.kind() == crate::terminal_pty::TerminalPtyErrorKind::Cancelled {
                TerminalStartupError::Cancelled
            } else if error.kind() == crate::terminal_pty::TerminalPtyErrorKind::Timeout {
                TerminalStartupError::Timeout
            } else {
                process_error(error)
            }
        })?;
        Ok(Self { pty, bootstrap })
    }

    pub(crate) fn commit(
        self,
        cancellation: &CancellationToken,
    ) -> Result<(TerminalStartupBackend, TerminalStartupControl)> {
        let pty = self.pty.commit(cancellation).map_err(|error| {
            if error.kind() == crate::terminal_pty::TerminalPtyErrorKind::Cancelled {
                TerminalStartupError::Cancelled
            } else if error.kind() == crate::terminal_pty::TerminalPtyErrorKind::Timeout {
                TerminalStartupError::Timeout
            } else {
                process_error(error)
            }
        })?;
        Ok(self.bootstrap.attach(pty))
    }
}

impl PreparedTerminalBootstrap {
    pub(crate) fn new(
        shell: &TerminalShell,
        command: Option<&str>,
        directory: OwnedFd,
        path: PathBuf,
        marker: &TerminalPtyHelper,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        crate::terminal_helper::check_deadline(deadline, cancellation)?;
        if deadline.saturating_duration_since(Instant::now()) > MAX_STARTUP_TIMEOUT {
            return Err(TerminalStartupError::InvalidRequest);
        }
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(process_error)?;
        let mut nonce = [0; 32];
        for (index, byte) in random.into_iter().enumerate() {
            nonce[index * 2] = b"0123456789abcdef"[usize::from(byte >> 4)];
            nonce[index * 2 + 1] = b"0123456789abcdef"[usize::from(byte & 15)];
        }
        let artifacts = StartupArtifacts::new(directory, path, nonce)?;
        let script = bootstrap_script(&artifacts, marker, command)?;
        let program = shell
            .program()
            .to_str()
            .ok_or(TerminalStartupError::InvalidRequest)?
            .to_owned();
        let mut source = format!(
            ". {}\n",
            quote(
                artifacts
                    .script_path()
                    .to_str()
                    .ok_or(TerminalStartupError::InvalidRequest)?
            )
        );
        if command.is_none() && (source.len() > 512 || !source.is_ascii()) {
            // Physical lines stay below both canonical tty limits. Builtin read
            // consumes ASCII-escaped path data, split only at Unicode chars.
            let nonce_text = std::str::from_utf8(&nonce).map_err(process_error)?;
            let name = format!("_mg_bootstrap_{}", &nonce_text[..16]);
            let part = format!("_mg_chunk_{}", &nonce_text[..16]);
            source = format!(
                "{}{nonce_text}\n{name}=''; while builtin printf '\\033Pmg:{nonce_text}\\033\\\\'; IFS= builtin read -r {part} && [[ ${part} != . ]]; do {name}+=\"${part}\"; done; builtin printf -v {name} '%b' \"${name}\"; builtin unset {part}; . \"${name}\"; builtin unset {name}\n",
                crate::terminal_helper::PACED_STARTUP_PREFIX
            );
            let path = artifacts.script_path();
            let path = path.to_str().ok_or(TerminalStartupError::InvalidRequest)?;
            let mut chunk = String::new();
            for character in path.chars() {
                chunk.push(character);
                if chunk.len() >= 64 {
                    writeln!(source, "{}", encode_path_fragment(&chunk)?).map_err(process_error)?;
                    chunk.clear();
                }
            }
            if !chunk.is_empty() {
                writeln!(source, "{}", encode_path_fragment(&chunk)?).map_err(process_error)?;
            }
            source.push_str(".\n");
        }
        let mut arguments = shell.interactive_arguments();
        if command.is_some() {
            arguments.extend(["-c".into(), source.clone()]);
        }
        Ok(Self {
            program,
            arguments,
            source: command.is_none().then_some(source),
            script,
            nonce,
            deadline,
            artifacts,
            marker: marker.clone(),
        })
    }

    pub(crate) fn program(&self) -> &str {
        &self.program
    }
    pub(crate) fn arguments(&self) -> &[String] {
        &self.arguments
    }
    pub(crate) fn startup_source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Call only after transport arguments, environment and cwd are validated.
    pub(crate) fn publish(
        mut self,
        cancellation: &CancellationToken,
    ) -> Result<PublishedTerminalBootstrap> {
        crate::terminal_helper::check_deadline(self.deadline, cancellation)?;
        let listener =
            self.artifacts
                .publish(&self.script, &self.marker, self.deadline, cancellation)?;
        Ok(PublishedTerminalBootstrap {
            nonce: self.nonce,
            has_command: self.source.is_none(),
            deadline: self.deadline,
            listener,
            artifacts: self.artifacts,
        })
    }
}

impl PublishedTerminalBootstrap {
    /// Attach only the committed, owned transport. Marker receipts still require
    /// owner persistence before ACK; attachment itself does not release input.
    pub(crate) fn attach<B: TerminalSessionBackend>(
        self,
        pty: B,
    ) -> (TerminalStartupBackend<B>, TerminalStartupControl) {
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
            deadline: self.deadline,
            latch,
        };
        (backend, control)
    }
}

/// Owns the child and PTY. No startup-control handle grants process authority.
pub(crate) struct TerminalStartupBackend<B: TerminalSessionBackend = TerminalPty> {
    pty: B,
    latch: Arc<AtomicU8>,
    echo_disabled: bool,
    artifacts: Arc<Mutex<ArtifactRetirement>>,
}
impl TerminalStartupBackend {
    #[cfg(test)]
    pub(crate) fn pid(&self) -> std::num::NonZeroU32 {
        self.pty.pid()
    }
}
impl<B: TerminalSessionBackend> TerminalStartupBackend<B> {
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
impl<B: TerminalSessionBackend> TerminalSessionBackend for TerminalStartupBackend<B> {
    fn restore_startup_echo(&mut self) -> std::result::Result<(), ()> {
        self.restore_startup_echo().map_err(|_| ())
    }
    fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
        self.pty.read(buffer)
    }
    fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
        self.write_with_paste(bytes, false)
    }
    fn write_with_paste(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> std::result::Result<BackgroundInputReceipt, ()> {
        if bytes.is_empty() || bytes.len() > self.pty.input_write_limit() {
            return Err(());
        }
        match self.latch.load(Ordering::Acquire) {
            RELEASED => self.pty.write_with_paste(bytes, paste),
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
    fn input_write_limit(&self) -> usize {
        self.pty.input_write_limit()
    }
    fn settle_write(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> std::task::Poll<std::result::Result<BackgroundInputReceipt, ()>> {
        self.pty.settle_write(bytes, paste)
    }
    fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
        // The owner's existing lost-session path quiesces the child and drains
        // output under its persistence authority. Do not discard that tail here.
        if self.latch.load(Ordering::Acquire) == FAILED {
            return Err(());
        }
        self.pty.status()
    }
    fn resize(&mut self, dimensions: &TerminalDimensions) -> std::result::Result<(), ()> {
        TerminalSessionBackend::resize(&mut self.pty, dimensions)
    }
    fn signal(&mut self, signal: TerminalSignal) -> std::result::Result<(), ()> {
        TerminalSessionBackend::signal(&mut self.pty, signal)
    }
    fn signal_may_discard_output(&self) -> bool {
        self.pty.signal_may_discard_output()
    }
    fn close(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
    ) -> std::result::Result<TerminalPtyClose, ()> {
        self.latch.store(FAILED, Ordering::Release);
        let closed = self.pty.close(force, output);
        self.artifacts
            .lock()
            .map_err(|_| {
                #[cfg(test)]
                eprintln!("startup close artifact lock poisoned");
            })?
            .cleanup()
            .map_err(|error| {
                let _ = error;
                #[cfg(test)]
                eprintln!("startup close artifact cleanup failed: {error:?}");
            })?;
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
    pub(crate) fn has_command(&self) -> bool {
        self.has_command
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub(crate) fn poll(
        &mut self,
        now: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Option<TerminalStartupEvent>> {
        self.check(now.max(Instant::now()), cancellation)?;
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
            self.check(now.max(Instant::now()), cancellation)?;
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
    #[cfg(test)]
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

impl<B: TerminalSessionBackend> Drop for TerminalStartupBackend<B> {
    fn drop(&mut self) {
        self.latch.store(FAILED, Ordering::Release);
        let _ = self.pty.close(true, &mut |_: &[u8]| {});
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
impl StartupArtifacts {
    fn new(directory: OwnedFd, path: PathBuf, nonce: [u8; 32]) -> Result<Self> {
        let text = path.to_str().ok_or(TerminalStartupError::InvalidRequest)?;
        if !path.is_absolute()
            || text.len() > MAX_STARTUP_PATH_BYTES
            || text.chars().any(char::is_control)
            || std::fs::canonicalize(&path).map_err(process_error)? != path
        {
            return Err(TerminalStartupError::InvalidRequest);
        }
        let nonce_text = std::str::from_utf8(&nonce).map_err(process_error)?;
        let script_name = format!("b-{nonce_text}");
        let socket_name = format!("s-{nonce_text}");
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
    fn publish(
        &mut self,
        bootstrap: &str,
        helper: &TerminalPtyHelper,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<UnixListener> {
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
        let listener = crate::terminal_helper::bind_startup_listener(
            helper,
            true,
            &self.directory,
            &self.path,
            &self.socket_name,
            deadline,
            cancellation,
        )?;
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
        crate::terminal_helper::check_deadline(deadline, cancellation)?;
        if validate_directory(&self.directory, &self.path)? != self.identity {
            return Err(TerminalStartupError::InvalidRequest);
        }
        rustix::net::listen(&listener, 8).map_err(process_error)?;
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
fn encode_path_fragment(value: &str) -> Result<String> {
    // Readline under LANG=C may discard non-ASCII typed bytes. Bash and zsh
    // decode these host-owned ASCII escapes after reading each bounded line.
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        write!(encoded, "\\x{byte:02x}").map_err(process_error)?;
    }
    Ok(encoded)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU64;

    struct Directory(PathBuf);
    #[derive(Default)]
    struct Forwarded {
        output: Vec<u8>,
        writes: Vec<(usize, bool)>,
        settlements: Vec<(usize, bool)>,
        echoes: usize,
    }
    struct AlternateBackend(Arc<Mutex<Forwarded>>);
    impl TerminalSessionBackend for AlternateBackend {
        fn restore_startup_echo(&mut self) -> std::result::Result<(), ()> {
            self.0.lock().unwrap().echoes += 1;
            Ok(())
        }
        fn read(&mut self, bytes: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
            let mut state = self.0.lock().unwrap();
            let count = bytes.len().min(state.output.len());
            bytes[..count].copy_from_slice(&state.output[..count]);
            state.output.drain(..count);
            Ok(TerminalPtyRead {
                bytes_read: count,
                closed: false,
            })
        }
        fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
            self.write_with_paste(bytes, false)
        }
        fn write_with_paste(
            &mut self,
            bytes: &[u8],
            paste: bool,
        ) -> std::result::Result<BackgroundInputReceipt, ()> {
            self.0.lock().unwrap().writes.push((bytes.len(), paste));
            Ok(BackgroundInputReceipt::new(
                bytes.len(),
                false,
                BackgroundInputStatus::Written,
            ))
        }
        fn input_write_limit(&self) -> usize {
            32 * 1024
        }
        fn settle_write(
            &mut self,
            bytes: &[u8],
            paste: bool,
        ) -> std::task::Poll<std::result::Result<BackgroundInputReceipt, ()>> {
            self.0
                .lock()
                .unwrap()
                .settlements
                .push((bytes.len(), paste));
            std::task::Poll::Ready(Ok(BackgroundInputReceipt::new(
                bytes.len(),
                true,
                BackgroundInputStatus::Closed,
            )))
        }
        fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
            Ok(TerminalPtyStatus::Running)
        }
        fn resize(&mut self, _: &TerminalDimensions) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal(&mut self, _: TerminalSignal) -> std::result::Result<(), ()> {
            Ok(())
        }
        fn signal_may_discard_output(&self) -> bool {
            false
        }
        fn close(
            &mut self,
            _: bool,
            _: &mut dyn FnMut(&[u8]),
        ) -> std::result::Result<TerminalPtyClose, ()> {
            Ok(TerminalPtyClose {
                status: TerminalPtyStatus::Exited(0),
                output_incomplete: false,
            })
        }
    }

    #[test]
    fn shared_bootstrap_preserves_alternate_transport_input_and_settlement() {
        let directory = Directory::new();
        let prepared = PreparedTerminalBootstrap::new(
            &TerminalShell::from_executable(Path::new("/bin/bash"), true).unwrap(),
            None,
            directory.fd(),
            std::fs::canonicalize(&directory.0).unwrap(),
            &marker_helper(),
            Instant::now() + DEFAULT_STARTUP_TIMEOUT,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(prepared.startup_source().is_some());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
        let forwarded = Arc::new(Mutex::new(Forwarded::default()));
        let (mut backend, mut control) = prepared
            .publish(&CancellationToken::new())
            .unwrap()
            .attach(AlternateBackend(Arc::clone(&forwarded)));
        let forged = format!(
            "ordinary\x1bPmg:{}\x1b\\output",
            std::str::from_utf8(&control.nonce).unwrap()
        )
        .into_bytes();
        forwarded.lock().unwrap().output = forged.clone();
        let mut observed = [0; 128];
        let read = backend.read(&mut observed).unwrap();
        assert_eq!(&observed[..read.bytes_read], &forged);
        assert_eq!(
            control
                .poll(Instant::now(), &CancellationToken::new())
                .unwrap(),
            None
        );
        let bytes = vec![b'x'; 16 * 1024];
        assert_eq!(backend.input_write_limit(), 32 * 1024);
        assert!(!backend.signal_may_discard_output());
        backend.write_with_paste(&bytes, true).unwrap();
        assert!(forwarded.lock().unwrap().writes.is_empty());
        backend.restore_startup_echo().unwrap();
        backend.restore_startup_echo().unwrap();
        assert_eq!(forwarded.lock().unwrap().echoes, 1);
        backend.latch.store(RELEASED, Ordering::Release);
        backend.write_with_paste(&bytes, true).unwrap();
        assert_eq!(forwarded.lock().unwrap().writes, [(bytes.len(), true)]);
        drop(control);
        backend.write_with_paste(&bytes, true).unwrap();
        assert_eq!(forwarded.lock().unwrap().writes.len(), 1);
        assert!(backend.settle_write(&bytes, true).is_ready());
        assert_eq!(forwarded.lock().unwrap().settlements, [(bytes.len(), true)]);
        backend.close(true, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
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
                rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1)).unwrap();
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
    fn marker_helper() -> TerminalPtyHelper {
        if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            let program = PathBuf::from(program);
            assert!(
                program.is_absolute(),
                "release helper path must be absolute"
            );
            return TerminalPtyHelper::new(
                program,
                vec![crate::terminal_helper::TERMINAL_STARTUP_MARKER_ARGUMENT.into()],
            )
            .unwrap();
        }
        TerminalPtyHelper::new(
            std::env::current_exe().unwrap(),
            vec![
                "--exact".into(),
                "terminal_startup::tests::marker_helper_entry".into(),
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
            marker_helper: marker_helper(),
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
    #[test]
    fn long_quoted_artifact_paths_preserve_command_and_commandless_startup() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(shell).exists() {
                continue;
            }
            for commandless in [false, true] {
                let cwd = Directory::new();
                let root = Directory::new();
                let mut path = std::fs::canonicalize(&root.0).unwrap();
                while path.as_os_str().as_bytes().len() < 850 {
                    path.push(format!("é{}", "'".repeat(60)));
                    std::fs::create_dir(&path).unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                }
                let artifacts = Directory(path);
                let original_cwd = std::env::current_dir().unwrap();
                let (mut backend, mut control) = start(request(
                    &cwd,
                    &artifacts,
                    shell,
                    true,
                    (!commandless).then(|| "printf long > executed; exit 17".into()),
                ));
                assert_eq!(std::env::current_dir().unwrap(), original_cwd);
                let mut output = Vec::new();
                event(
                    &mut backend,
                    &mut control,
                    TerminalStartupEvent::ShellReady,
                    &mut output,
                );
                shell_ack(&mut backend, &mut control);
                if commandless {
                    let input = b"printf long > executed; exit 17\n";
                    assert_eq!(backend.write(input).unwrap().bytes_written(), input.len());
                } else {
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
                }
                finish(&mut backend, 17, &mut output);
                control.retry_cleanup().unwrap();
                assert_eq!(
                    std::fs::read_to_string(cwd.0.join("executed")).unwrap(),
                    "long"
                );
                assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
                assert_eq!(std::env::current_dir().unwrap(), original_cwd);
            }
        }
    }
    fn delayed_helper(cwd: &Directory) -> TerminalPtyHelper {
        let helper = helper();
        let mut arguments = vec![
            "-c".into(),
            "printf '%s' \"$$\" > \"$1\"; shift; sleep 2.2; exec \"$@\"".into(),
            "startup-delay".into(),
            cwd.0.join("helper-pid").into_os_string(),
            helper.program().as_os_str().to_owned(),
        ];
        arguments.extend_from_slice(helper.arguments());
        TerminalPtyHelper::new("/bin/sh".into(), arguments).unwrap()
    }

    struct RegisteredStartup {
        registry: crate::terminal_registry::TerminalRegistry<TerminalStartupBackend>,
        store: crate::terminal_profile_store::TerminalProfileStore,
        budget: crate::terminal_profile::TerminalProfileBudget,
        _catalog: crate::terminal_catalog::TerminalCatalog,
        owner: machine_god_core::BackgroundOutputOwner,
        id: machine_god_core::TerminalSessionId,
    }
    fn registered_startup(
        root: &Directory,
        cwd: &Directory,
        artifacts: &Directory,
        command: Option<String>,
        expired: bool,
    ) -> RegisteredStartup {
        use crate::terminal_history::TerminalHistory;
        use crate::terminal_journal::TerminalJournalLimits;
        use crate::terminal_profile::{
            TerminalProfileBudget, TerminalProfileLimits, TerminalProfileMutationContext,
        };
        use crate::terminal_profile_store::TerminalProfileStore;
        use crate::terminal_registry::TerminalRegistry;
        use crate::terminal_session::TerminalSession;
        use machine_god_core::{
            BackgroundOutputOwner, SessionId, SessionIncarnationId, TerminalSessionId,
        };
        let workspace = std::fs::canonicalize(&cwd.0)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let owner = BackgroundOutputOwner::new(
            SessionId::new("startup-owner").unwrap(),
            SessionIncarnationId::new("startup-incarnation").unwrap(),
        );
        let id = TerminalSessionId::new("startup-session").unwrap();
        let store = TerminalProfileStore::prepare(root.fd()).unwrap();
        let budget = TerminalProfileBudget::new(TerminalProfileLimits::default()).unwrap();
        let mut transaction = store.transaction().unwrap();
        let mut catalog = transaction
            .prepare_catalog(workspace.clone(), owner.clone())
            .unwrap();
        drop(transaction.create_session(&mut catalog, &id).unwrap());
        let journal = budget
            .create_journal(
                &mut transaction,
                catalog.namespace_key(),
                &id,
                TerminalJournalLimits::default(),
            )
            .unwrap();
        journal.accounting.unwrap();
        let mut context =
            TerminalProfileMutationContext::new(&mut transaction, budget, catalog.namespace_key());
        let history = TerminalHistory::create_with(
            &mut context,
            journal.operation.unwrap(),
            &TerminalDimensions::new(24, 80).unwrap(),
        )
        .unwrap();
        let mut metadata = crate::terminal_session_record::test_metadata();
        metadata.workspace = workspace.clone();
        metadata.cwd = workspace.clone();
        metadata.command = command.clone();
        metadata.profile = machine_god_core::TerminalProfile::User;
        let (backend, mut control) = start(request(cwd, artifacts, "/bin/bash", false, command));
        if expired {
            control.deadline = Instant::now();
        }
        let mut session = TerminalSession::new_with(
            &mut context,
            backend,
            history,
            owner.clone(),
            id.clone(),
            metadata,
            0,
        )
        .unwrap();
        session
            .attach_startup_control(control, CancellationToken::new())
            .unwrap();
        drop(transaction);
        let mut registry = TerminalRegistry::new(workspace).unwrap();
        registry
            .start(owner.clone(), id.clone(), || Ok(session))
            .unwrap();
        RegisteredStartup {
            registry,
            store,
            budget,
            _catalog: catalog,
            owner,
            id,
        }
    }

    #[test]
    fn owner_pump_persists_command_boundary_after_profile_output_before_release() {
        use crate::terminal_session_record::TerminalStartupStage;
        use machine_god_core::{TerminalClosePolicy, TerminalCursor, TerminalLifecycle};
        let root = Directory::new();
        let cwd = Directory::new();
        let artifacts = Directory::new();
        std::fs::write(
            cwd.0.join(".bash_profile"),
            "printf PROFILE; for fd in 3 4 5 6 7 8 9; do eval \"exec $fd>&-\"; done\n",
        )
        .unwrap();
        let mut fixture = registered_startup(
            &root,
            &cwd,
            &artifacts,
            Some("printf COMMAND; printf yes > executed; exit 7".into()),
            false,
        );
        let started = Instant::now();
        let mut shell_seen = false;
        let facts = loop {
            assert!(started.elapsed() < Duration::from_secs(10));
            let now = i64::try_from(started.elapsed().as_millis()).unwrap();
            for step in fixture
                .registry
                .pump_with_profile(&fixture.store, &fixture.budget, now, 1)
                .unwrap()
            {
                assert!(step.result.is_ok(), "{:?}", step.result.err());
                assert!(step.cleanup_error.is_none());
            }
            let facts = fixture
                .registry
                .inspect(&fixture.owner, &fixture.id)
                .unwrap();
            if facts.startup_stage == Some(TerminalStartupStage::ShellReady) {
                shell_seen = true;
                assert_eq!(facts.context.lifecycle, TerminalLifecycle::Starting);
                assert!(!cwd.0.join("executed").exists());
            }
            if facts.context.lifecycle == TerminalLifecycle::Exited {
                break facts;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(shell_seen);
        assert_eq!(
            facts.startup_stage,
            Some(TerminalStartupStage::CommandStarted)
        );
        let boundary = facts.command_start_cursor.unwrap();
        let prefix = fixture
            .registry
            .read(
                &fixture.owner,
                &fixture.id,
                &TerminalCursor::new(1, 0).unwrap(),
                16384,
            )
            .unwrap();
        assert!(String::from_utf8_lossy(&prefix.bytes).contains("PROFILE"));
        assert!(boundary.offset() > 0);
        let command = fixture
            .registry
            .read(&fixture.owner, &fixture.id, &boundary, 16384)
            .unwrap();
        assert!(String::from_utf8_lossy(&command.bytes).contains("COMMAND"));
        assert!(!String::from_utf8_lossy(&command.bytes).contains("PROFILE"));
        assert_eq!(
            std::fs::read_to_string(cwd.0.join("executed")).unwrap(),
            "yes"
        );
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
        assert!(
            fixture
                .registry
                .shutdown_with_profile(
                    &fixture.store,
                    &fixture.budget,
                    fixture.registry.minimum_time_ms(),
                    TerminalClosePolicy::Force
                )
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn startup_deadline_quiesces_owned_process_even_while_profile_is_locked() {
        use machine_god_core::TerminalLifecycle;
        let root = Directory::new();
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let mut fixture = registered_startup(
            &root,
            &cwd,
            &artifacts,
            Some("printf BAD > executed".into()),
            true,
        );
        let transaction = fixture.store.transaction().unwrap();
        let steps = fixture
            .registry
            .pump_with_profile(&fixture.store, &fixture.budget, 1, 1)
            .unwrap();
        assert!(steps[0].result.is_err());
        let session = fixture
            .registry
            .live_mut(&fixture.owner, &fixture.id)
            .unwrap();
        assert!(!session.owns_backend());
        assert!(session.publication_error().is_some());
        assert_eq!(session.context().lifecycle, TerminalLifecycle::Lost);
        assert!(!cwd.0.join("executed").exists());
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
        drop(transaction);
        assert!(
            fixture
                .registry
                .shutdown_with_profile(
                    &fixture.store,
                    &fixture.budget,
                    2,
                    machine_god_core::TerminalClosePolicy::Force
                )
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn delayed_helper_within_requested_startup_budget_executes_and_reaps() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let prepared = PreparedTerminalStartup::prepare(
            &delayed_helper(&cwd),
            request(
                &cwd,
                &artifacts,
                "/bin/bash",
                true,
                Some("printf command > executed".into()),
            ),
            &CancellationToken::new(),
        )
        .unwrap();
        let (mut backend, mut control) = prepared.commit(&CancellationToken::new()).unwrap();
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
            "command"
        );
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
    }
    #[test]
    fn short_prepare_budget_is_timeout_and_collects_delayed_helper() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let mut request = request(
            &cwd,
            &artifacts,
            "/bin/bash",
            true,
            Some("printf BAD > executed".into()),
        );
        request.timeout = Duration::from_secs(1);
        assert!(matches!(
            PreparedTerminalStartup::prepare(
                &delayed_helper(&cwd),
                request,
                &CancellationToken::new()
            ),
            Err(TerminalStartupError::Timeout)
        ));
        let pid = std::fs::read_to_string(cwd.0.join("helper-pid"))
            .unwrap()
            .parse::<i32>()
            .unwrap();
        assert_eq!(
            rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()),
            Err(rustix::io::Errno::SRCH)
        );
        assert!(!cwd.0.join("executed").exists());
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
    }
    #[test]
    fn owner_pause_before_commit_uses_remaining_original_budget() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let prepared = PreparedTerminalStartup::prepare(
            &helper(),
            request(
                &cwd,
                &artifacts,
                "/bin/bash",
                true,
                Some("printf command > executed".into()),
            ),
            &CancellationToken::new(),
        )
        .unwrap();
        let deadline = prepared.bootstrap.deadline;
        std::thread::sleep(Duration::from_millis(2200));
        let (mut backend, mut control) = prepared.commit(&CancellationToken::new()).unwrap();
        assert_eq!(control.deadline, deadline);
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
            "command"
        );
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
    }
    #[test]
    fn expired_commit_never_refreshes_budget_and_cancellation_wins() {
        for cancel in [false, true] {
            let cwd = Directory::new();
            let artifacts = Directory::new();
            let mut request = request(
                &cwd,
                &artifacts,
                "/bin/bash",
                true,
                Some("printf BAD > executed".into()),
            );
            request.timeout = Duration::from_millis(2500);
            let prepared =
                PreparedTerminalStartup::prepare(&helper(), request, &CancellationToken::new())
                    .unwrap();
            std::thread::sleep(
                prepared
                    .bootstrap
                    .deadline
                    .saturating_duration_since(Instant::now())
                    + Duration::from_millis(10),
            );
            let cancellation = CancellationToken::new();
            if cancel {
                cancellation.cancel();
            }
            assert!(
                matches!(prepared.commit(&cancellation), Err(error) if error == if cancel { TerminalStartupError::Cancelled } else { TerminalStartupError::Timeout })
            );
            assert!(!cwd.0.join("executed").exists());
            assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
        }
    }
    #[test]
    fn stale_owner_timestamp_cannot_acknowledge_after_actual_deadline() {
        let cwd = Directory::new();
        let artifacts = Directory::new();
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
        let before_persistence = Instant::now();
        control.deadline = Instant::now();
        assert_eq!(
            control.release_command(before_persistence, &CancellationToken::new()),
            Err(TerminalStartupError::Timeout)
        );
        backend.close(true, &mut |_| {}).unwrap();
        assert!(!cwd.0.join("executed").exists());
        assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
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
                    let long = artifacts.0.join("invalid\npath");
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
