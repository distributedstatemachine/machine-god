//! Bounded adapter for an already prepared, authenticated tmux pane.
//!
//! The host owns startup, durable identity and the authenticated pipe-pane raw
//! stream. Recovery must supply a fresh process-incarnation capability; a saved
//! PID is comparison data only. All subprocesses belong to the blocking owner.

use crate::background_input::{
    BackgroundInputReceipt, BackgroundInputStatus, MAX_BACKGROUND_INPUT_BYTES,
};
use crate::background_process::{TmuxChild, ValidatedBackgroundEnvironment};
use crate::terminal_helper::validate_startup_directory;
use crate::terminal_pty::{TerminalPtyClose, TerminalPtyRead, TerminalPtyStatus};
use crate::terminal_session::TerminalSessionBackend;
use machine_god_core::{TerminalDimensions, TerminalSignal};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{AtFlags, FileType, OFlags};
use std::ffi::OsString;
use std::fmt;
use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Child;
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::task::Poll;
use std::time::{Duration, Instant};

const MAX_COMMAND_OUTPUT: usize = 64 * 1024;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_COMMAND_ERROR: usize = 4096;
const READ_BYTES: usize = 16 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const STATUS_INTERVAL: Duration = Duration::from_millis(100);
const CLOSE_GRACE: Duration = Duration::from_millis(800);
const CLOSE_SETTLE: Duration = Duration::from_millis(500);
pub(crate) const NAMESPACE_OPTION: &str = "@machine_god_terminal_namespace";
const INSPECT_FORMAT: &str = "#{@machine_god_terminal_namespace}|#{session_name}|#{pane_id}|#{pane_pid}|#{pane_dead}|#{pane_dead_status}|#{pane_width}|#{pane_height}";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalTmuxError {
    Invalid,
    Identity,
    Busy,
    Closed,
    Command,
    Protocol,
    Timeout,
    Capacity,
    Cleanup,
    WriteAmbiguous,
}
impl fmt::Display for TerminalTmuxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal tmux operation failed")
    }
}
impl std::error::Error for TerminalTmuxError {}
type Result<T> = std::result::Result<T, TerminalTmuxError>;

/// Inert, bounded comparison data. Constructing it grants no process authority.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct TerminalTmuxIdentity {
    namespace: String,
    session: String,
    pane: String,
    pid: NonZeroU32,
}
impl fmt::Debug for TerminalTmuxIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TerminalTmuxIdentity { .. }")
    }
}
impl TerminalTmuxIdentity {
    pub(crate) fn new(namespace: String, pane: String, pid: NonZeroU32) -> Result<Self> {
        if namespace.len() != 32
            || !namespace
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !pane.starts_with('%')
            || !decimal(&pane[1..])
            || pane[1..].parse::<u32>().is_err()
        {
            return Err(TerminalTmuxError::Invalid);
        }
        Ok(Self {
            session: format!("machine-god-{namespace}"),
            namespace,
            pane,
            pid,
        })
    }
    pub(crate) fn namespace(&self) -> &str {
        &self.namespace
    }
    #[cfg(test)]
    pub(crate) fn session(&self) -> &str {
        &self.session
    }
    pub(crate) fn pane(&self) -> &str {
        &self.pane
    }
    pub(crate) const fn pid(&self) -> NonZeroU32 {
        self.pid
    }
}

/// Native authority retained from startup or freshly authenticated recovery.
/// Implementations bind the entire identity, including a native incarnation
/// token unavailable from these display fields. Signals target that retained
/// scope, not a PID parsed from tmux output. Drop retains native cleanup duties.
pub(crate) trait TerminalTmuxProcess: Send {
    fn validate(&mut self, identity: &TerminalTmuxIdentity) -> Result<()>;
    fn signal(&mut self, signal: TerminalSignal) -> Result<()>;
    /// Cleanup-only progress after a failed close phase. Implementations must
    /// check the complete retained identity and use only already authenticated
    /// native handles, without discovery or treating success as quiescence.
    /// The default grants no fallback authority to an arbitrary adapter.
    fn signal_retained_cleanup(
        &mut self,
        _identity: &TerminalTmuxIdentity,
        _signal: TerminalSignal,
    ) -> Result<()> {
        Err(TerminalTmuxError::Identity)
    }
    /// Includes the original scope's descendants, not merely its shell leader.
    fn is_absent(&mut self) -> Result<bool>;
    /// Authenticated launcher/process outcome. tmux 3.2 does not expose a dead
    /// pane's signal; missing outcome is not permission to invent one.
    fn outcome(&mut self) -> Result<Option<TerminalPtyStatus>>;
    /// Supervising launchers separate their own pane lifetime from the actual
    /// child job. Ordinary prepared panes keep the default tmux observation.
    fn job_status(&mut self) -> Result<Option<TerminalPtyStatus>> {
        Ok(None)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TerminalTmuxCommand {
    Probe,
    Inspect,
    Load(Vec<u8>),
    Paste,
    PasteBracketed,
    DeleteBuffer,
    Resize(TerminalDimensions),
    KillSession,
    HasSession,
}
pub(crate) struct TerminalTmuxReply {
    pub(crate) success: bool,
    pub(crate) output: Vec<u8>,
}
/// One owned subprocess at a time; polling performs a bounded amount of I/O.
pub(crate) trait TerminalTmuxControl: Send {
    fn identity(&self) -> &TerminalTmuxIdentity;
    fn begin(&mut self, command: TerminalTmuxCommand, deadline: Instant) -> Result<()>;
    fn poll(&mut self) -> Poll<Result<TerminalTmuxReply>>;
    fn abort(&mut self) -> Result<()>;
}

/// Explicit production command authority. The retained private directory and
/// exact socket inode are revalidated before each command. `-N` prevents a
/// missing/replaced server from silently creating a new control namespace.
pub(crate) struct NativeTerminalTmuxControl {
    helper: Option<crate::terminal_pty::TerminalPtyHelper>,
    executable: PathBuf,
    environment: ValidatedBackgroundEnvironment,
    directory: OwnedFd,
    directory_path: PathBuf,
    socket_name: OsString,
    socket_identity: (u64, u64),
    identity: TerminalTmuxIdentity,
    pending: Option<CommandProcess>,
    immediate: Option<TerminalTmuxReply>,
}
impl NativeTerminalTmuxControl {
    pub(crate) fn new_with_helper(
        helper: crate::terminal_pty::TerminalPtyHelper,
        executable: PathBuf,
        environment: ValidatedBackgroundEnvironment,
        directory: OwnedFd,
        socket_path: &Path,
        identity: TerminalTmuxIdentity,
    ) -> Result<Self> {
        Self::compose(
            Some(helper),
            executable,
            environment,
            directory,
            socket_path,
            identity,
        )
    }
    #[cfg(test)]
    pub(crate) fn new(
        executable: PathBuf,
        environment: ValidatedBackgroundEnvironment,
        directory: OwnedFd,
        socket_path: &Path,
        identity: TerminalTmuxIdentity,
    ) -> Result<Self> {
        Self::compose(
            None,
            executable,
            environment,
            directory,
            socket_path,
            identity,
        )
    }
    fn compose(
        helper: Option<crate::terminal_pty::TerminalPtyHelper>,
        executable: PathBuf,
        environment: ValidatedBackgroundEnvironment,
        directory: OwnedFd,
        socket_path: &Path,
        identity: TerminalTmuxIdentity,
    ) -> Result<Self> {
        if !executable.is_absolute()
            || executable.as_os_str().as_bytes().len() > 4096
            || executable.as_os_str().as_bytes().contains(&0)
            || !socket_path.is_absolute()
            || socket_path.as_os_str().as_bytes().len()
                > crate::terminal_helper::MAX_STARTUP_PATH_BYTES + 35
            || socket_path.as_os_str().as_bytes().contains(&0)
        {
            return Err(TerminalTmuxError::Invalid);
        }
        let directory_path = socket_path
            .parent()
            .ok_or(TerminalTmuxError::Invalid)?
            .to_owned();
        if std::fs::canonicalize(&directory_path).map_err(|_| TerminalTmuxError::Identity)?
            != directory_path
        {
            return Err(TerminalTmuxError::Identity);
        }
        validate_startup_directory(&directory, &directory_path)
            .map_err(|_| TerminalTmuxError::Identity)?;
        let socket_name = socket_path
            .file_name()
            .ok_or(TerminalTmuxError::Invalid)?
            .to_owned();
        let socket_identity =
            socket_identity(&directory, &socket_name)?.ok_or(TerminalTmuxError::Identity)?;
        Ok(Self {
            helper,
            executable,
            environment,
            directory,
            directory_path,
            socket_name,
            socket_identity,
            identity,
            pending: None,
            immediate: None,
        })
    }
    fn validate_socket(&self) -> Result<bool> {
        validate_startup_directory(&self.directory, &self.directory_path)
            .map_err(|_| TerminalTmuxError::Identity)?;
        match socket_identity(&self.directory, &self.socket_name)? {
            Some(identity) if identity == self.socket_identity => Ok(true),
            Some(_) => Err(TerminalTmuxError::Identity),
            None => Ok(false),
        }
    }
    fn arguments(&self, operation: &TerminalTmuxCommand) -> Result<(Vec<OsString>, Vec<u8>)> {
        let mut arguments: Vec<OsString> = ["-N", "-S"].into_iter().map(Into::into).collect();
        arguments.push(if self.helper.is_some() {
            self.socket_name.clone()
        } else {
            self.directory_path.join(&self.socket_name).into_os_string()
        });
        arguments.extend(["-f", "/dev/null"].into_iter().map(Into::into));
        let target = self.identity.pane.as_str();
        let session = format!("={}", self.identity.session);
        let buffer = format!("machine-god-{}", self.identity.namespace);
        let mut input = Vec::new();
        let fields: Vec<String> = match operation {
            TerminalTmuxCommand::Probe => {
                return Ok((vec!["-V".into()], input));
            }
            TerminalTmuxCommand::Inspect => vec![
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                target.into(),
                INSPECT_FORMAT.into(),
            ],
            TerminalTmuxCommand::Load(bytes) => {
                if bytes.is_empty() || bytes.len() > MAX_INPUT_BYTES {
                    return Err(TerminalTmuxError::Invalid);
                }
                input.clone_from(bytes);
                vec!["load-buffer".into(), "-b".into(), buffer, "-".into()]
            }
            // Pinned tmux semantics convert LF to CR; paste additionally asks
            // tmux to bracket the bytes when the pane enables that mode.
            TerminalTmuxCommand::Paste | TerminalTmuxCommand::PasteBracketed => {
                let mut arguments = vec![
                    "paste-buffer".into(),
                    "-d".into(),
                    "-b".into(),
                    buffer,
                    "-t".into(),
                    target.into(),
                ];
                if *operation == TerminalTmuxCommand::PasteBracketed {
                    arguments.push("-p".into());
                }
                arguments
            }
            TerminalTmuxCommand::DeleteBuffer => vec!["delete-buffer".into(), "-b".into(), buffer],
            TerminalTmuxCommand::Resize(dimensions) => vec![
                "resize-window".into(),
                "-t".into(),
                session,
                "-x".into(),
                dimensions.columns().to_string(),
                "-y".into(),
                dimensions.rows().to_string(),
            ],
            TerminalTmuxCommand::KillSession => vec!["kill-session".into(), "-t".into(), session],
            TerminalTmuxCommand::HasSession => vec!["has-session".into(), "-t".into(), session],
        };
        arguments.extend(fields.into_iter().map(Into::into));
        Ok((arguments, input))
    }
}
impl TerminalTmuxControl for NativeTerminalTmuxControl {
    fn identity(&self) -> &TerminalTmuxIdentity {
        &self.identity
    }
    fn begin(&mut self, operation: TerminalTmuxCommand, deadline: Instant) -> Result<()> {
        if self.pending.is_some() || self.immediate.is_some() {
            return Err(TerminalTmuxError::Busy);
        }
        if Instant::now() >= deadline {
            return Err(TerminalTmuxError::Timeout);
        }
        if !self.validate_socket()? {
            if operation == TerminalTmuxCommand::HasSession {
                self.immediate = Some(TerminalTmuxReply {
                    success: false,
                    output: Vec::new(),
                });
                return Ok(());
            }
            return Err(TerminalTmuxError::Identity);
        }
        let (arguments, input) = self.arguments(&operation)?;
        let mut command = if let Some(helper) = &self.helper {
            crate::terminal_tmux_helper::relative_command(
                helper,
                &self.directory,
                &self.directory_path,
                &self.executable,
                &arguments,
            )
            .map_err(|_| TerminalTmuxError::Identity)?
        } else {
            let mut command = Command::new(&self.executable);
            command.args(arguments);
            command
        };
        command
            .env_clear()
            .envs(self.environment.entries().iter().cloned())
            .current_dir("/");
        self.pending = Some(CommandProcess::spawn(command, input, deadline)?);
        Ok(())
    }
    fn poll(&mut self) -> Poll<Result<TerminalTmuxReply>> {
        if let Some(reply) = self.immediate.take() {
            return Poll::Ready(Ok(reply));
        }
        let Some(pending) = self.pending.as_mut() else {
            return Poll::Ready(Err(TerminalTmuxError::Invalid));
        };
        let result = pending.poll();
        if result.is_ready() {
            if pending.abort().is_err() {
                // Keep failed subprocess cleanup on this same owner. A later
                // close/abort must retry it instead of losing the child handle.
                return Poll::Ready(Err(TerminalTmuxError::Cleanup));
            }
            self.pending.take();
        }
        result
    }
    fn abort(&mut self) -> Result<()> {
        self.immediate.take();
        if let Some(pending) = self.pending.as_mut() {
            pending.abort()?;
        }
        self.pending.take();
        Ok(())
    }
}
fn socket_identity(directory: &OwnedFd, name: &OsString) -> Result<Option<(u64, u64)>> {
    let stat = match rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => return Err(TerminalTmuxError::Identity),
    };
    if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
        || stat.st_uid != rustix::process::getuid().as_raw()
        || stat.st_mode & 0o077 != 0
    {
        return Err(TerminalTmuxError::Identity);
    }
    #[allow(
        clippy::unnecessary_cast,
        clippy::cast_sign_loss,
        reason = "Device IDs are opaque platform bit patterns."
    )]
    Ok(Some((stat.st_dev as u64, stat.st_ino as u64)))
}

pub(crate) struct CommandProcess {
    child: TmuxChild,
    input: Option<ChildStdin>,
    output: Option<ChildStdout>,
    error: Option<ChildStderr>,
    input_bytes: Vec<u8>,
    input_offset: usize,
    output_bytes: Vec<u8>,
    error_bytes: usize,
    #[cfg(test)]
    diagnostic_stderr: Vec<u8>,
    deadline: Instant,
    reaped: bool,
}
impl CommandProcess {
    pub(crate) fn spawn(
        mut command: Command,
        input_bytes: Vec<u8>,
        deadline: Instant,
    ) -> Result<Self> {
        if input_bytes.len() > MAX_INPUT_BYTES {
            return Err(TerminalTmuxError::Capacity);
        }
        let mut child = TmuxChild::spawn(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .map_err(|_| TerminalTmuxError::Command)?;
        let (input, output, error) = child.take_pipes();
        let mut value = Self {
            input,
            output,
            error,
            child,
            input_bytes,
            input_offset: 0,
            output_bytes: Vec::new(),
            error_bytes: 0,
            #[cfg(test)]
            diagnostic_stderr: Vec::new(),
            deadline,
            reaped: false,
        };
        set_nonblocking(value.input.as_ref().ok_or(TerminalTmuxError::Command)?)?;
        set_nonblocking(value.output.as_ref().ok_or(TerminalTmuxError::Command)?)?;
        set_nonblocking(value.error.as_ref().ok_or(TerminalTmuxError::Command)?)?;
        if value.input_bytes.is_empty() {
            value.input.take();
        }
        Ok(value)
    }
    pub(crate) fn poll(&mut self) -> Poll<Result<TerminalTmuxReply>> {
        if Instant::now() >= self.deadline {
            #[cfg(test)]
            eprintln!(
                "tmux command deadline expired: input={}/{} output={} stderr={:?}",
                self.input_offset,
                self.input_bytes.len(),
                self.output_bytes.len(),
                String::from_utf8_lossy(&self.diagnostic_stderr)
            );
            return Poll::Ready(Err(TerminalTmuxError::Timeout));
        }
        match self.step() {
            Ok(Some(reply)) => Poll::Ready(Ok(reply)),
            Ok(None) => Poll::Pending,
            Err(error) => {
                #[cfg(test)]
                eprintln!(
                    "tmux command failed: error={error:?} input={}/{} output={} stderr={:?}",
                    self.input_offset,
                    self.input_bytes.len(),
                    self.output_bytes.len(),
                    String::from_utf8_lossy(&self.diagnostic_stderr)
                );
                Poll::Ready(Err(error))
            }
        }
    }
    fn step(&mut self) -> Result<Option<TerminalTmuxReply>> {
        if let Some(input) = self.input.as_mut() {
            let end = self
                .input_bytes
                .len()
                .min(self.input_offset + MAX_BACKGROUND_INPUT_BYTES);
            match input.write(&self.input_bytes[self.input_offset..end]) {
                Ok(0) => return Err(TerminalTmuxError::Command),
                Ok(count) => self.input_offset += count,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => return Err(TerminalTmuxError::Command),
            }
            if self.input_offset == self.input_bytes.len() {
                self.input.take();
            }
        }
        let mut buffer = [0; READ_BYTES];
        if let Some(output) = self.output.as_mut() {
            match read_once(output, &mut buffer)? {
                Some(0) => {
                    self.output.take();
                }
                Some(count) => {
                    if count > MAX_COMMAND_OUTPUT.saturating_sub(self.output_bytes.len()) {
                        return Err(TerminalTmuxError::Capacity);
                    }
                    self.output_bytes.extend_from_slice(&buffer[..count]);
                }
                None => {}
            }
        }
        if let Some(error) = self.error.as_mut() {
            match read_once(error, &mut buffer)? {
                Some(0) => {
                    self.error.take();
                }
                Some(count) => {
                    self.error_bytes += count;
                    #[cfg(test)]
                    self.diagnostic_stderr.extend_from_slice(
                        &buffer[..count
                            .min(MAX_COMMAND_ERROR.saturating_sub(self.diagnostic_stderr.len()))],
                    );
                    if self.error_bytes > MAX_COMMAND_ERROR {
                        return Err(TerminalTmuxError::Capacity);
                    }
                }
                None => {}
            }
        }
        let status = self
            .child
            .try_wait()
            .map_err(|_| TerminalTmuxError::Command)?;
        if status.is_some() {
            self.reaped = true;
        }
        if let Some(status) = status
            && self.output.is_none()
            && self.error.is_none()
        {
            if self.input_offset != self.input_bytes.len() || !matches!(status.code(), Some(0 | 1))
            {
                #[cfg(test)]
                eprintln!("tmux command unexpected exit: {status}");
                return Err(TerminalTmuxError::Command);
            }
            return Ok(Some(TerminalTmuxReply {
                success: status.success(),
                output: std::mem::take(&mut self.output_bytes),
            }));
        }
        Ok(None)
    }
    pub(crate) fn abort(&mut self) -> Result<()> {
        self.input.take();
        self.output.take();
        self.error.take();
        if !self.reaped {
            self.child.abort().map_err(|_| TerminalTmuxError::Cleanup)?;
            self.reaped = true;
        }
        Ok(())
    }
}
impl Drop for CommandProcess {
    fn drop(&mut self) {
        // The child owner handles bounded cleanup after every transport is
        // closed, reusing any already-expired explicit abort deadline.
        self.input.take();
        self.output.take();
        self.error.take();
    }
}
fn set_nonblocking(fd: &impl AsFd) -> Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd).map_err(|_| TerminalTmuxError::Command)?;
    rustix::fs::fcntl_setfl(fd, flags | OFlags::NONBLOCK).map_err(|_| TerminalTmuxError::Command)
}
fn read_once(reader: &mut impl Read, buffer: &mut [u8]) -> Result<Option<usize>> {
    match reader.read(buffer) {
        Ok(count) => Ok(Some(count)),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(None)
        }
        Err(_) => Err(TerminalTmuxError::Command),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaneObservation {
    status: TerminalPtyStatus,
    transport_closed: bool,
    dimensions: TerminalDimensions,
}
fn decimal(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 10
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
}
fn inspect(
    identity: &TerminalTmuxIdentity,
    bytes: &[u8],
    outcome: impl FnOnce() -> Result<Option<TerminalPtyStatus>>,
) -> Result<PaneObservation> {
    if bytes.len() > 512 {
        return Err(TerminalTmuxError::Protocol);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| TerminalTmuxError::Protocol)?
        .strip_suffix('\n')
        .ok_or(TerminalTmuxError::Protocol)?;
    let fields: Vec<&str> = text.split('|').collect();
    if fields.len() != 8
        || fields[0] != identity.namespace
        || fields[1] != identity.session
        || fields[2] != identity.pane
        || !decimal(fields[3])
        || fields[3].parse::<u32>().ok() != Some(identity.pid.get())
    {
        return Err(TerminalTmuxError::Identity);
    }
    let status = match (fields[4], fields[5]) {
        ("0", "") => TerminalPtyStatus::Running,
        ("1", code) if decimal(code) => TerminalPtyStatus::Exited(
            code.parse::<u8>()
                .map_err(|_| TerminalTmuxError::Protocol)?
                .into(),
        ),
        ("1", "") => match outcome()? {
            Some(
                status @ (TerminalPtyStatus::Exited(0..=255)
                | TerminalPtyStatus::Signalled(1..=127)),
            ) => status,
            _ => return Err(TerminalTmuxError::Protocol),
        },
        _ => return Err(TerminalTmuxError::Protocol),
    };
    if !decimal(fields[6]) || !decimal(fields[7]) {
        return Err(TerminalTmuxError::Protocol);
    }
    let columns = fields[6].parse().map_err(|_| TerminalTmuxError::Protocol)?;
    let rows = fields[7].parse().map_err(|_| TerminalTmuxError::Protocol)?;
    Ok(PaneObservation {
        status,
        transport_closed: fields[4] == "1",
        dimensions: TerminalDimensions::new(rows, columns)
            .map_err(|_| TerminalTmuxError::Protocol)?,
    })
}
pub(crate) fn compatible_version(bytes: &[u8]) -> bool {
    if bytes.len() > 1024 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Some(text) = text.trim().strip_prefix("tmux ") else {
        return false;
    };
    let Some((major, minor)) = text.split_once('.') else {
        return false;
    };
    let end = minor.bytes().take_while(u8::is_ascii_digit).count();
    if !decimal(major)
        || !decimal(&minor[..end])
        || !minor[end..].bytes().all(|b| b.is_ascii_alphabetic())
    {
        return false;
    }
    matches!((major.parse::<u16>(), minor[..end].parse::<u16>()), (Ok(major), Ok(minor)) if major > 3 || major == 3 && minor >= 2)
}
fn process_observation(
    identity: &TerminalTmuxIdentity,
    bytes: &[u8],
    process: &mut impl TerminalTmuxProcess,
) -> Result<PaneObservation> {
    let mut observation = inspect(identity, bytes, || process.outcome())?;
    if let Some(status) = process.job_status()? {
        if !matches!(
            status,
            TerminalPtyStatus::Running
                | TerminalPtyStatus::Exited(0..=255)
                | TerminalPtyStatus::Signalled(1..=127)
        ) || status == TerminalPtyStatus::Running
            && observation.status != TerminalPtyStatus::Running
        {
            return Err(TerminalTmuxError::Protocol);
        }
        observation.status = status;
    }
    Ok(observation)
}
fn run(
    control: &mut impl TerminalTmuxControl,
    command: TerminalTmuxCommand,
    deadline: Instant,
) -> Result<TerminalTmuxReply> {
    control.begin(command, deadline)?;
    loop {
        if Instant::now() >= deadline {
            control.abort()?;
            return Err(TerminalTmuxError::Timeout);
        }
        match control.poll() {
            Poll::Ready(reply) => return reply,
            Poll::Pending => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}
fn success(reply: TerminalTmuxReply) -> Result<Vec<u8>> {
    if reply.success {
        Ok(reply.output)
    } else {
        Err(TerminalTmuxError::Command)
    }
}

enum Pending {
    Observe,
    WriteInspect(Vec<u8>, Instant),
    WriteLoad(Vec<u8>, Instant),
    WritePaste(Vec<u8>),
}
#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent capture, input, publication and cleanup obligations can overlap."
)]
pub(crate) struct TerminalTmuxBackend<C: TerminalTmuxControl, P: TerminalTmuxProcess> {
    control: C,
    process: P,
    capture: Option<UnixStream>,
    observation: PaneObservation,
    pending: Option<Pending>,
    receipt: Option<Vec<u8>>,
    paste: bool,
    write_ambiguous: bool,
    next_observation: Instant,
    failed: bool,
    input_closed: bool,
    capture_eof: bool,
    eof_needs_observation: bool,
    output_incomplete: bool,
    retired: bool,
}
impl<C: TerminalTmuxControl, P: TerminalTmuxProcess> TerminalTmuxBackend<C, P> {
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn process_for_test(&mut self) -> &mut P {
        &mut self.process
    }
    /// Blocking-owner preparation only. The caller has authenticated the raw
    /// capture peer and installed startup/cancellation ownership before entry.
    /// This constructor also serves recovery, but never creates process authority.
    #[cfg(test)]
    pub(crate) fn attach(control: C, process: P, capture: UnixStream) -> Result<Self> {
        Self::attach_until(control, process, capture, Instant::now() + COMMAND_TIMEOUT)
    }
    pub(crate) fn attach_until(
        mut control: C,
        mut process: P,
        capture: UnixStream,
        deadline: Instant,
    ) -> Result<Self> {
        process.validate(control.identity())?;
        if !compatible_version(&success(run(
            &mut control,
            TerminalTmuxCommand::Probe,
            deadline,
        )?)?) {
            return Err(TerminalTmuxError::Protocol);
        }
        let output = success(run(&mut control, TerminalTmuxCommand::Inspect, deadline)?)?;
        let observation = process_observation(control.identity(), &output, &mut process)?;
        if observation.status == TerminalPtyStatus::Running {
            process.validate(control.identity())?;
        }
        capture
            .set_nonblocking(true)
            .map_err(|_| TerminalTmuxError::Command)?;
        Ok(Self {
            control,
            process,
            capture: Some(capture),
            observation,
            pending: None,
            receipt: None,
            paste: false,
            write_ambiguous: false,
            next_observation: Instant::now() + STATUS_INTERVAL,
            failed: false,
            input_closed: false,
            capture_eof: false,
            eof_needs_observation: false,
            output_incomplete: false,
            retired: false,
        })
    }
    fn drive(&mut self) -> Result<()> {
        if self.failed {
            return Err(TerminalTmuxError::Closed);
        }
        if self.pending.is_none() {
            return Ok(());
        }
        let Poll::Ready(reply) = self.control.poll() else {
            return Ok(());
        };
        let pending = self.pending.take().ok_or(TerminalTmuxError::Invalid)?;
        #[cfg(test)]
        let stage = match &pending {
            Pending::Observe => "observe",
            Pending::WriteInspect(..) => "write-inspect",
            Pending::WriteLoad(..) => "write-load",
            Pending::WritePaste(..) => "write-paste",
        };
        let result = (|| {
            let bytes = success(reply?)?;
            match pending {
                Pending::Observe => {
                    self.observation =
                        process_observation(self.control.identity(), &bytes, &mut self.process)?;
                    if self.observation.status == TerminalPtyStatus::Running {
                        self.process.validate(self.control.identity())?;
                    }
                    self.next_observation = Instant::now() + STATUS_INTERVAL;
                    self.eof_needs_observation = false;
                }
                Pending::WriteInspect(input, deadline) => {
                    self.observation =
                        process_observation(self.control.identity(), &bytes, &mut self.process)?;
                    if self.observation.status != TerminalPtyStatus::Running {
                        return Err(TerminalTmuxError::Closed);
                    }
                    self.process.validate(self.control.identity())?;
                    self.control
                        .begin(TerminalTmuxCommand::Load(input.clone()), deadline)?;
                    self.pending = Some(Pending::WriteLoad(input, deadline));
                }
                Pending::WriteLoad(input, deadline) => {
                    self.process.validate(self.control.identity())?;
                    // Once the paste command is submitted, an error or abort
                    // cannot prove that the server accepted zero input bytes.
                    self.write_ambiguous = true;
                    self.control.begin(
                        if self.paste {
                            TerminalTmuxCommand::PasteBracketed
                        } else {
                            TerminalTmuxCommand::Paste
                        },
                        deadline,
                    )?;
                    self.pending = Some(Pending::WritePaste(input));
                }
                Pending::WritePaste(input) => {
                    self.write_ambiguous = false;
                    self.receipt = Some(input);
                }
            }
            Ok(())
        })();
        if result.is_err() {
            #[cfg(test)]
            eprintln!("tmux drive failed: stage={stage} result={result:?}");
            self.failed = true;
            self.input_closed = true;
        }
        result
    }
    fn write_inner(&mut self, bytes: &[u8]) -> Result<BackgroundInputReceipt> {
        self.write_kind(bytes, false)
    }
    pub(crate) fn write_with_paste(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> std::result::Result<BackgroundInputReceipt, ()> {
        self.write_kind(bytes, paste).map_err(|_| ())
    }
    #[cfg(test)]
    pub(crate) const fn write_ambiguous(&self) -> bool {
        self.write_ambiguous
    }
    fn settle_write_inner(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> Poll<Result<BackgroundInputReceipt>> {
        self.input_closed = true;
        if let Some(input) = self.receipt.as_ref() {
            if input != bytes || self.paste != paste {
                return Poll::Ready(Err(TerminalTmuxError::Identity));
            }
            let input = self.receipt.take().expect("matching retained completion");
            return Poll::Ready(Ok(BackgroundInputReceipt::new(
                input.len(),
                false,
                BackgroundInputStatus::Written,
            )));
        }
        match self.pending.as_ref() {
            Some(Pending::WriteInspect(input, _) | Pending::WriteLoad(input, _)) => {
                if input != bytes || self.paste != paste {
                    return Poll::Ready(Err(TerminalTmuxError::Identity));
                }
                // Cancelling preparation must never advance into paste-buffer.
                if let Err(error) = self.control.abort() {
                    return Poll::Ready(Err(error));
                }
                self.pending.take();
            }
            Some(Pending::WritePaste(input)) => {
                if input != bytes || self.paste != paste {
                    return Poll::Ready(Err(TerminalTmuxError::Identity));
                }
                let Poll::Ready(reply) = self.control.poll() else {
                    return Poll::Pending;
                };
                let Some(Pending::WritePaste(input)) = self.pending.take() else {
                    unreachable!()
                };
                if reply.and_then(success).is_err() {
                    self.failed = true;
                    return Poll::Ready(Err(TerminalTmuxError::WriteAmbiguous));
                }
                self.write_ambiguous = false;
                return Poll::Ready(Ok(BackgroundInputReceipt::new(
                    input.len(),
                    false,
                    BackgroundInputStatus::Written,
                )));
            }
            _ => {}
        }
        if self.write_ambiguous {
            Poll::Ready(Err(TerminalTmuxError::WriteAmbiguous))
        } else {
            Poll::Ready(Ok(BackgroundInputReceipt::new(
                0,
                true,
                BackgroundInputStatus::Closed,
            )))
        }
    }
    fn write_kind(&mut self, bytes: &[u8], paste: bool) -> Result<BackgroundInputReceipt> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(TerminalTmuxError::Capacity);
        }
        if let Some(
            Pending::WriteInspect(input, _)
            | Pending::WriteLoad(input, _)
            | Pending::WritePaste(input),
        ) = &self.pending
            && (input != bytes || self.paste != paste)
        {
            return Err(TerminalTmuxError::Busy);
        }
        if self
            .receipt
            .as_ref()
            .is_some_and(|input| input != bytes || self.paste != paste)
        {
            return Err(TerminalTmuxError::Busy);
        }
        if self.receipt.is_none() && !self.input_closed && !self.retired {
            self.drive()?;
        }
        if let Some(input) = self.receipt.take() {
            return Ok(BackgroundInputReceipt::new(
                input.len(),
                false,
                BackgroundInputStatus::Written,
            ));
        }
        if self.write_ambiguous && (self.input_closed || self.retired || self.failed) {
            return Err(TerminalTmuxError::WriteAmbiguous);
        }
        if self.input_closed || self.retired {
            return Ok(BackgroundInputReceipt::new(
                0,
                true,
                BackgroundInputStatus::Closed,
            ));
        }
        if bytes.is_empty() {
            return Ok(BackgroundInputReceipt::new(
                0,
                false,
                BackgroundInputStatus::Written,
            ));
        }
        if self.pending.is_none() {
            let deadline = Instant::now() + COMMAND_TIMEOUT;
            self.process.validate(self.control.identity())?;
            self.control.begin(TerminalTmuxCommand::Inspect, deadline)?;
            self.pending = Some(Pending::WriteInspect(bytes.to_vec(), deadline));
            self.paste = paste;
        }
        Ok(BackgroundInputReceipt::new(
            0,
            false,
            BackgroundInputStatus::Backpressure,
        ))
    }
    fn read_capture(&mut self, buffer: &mut [u8]) -> Result<TerminalPtyRead> {
        if buffer.len() > READ_BYTES {
            return Err(TerminalTmuxError::Capacity);
        }
        if buffer.is_empty() {
            return Ok(TerminalPtyRead {
                bytes_read: 0,
                closed: self.capture_eof,
            });
        }
        let Some(capture) = self.capture.as_mut() else {
            return Ok(TerminalPtyRead {
                bytes_read: 0,
                closed: true,
            });
        };
        match read_once(capture, buffer)? {
            Some(0) => {
                if !self.capture_eof {
                    self.eof_needs_observation = true;
                    self.next_observation = Instant::now();
                }
                self.capture_eof = true;
                Ok(TerminalPtyRead {
                    bytes_read: 0,
                    closed: true,
                })
            }
            Some(count) => Ok(TerminalPtyRead {
                bytes_read: count,
                closed: false,
            }),
            None => Ok(TerminalPtyRead {
                bytes_read: 0,
                closed: false,
            }),
        }
    }
    fn fresh_observation(&mut self) -> Result<()> {
        if matches!(self.pending, Some(Pending::Observe)) {
            self.control.abort()?;
            self.pending.take();
        }
        if self.pending.is_some() || self.receipt.is_some() {
            return Err(TerminalTmuxError::Busy);
        }
        let output = success(run(
            &mut self.control,
            TerminalTmuxCommand::Inspect,
            Instant::now() + COMMAND_TIMEOUT,
        )?)?;
        self.observation =
            process_observation(self.control.identity(), &output, &mut self.process)?;
        if self.observation.status == TerminalPtyStatus::Running {
            self.process.validate(self.control.identity())?;
        }
        Ok(())
    }
    fn drain(&mut self, budget: &mut usize, output: &mut dyn FnMut(&[u8])) -> Result<()> {
        let mut bytes = [0; READ_BYTES];
        while *budget != 0 && !self.capture_eof {
            *budget -= 1;
            let result = self.read_capture(&mut bytes)?;
            if result.bytes_read != 0 {
                output(&bytes[..result.bytes_read]);
            }
            if result.bytes_read == 0 {
                break;
            }
        }
        Ok(())
    }
    fn close_inner(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
    ) -> Result<TerminalPtyClose> {
        let mut delivery_attempted = false;
        let result = self.try_close_inner(force, output, &mut delivery_attempted);
        if result.is_err() && !delivery_attempted {
            // Command-abort, inventory, capture and outcome errors cannot
            // revoke existing exact-process cleanup authority. In particular,
            // retained live pins must be killable when discovery has exhausted
            // its own descriptor budget. No new PID can be admitted here.
            // Preserve the original failure and every cleanup obligation:
            // delivery is not complete inventory, exit, or namespace absence.
            let _ = self.process.signal_retained_cleanup(
                self.control.identity(),
                if force {
                    TerminalSignal::Kill
                } else {
                    TerminalSignal::Terminate
                },
            );
        }
        result
    }

    fn try_close_inner(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
        delivery_attempted: &mut bool,
    ) -> Result<TerminalPtyClose> {
        self.input_closed = true;
        if self.retired {
            return Ok(TerminalPtyClose {
                status: self.observation.status,
                output_incomplete: self.output_incomplete,
            });
        }
        if matches!(self.pending, Some(Pending::WritePaste(_))) {
            // Preserve any completion already available before cancelling the
            // outstanding command. Pending or failed completion stays ambiguous.
            let _ = self.drive();
        }
        self.control.abort()?;
        self.pending.take();
        let mut budget = 128;
        if self.drain(&mut budget, output).is_err() {
            self.output_incomplete = true;
        }
        if !self.process.is_absent()? {
            self.process.validate(self.control.identity())?;
            *delivery_attempted = true;
            self.process.signal(if force {
                TerminalSignal::Kill
            } else {
                TerminalSignal::Terminate
            })?;
            let deadline = Instant::now() + if force { CLOSE_SETTLE } else { CLOSE_GRACE };
            while Instant::now() < deadline && !self.process.is_absent()? {
                if self.drain(&mut budget, output).is_err() {
                    self.output_incomplete = true;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            if !self.process.is_absent()? {
                self.process.signal(TerminalSignal::Kill)?;
                let deadline = Instant::now() + CLOSE_SETTLE;
                while Instant::now() < deadline && !self.process.is_absent()? {
                    if self.drain(&mut budget, output).is_err() {
                        self.output_incomplete = true;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        if !self.process.is_absent()? {
            return Err(TerminalTmuxError::Cleanup);
        }
        // Process absence must be proven before retiring the owned namespace.
        // A terminal status must be observed, never inferred from our signal.
        // tmux closes the pane fd only after its pipe output and PTY input
        // drain; the supervised child's exit alone is not that transport proof.
        let transport_deadline = Instant::now() + COMMAND_TIMEOUT;
        while !self.observation.transport_closed {
            if Instant::now() >= transport_deadline || budget == 0 {
                self.output_incomplete = true;
                break;
            }
            if self.drain(&mut budget, output).is_err() {
                self.output_incomplete = true;
            }
            let output = success(run(
                &mut self.control,
                TerminalTmuxCommand::Inspect,
                transport_deadline,
            )?)?;
            self.observation =
                process_observation(self.control.identity(), &output, &mut self.process)?;
            if !self.observation.transport_closed {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if self.observation.status == TerminalPtyStatus::Running {
            return Err(TerminalTmuxError::Cleanup);
        }
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let _ = run(
            &mut self.control,
            TerminalTmuxCommand::DeleteBuffer,
            deadline,
        );
        let _ = run(
            &mut self.control,
            TerminalTmuxCommand::KillSession,
            deadline,
        );
        if run(&mut self.control, TerminalTmuxCommand::HasSession, deadline)?.success {
            return Err(TerminalTmuxError::Cleanup);
        }
        let deadline = Instant::now() + CLOSE_SETTLE;
        while !self.capture_eof && budget != 0 && Instant::now() < deadline {
            if self.drain(&mut budget, output).is_err() {
                self.output_incomplete = true;
                break;
            }
            if !self.capture_eof {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        self.output_incomplete |= !self.capture_eof;
        self.capture.take();
        self.retired = true;
        Ok(TerminalPtyClose {
            status: self.observation.status,
            output_incomplete: self.output_incomplete,
        })
    }
}
impl<C: TerminalTmuxControl, P: TerminalTmuxProcess> TerminalSessionBackend
    for TerminalTmuxBackend<C, P>
{
    fn read(&mut self, buffer: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
        self.drive()
            .and_then(|()| self.read_capture(buffer))
            .map_err(|error| {
                let _ = error;
                #[cfg(test)]
                eprintln!("tmux raw read failed: {error:?}");
            })
    }
    fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
        self.write_inner(bytes).map_err(|error| {
            let _ = error;
            #[cfg(test)]
            eprintln!("tmux write failed: {error:?}");
        })
    }
    fn write_with_paste(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> std::result::Result<BackgroundInputReceipt, ()> {
        self.write_kind(bytes, paste).map_err(|_| ())
    }
    fn input_write_limit(&self) -> usize {
        MAX_INPUT_BYTES
    }
    fn settle_write(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> Poll<std::result::Result<BackgroundInputReceipt, ()>> {
        self.settle_write_inner(bytes, paste)
            .map(|result| result.map_err(|_| ()))
    }
    fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
        self.drive().map_err(|_| ())?;
        if self.capture_eof
            && !self.eof_needs_observation
            && self.observation.status == TerminalPtyStatus::Running
        {
            return Err(());
        }
        if !self.retired && self.pending.is_none() && Instant::now() >= self.next_observation {
            self.control
                .begin(
                    TerminalTmuxCommand::Inspect,
                    Instant::now() + COMMAND_TIMEOUT,
                )
                .map_err(|_| ())?;
            self.pending = Some(Pending::Observe);
        }
        Ok(self.observation.status)
    }
    fn resize(&mut self, dimensions: &TerminalDimensions) -> std::result::Result<(), ()> {
        if self.input_closed || self.failed {
            return Err(());
        }
        self.fresh_observation().map_err(|_| ())?;
        if self.observation.status != TerminalPtyStatus::Running {
            return Err(());
        }
        success(
            run(
                &mut self.control,
                TerminalTmuxCommand::Resize(dimensions.clone()),
                Instant::now() + COMMAND_TIMEOUT,
            )
            .map_err(|_| ())?,
        )
        .map_err(|_| ())?;
        self.fresh_observation().map_err(|_| ())?;
        if &self.observation.dimensions != dimensions {
            return Err(());
        }
        Ok(())
    }
    fn signal(&mut self, signal: TerminalSignal) -> std::result::Result<(), ()> {
        if self.input_closed || self.failed {
            return Err(());
        }
        self.fresh_observation().map_err(|_| ())?;
        if self.observation.status != TerminalPtyStatus::Running {
            return Err(());
        }
        self.process.signal(signal).map_err(|_| ())
    }
    fn signal_may_discard_output(&self) -> bool {
        true
    }
    fn close(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
    ) -> std::result::Result<TerminalPtyClose, ()> {
        self.close_inner(force, output).map_err(|_| ())
    }
}
impl<C: TerminalTmuxControl, P: TerminalTmuxProcess> Drop for TerminalTmuxBackend<C, P> {
    fn drop(&mut self) {
        if !self.retired {
            let _ = self.close_inner(true, &mut |_| {});
        }
    }
}

#[cfg(test)]
mod tests {
    include!("terminal_tmux_reap_tests.rs");
    use super::*;
    use std::collections::VecDeque;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    fn identity() -> TerminalTmuxIdentity {
        TerminalTmuxIdentity::new(
            "0123456789abcdef0123456789abcdef".into(),
            "%7".into(),
            NonZeroU32::new(42).unwrap(),
        )
        .unwrap()
    }
    fn observation(
        id: &TerminalTmuxIdentity,
        status: TerminalPtyStatus,
        dimensions: &TerminalDimensions,
    ) -> Vec<u8> {
        let (dead, code) = match status {
            TerminalPtyStatus::Running => (0, String::new()),
            TerminalPtyStatus::Exited(code) => (1, code.to_string()),
            TerminalPtyStatus::Signalled(_) => (1, String::new()),
        };
        format!(
            "{}|{}|{}|{}|{dead}|{code}|{}|{}\n",
            id.namespace,
            id.session,
            id.pane,
            id.pid,
            dimensions.columns(),
            dimensions.rows()
        )
        .into_bytes()
    }
    #[allow(
        clippy::struct_excessive_bools,
        reason = "Independent fault injections for the deterministic backend."
    )]
    struct State {
        allowed: bool,
        absent: bool,
        status: TerminalPtyStatus,
        dimensions: TerminalDimensions,
        commands: Vec<TerminalTmuxCommand>,
        signals: Vec<TerminalSignal>,
        cleanup_signals: Vec<TerminalSignal>,
        close_failure: Option<&'static str>,
        validations: usize,
        outcome_available: bool,
        signal_exits: bool,
        namespace_present: bool,
        fails: VecDeque<TerminalTmuxCommand>,
        polls_pending: usize,
        suppress_resize: bool,
        job_status: Option<TerminalPtyStatus>,
    }
    impl Default for State {
        fn default() -> Self {
            Self {
                allowed: true,
                absent: false,
                status: TerminalPtyStatus::Running,
                dimensions: TerminalDimensions::new(24, 80).unwrap(),
                commands: Vec::new(),
                signals: Vec::new(),
                cleanup_signals: Vec::new(),
                close_failure: None,
                validations: 0,
                outcome_available: true,
                signal_exits: true,
                namespace_present: true,
                fails: VecDeque::new(),
                polls_pending: 0,
                suppress_resize: false,
                job_status: None,
            }
        }
    }
    struct Control {
        id: TerminalTmuxIdentity,
        state: Arc<Mutex<State>>,
        pending: Option<TerminalTmuxCommand>,
    }
    impl TerminalTmuxControl for Control {
        fn identity(&self) -> &TerminalTmuxIdentity {
            &self.id
        }
        fn begin(&mut self, command: TerminalTmuxCommand, deadline: Instant) -> Result<()> {
            if self.pending.is_some() {
                return Err(TerminalTmuxError::Busy);
            }
            if Instant::now() >= deadline {
                return Err(TerminalTmuxError::Timeout);
            }
            self.state.lock().unwrap().commands.push(command.clone());
            self.pending = Some(command);
            Ok(())
        }
        fn poll(&mut self) -> Poll<Result<TerminalTmuxReply>> {
            let mut state = self.state.lock().unwrap();
            if state.polls_pending != 0 {
                state.polls_pending -= 1;
                return Poll::Pending;
            }
            let command = self.pending.take().expect("owned command");
            if state.fails.front() == Some(&command) {
                state.fails.pop_front();
                return Poll::Ready(Err(TerminalTmuxError::Command));
            }
            let mut success = true;
            let output = match command {
                TerminalTmuxCommand::Probe => b"tmux 3.2\n".to_vec(),
                TerminalTmuxCommand::Inspect => {
                    observation(&self.id, state.status, &state.dimensions)
                }
                TerminalTmuxCommand::Resize(dimensions) => {
                    if !state.suppress_resize {
                        state.dimensions = dimensions;
                    }
                    Vec::new()
                }
                TerminalTmuxCommand::KillSession => {
                    state.namespace_present = false;
                    Vec::new()
                }
                TerminalTmuxCommand::HasSession => {
                    success = state.namespace_present;
                    Vec::new()
                }
                _ => Vec::new(),
            };
            Poll::Ready(Ok(TerminalTmuxReply { success, output }))
        }
        fn abort(&mut self) -> Result<()> {
            if self.state.lock().unwrap().close_failure == Some("abort") {
                return Err(TerminalTmuxError::Cleanup);
            }
            self.pending.take();
            Ok(())
        }
    }
    struct Process {
        id: TerminalTmuxIdentity,
        state: Arc<Mutex<State>>,
    }
    impl TerminalTmuxProcess for Process {
        fn validate(&mut self, identity: &TerminalTmuxIdentity) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.validations += 1;
            if !state.allowed || identity != &self.id || state.close_failure == Some("validate") {
                return Err(TerminalTmuxError::Identity);
            }
            Ok(())
        }
        fn signal(&mut self, signal: TerminalSignal) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            if !state.allowed {
                return Err(TerminalTmuxError::Identity);
            }
            state.signals.push(signal);
            if state.close_failure == Some("signal") {
                return Err(TerminalTmuxError::Command);
            }
            if state.signal_exits {
                state.absent = true;
                state.status = TerminalPtyStatus::Signalled(if signal == TerminalSignal::Kill {
                    9
                } else {
                    15
                });
            }
            Ok(())
        }
        fn is_absent(&mut self) -> Result<bool> {
            let state = self.state.lock().unwrap();
            if state.close_failure == Some("absence") {
                return Err(TerminalTmuxError::Cleanup);
            }
            Ok(state.absent)
        }
        fn signal_retained_cleanup(
            &mut self,
            identity: &TerminalTmuxIdentity,
            signal: TerminalSignal,
        ) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            if !state.allowed || identity != &self.id {
                return Err(TerminalTmuxError::Identity);
            }
            state.cleanup_signals.push(signal);
            // Retained delivery may itself fail; it never certifies absence.
            Err(TerminalTmuxError::Command)
        }
        fn outcome(&mut self) -> Result<Option<TerminalPtyStatus>> {
            let state = self.state.lock().unwrap();
            Ok(state.outcome_available.then_some(state.status))
        }
        fn job_status(&mut self) -> Result<Option<TerminalPtyStatus>> {
            Ok(self.state.lock().unwrap().job_status)
        }
    }
    type Backend = TerminalTmuxBackend<Control, Process>;
    fn fixture() -> (Backend, UnixStream, Arc<Mutex<State>>) {
        let state = Arc::new(Mutex::new(State::default()));
        let (capture, peer) = UnixStream::pair().unwrap();
        let control = Control {
            id: identity(),
            state: state.clone(),
            pending: None,
        };
        let process = Process {
            id: identity(),
            state: state.clone(),
        };
        let backend = Backend::attach(control, process, capture).unwrap();
        (backend, peer, state)
    }
    fn finish(mut backend: Backend, peer: UnixStream, state: &Arc<Mutex<State>>) {
        {
            let mut state = state.lock().unwrap();
            state.absent = true;
            state.status = TerminalPtyStatus::Exited(0);
        }
        drop(peer);
        backend.close_inner(true, &mut |_| {}).unwrap();
    }

    #[test]
    fn identity_and_version_are_bounded_redacted_and_inert() {
        for namespace in [
            "",
            "0123456789ABCDEF0123456789ABCDEF",
            "../../../../../../../../../../../x",
        ] {
            assert_eq!(
                TerminalTmuxIdentity::new(
                    namespace.into(),
                    "%7".into(),
                    NonZeroU32::new(1).unwrap()
                ),
                Err(TerminalTmuxError::Invalid)
            );
        }
        for pane in ["7", "%", "%00", "%-1", "%1;kill-server", "%4294967296"] {
            assert!(
                TerminalTmuxIdentity::new(
                    identity().namespace,
                    pane.into(),
                    NonZeroU32::new(1).unwrap()
                )
                .is_err()
            );
        }
        assert!(!format!("{:?}", identity()).contains("012345"));
        for version in ["tmux 3.2\n", "tmux 3.6a\n", "tmux 4.0"] {
            assert!(compatible_version(version.as_bytes()));
        }
        for version in [
            "tmux 3.1",
            "tmux next",
            "tmux 3.2;exec",
            "tmux 3.",
            "tmux 65536.1",
            "tmux 03.2",
        ] {
            assert!(!compatible_version(version.as_bytes()));
        }
    }

    #[test]
    fn pane_observation_requires_exact_identity_and_authoritative_old_tmux_signal_outcome() {
        let id = identity();
        let dimensions = TerminalDimensions::new(24, 80).unwrap();
        let live = observation(&id, TerminalPtyStatus::Running, &dimensions);
        assert_eq!(
            inspect(&id, &live, || panic!(
                "running pane does not ask for outcome"
            ))
            .unwrap()
            .status,
            TerminalPtyStatus::Running
        );
        let signalled = observation(&id, TerminalPtyStatus::Signalled(9), &dimensions);
        assert_eq!(
            inspect(&id, &signalled, || Ok(Some(TerminalPtyStatus::Signalled(
                9
            ))))
            .unwrap()
            .status,
            TerminalPtyStatus::Signalled(9)
        );
        assert_eq!(
            inspect(&id, &signalled, || Ok(None)),
            Err(TerminalTmuxError::Protocol)
        );
        let mut wrong = id.clone();
        wrong.pid = NonZeroU32::new(43).unwrap();
        assert_eq!(
            inspect(&wrong, &live, || Ok(None)),
            Err(TerminalTmuxError::Identity)
        );
        assert!(inspect(&id, &live[..live.len() - 1], || Ok(None)).is_err());
        assert!(inspect(&id, &vec![b'x'; 513], || Ok(None)).is_err());
        let invalid = String::from_utf8(live)
            .unwrap()
            .replace("|0||80|24", "|2||80|24");
        assert!(inspect(&id, invalid.as_bytes(), || Ok(None)).is_err());
    }

    #[test]
    fn denied_attach_never_executes_a_tmux_command() {
        let state = Arc::new(Mutex::new(State {
            allowed: false,
            ..State::default()
        }));
        let (capture, _peer) = UnixStream::pair().unwrap();
        let result = Backend::attach(
            Control {
                id: identity(),
                state: state.clone(),
                pending: None,
            },
            Process {
                id: identity(),
                state: state.clone(),
            },
            capture,
        );
        assert!(matches!(result, Err(TerminalTmuxError::Identity)));
        assert!(state.lock().unwrap().commands.is_empty());
    }

    #[test]
    fn supervised_job_exit_does_not_remain_running_because_its_helper_is_alive() {
        for status in [
            TerminalPtyStatus::Exited(137),
            TerminalPtyStatus::Signalled(9),
        ] {
            let (mut backend, peer, state) = fixture();
            state.lock().unwrap().job_status = Some(status);
            backend.next_observation = Instant::now();
            backend.status().unwrap();
            assert_eq!(backend.status().unwrap(), status);
            assert!(!backend.observation.transport_closed);
            assert_eq!(state.lock().unwrap().status, TerminalPtyStatus::Running);
            finish(backend, peer, &state);
        }
    }

    #[test]
    fn raw_capture_is_exact_nonblocking_and_has_no_startup_authority() {
        let (mut backend, mut peer, state) = fixture();
        let mut buffer = [0; 128];
        assert_eq!(backend.read(&mut buffer).unwrap().bytes_read, 0);
        let bytes = b"\0\xff\x1b[2Jfake shell-ready\r\n";
        peer.write_all(bytes).unwrap();
        let read = backend.read(&mut buffer).unwrap();
        assert_eq!(&buffer[..read.bytes_read], bytes);
        assert_eq!(backend.status().unwrap(), TerminalPtyStatus::Running);
        assert!(backend.read(&mut vec![0; READ_BYTES + 1]).is_err());
        assert!(state.lock().unwrap().signals.is_empty());
        finish(backend, peer, &state);
    }

    #[test]
    fn writes_are_exact_single_pastes_and_keep_receipts_until_the_matching_retry() {
        let (mut backend, peer, state) = fixture();
        let bytes = b"\0\xff\r\n'$(touch forbidden)'\x1b[200~";
        assert_eq!(
            backend.write(bytes).unwrap().status(),
            BackgroundInputStatus::Backpressure
        );
        assert_eq!(
            backend.write_inner(b"different"),
            Err(TerminalTmuxError::Busy)
        );
        for _ in 0..3 {
            backend.drive().unwrap();
        }
        assert_eq!(
            backend.write_inner(b"different"),
            Err(TerminalTmuxError::Busy)
        );
        let receipt = backend.write(bytes).unwrap();
        assert_eq!(receipt.bytes_written(), bytes.len());
        assert_eq!(receipt.status(), BackgroundInputStatus::Written);
        let commands = state.lock().unwrap().commands.clone();
        assert_eq!(
            commands
                .iter()
                .filter(|command| **command == TerminalTmuxCommand::Paste)
                .count(),
            1
        );
        assert!(commands.contains(&TerminalTmuxCommand::Load(bytes.to_vec())));
        assert_eq!(backend.write(&[]).unwrap().bytes_written(), 0);
        finish(backend, peer, &state);
    }

    #[test]
    fn maximum_paste_remains_one_operation_and_cannot_change_kind_during_retry() {
        let (mut backend, peer, state) = fixture();
        let bytes = vec![b'p'; MAX_INPUT_BYTES];
        assert_eq!(
            TerminalSessionBackend::input_write_limit(&backend),
            bytes.len()
        );
        assert_eq!(
            TerminalSessionBackend::write_with_paste(&mut backend, &bytes, true)
                .unwrap()
                .bytes_written(),
            0
        );
        assert!(backend.write_with_paste(&bytes, false).is_err());
        for _ in 0..3 {
            backend.drive().unwrap();
        }
        assert!(backend.write_with_paste(&bytes, false).is_err());
        assert_eq!(
            backend
                .write_with_paste(&bytes, true)
                .unwrap()
                .bytes_written(),
            MAX_INPUT_BYTES
        );
        let commands = state.lock().unwrap().commands.clone();
        assert_eq!(
            commands
                .iter()
                .filter(|command| **command == TerminalTmuxCommand::PasteBracketed)
                .count(),
            1
        );
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, TerminalTmuxCommand::Load(_)))
                .count(),
            1
        );
        assert!(commands.contains(&TerminalTmuxCommand::Load(bytes)));
        finish(backend, peer, &state);
    }

    #[test]
    fn completed_paste_receipt_survives_exit_observation_and_close_before_retry() {
        let (mut backend, peer, state) = fixture();
        backend.write_with_paste(b"committed", true).unwrap();
        for _ in 0..3 {
            backend.drive().unwrap();
        }
        {
            let mut state = state.lock().unwrap();
            state.status = TerminalPtyStatus::Exited(0);
            state.absent = true;
        }
        backend.next_observation = Instant::now();
        backend.status().unwrap();
        assert_eq!(backend.status().unwrap(), TerminalPtyStatus::Exited(0));
        drop(peer);
        backend.close_inner(true, &mut |_| {}).unwrap();
        assert!(backend.write_with_paste(b"different", true).is_err());
        let receipt = backend.write_with_paste(b"committed", true).unwrap();
        assert_eq!(receipt.bytes_written(), 9);
        assert_eq!(receipt.status(), BackgroundInputStatus::Written);
        assert!(!backend.write_ambiguous());
        assert_eq!(
            backend
                .write_with_paste(b"committed", true)
                .unwrap()
                .status(),
            BackgroundInputStatus::Closed
        );
    }

    #[test]
    fn close_of_pending_paste_never_reports_a_reliable_zero_byte_receipt() {
        let (mut backend, peer, state) = fixture();
        backend.write(b"maybe committed").unwrap();
        backend.drive().unwrap();
        backend.drive().unwrap();
        state.lock().unwrap().polls_pending = 1;
        drop(peer);
        backend.close_inner(true, &mut |_| {}).unwrap();
        assert!(backend.write_ambiguous());
        assert_eq!(
            backend.write_inner(b"maybe committed"),
            Err(TerminalTmuxError::WriteAmbiguous)
        );
    }

    #[test]
    fn observation_only_settlement_cancels_preparation_without_submitting_paste() {
        for advances in [0, 1] {
            let (mut backend, peer, state) = fixture();
            backend.write_with_paste(b"never sent", true).unwrap();
            for _ in 0..advances {
                backend.drive().unwrap();
            }
            let Poll::Ready(Ok(receipt)) = backend.settle_write_inner(b"never sent", true) else {
                panic!()
            };
            assert_eq!(receipt.bytes_written(), 0);
            assert_eq!(receipt.status(), BackgroundInputStatus::Closed);
            backend.status().unwrap();
            assert!(
                !state
                    .lock()
                    .unwrap()
                    .commands
                    .iter()
                    .any(|command| matches!(
                        command,
                        TerminalTmuxCommand::Paste | TerminalTmuxCommand::PasteBracketed
                    ))
            );
            finish(backend, peer, &state);
        }
    }

    #[test]
    fn observation_only_settlement_preserves_close_time_completion_or_ambiguity() {
        for complete in [true, false] {
            let (mut backend, peer, state) = fixture();
            backend.write_with_paste(b"pending paste", true).unwrap();
            backend.drive().unwrap();
            backend.drive().unwrap();
            state.lock().unwrap().polls_pending = if complete { 1 } else { 2 };
            assert!(
                backend
                    .settle_write_inner(b"pending paste", true)
                    .is_pending()
            );
            drop(peer);
            backend.close_inner(true, &mut |_| {}).unwrap();
            let completion = backend.settle_write_inner(b"pending paste", true);
            if complete {
                let Poll::Ready(Ok(receipt)) = completion else {
                    panic!()
                };
                assert_eq!(receipt.bytes_written(), 13);
                assert_eq!(receipt.status(), BackgroundInputStatus::Written);
            } else {
                assert!(matches!(
                    completion,
                    Poll::Ready(Err(TerminalTmuxError::WriteAmbiguous))
                ));
            }
            backend.close_inner(true, &mut |_| {}).unwrap();
            assert_eq!(
                state
                    .lock()
                    .unwrap()
                    .commands
                    .iter()
                    .filter(|command| **command == TerminalTmuxCommand::PasteBracketed)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn backpressure_does_not_spawn_unbounded_commands_or_extend_deadlines() {
        let (mut backend, peer, state) = fixture();
        backend.write(b"x").unwrap();
        let deadline = match backend.pending.as_ref().unwrap() {
            Pending::WriteInspect(_, deadline) => *deadline,
            _ => panic!(),
        };
        state.lock().unwrap().polls_pending = 100;
        for _ in 0..100 {
            assert_eq!(backend.write(b"x").unwrap().bytes_written(), 0);
        }
        assert_eq!(state.lock().unwrap().commands.len(), 3);
        match backend.pending.as_ref().unwrap() {
            Pending::WriteInspect(_, retained) => assert_eq!(*retained, deadline),
            _ => panic!(),
        }
        assert_eq!(
            backend.write_inner(&vec![0; MAX_INPUT_BYTES + 1]),
            Err(TerminalTmuxError::Capacity)
        );
        finish(backend, peer, &state);
    }

    #[test]
    fn permission_loss_after_load_never_pastes_and_close_retires_the_buffer() {
        let (mut backend, peer, state) = fixture();
        backend.write(b"x").unwrap();
        backend.drive().unwrap();
        state.lock().unwrap().allowed = false;
        assert_eq!(backend.drive(), Err(TerminalTmuxError::Identity));
        assert!(
            !state
                .lock()
                .unwrap()
                .commands
                .contains(&TerminalTmuxCommand::Paste)
        );
        state.lock().unwrap().allowed = true;
        finish(backend, peer, &state);
        assert!(
            state
                .lock()
                .unwrap()
                .commands
                .contains(&TerminalTmuxCommand::DeleteBuffer)
        );
    }

    #[test]
    fn ambiguous_paste_failure_is_terminal_and_never_retried() {
        let (mut backend, peer, state) = fixture();
        state
            .lock()
            .unwrap()
            .fails
            .push_back(TerminalTmuxCommand::Paste);
        backend.write(b"x").unwrap();
        backend.drive().unwrap();
        backend.drive().unwrap();
        assert_eq!(backend.drive(), Err(TerminalTmuxError::Command));
        assert_eq!(
            backend.write_inner(b"x"),
            Err(TerminalTmuxError::WriteAmbiguous)
        );
        assert!(backend.write_ambiguous());
        assert_eq!(
            state
                .lock()
                .unwrap()
                .commands
                .iter()
                .filter(|command| **command == TerminalTmuxCommand::Paste)
                .count(),
            1
        );
        finish(backend, peer, &state);
    }

    #[test]
    fn resize_and_signal_cannot_overtake_pending_writes_and_resize_is_verified() {
        let (mut backend, peer, state) = fixture();
        let dimensions = TerminalDimensions::new(40, 120).unwrap();
        backend.write(b"x").unwrap();
        assert!(backend.resize(&dimensions).is_err());
        assert!(backend.signal(TerminalSignal::Interrupt).is_err());
        for _ in 0..3 {
            backend.drive().unwrap();
        }
        backend.write(b"x").unwrap();
        backend.resize(&dimensions).unwrap();
        state.lock().unwrap().suppress_resize = true;
        assert!(
            backend
                .resize(&TerminalDimensions::new(41, 120).unwrap())
                .is_err()
        );
        state.lock().unwrap().signal_exits = false;
        backend.signal(TerminalSignal::Interrupt).unwrap();
        assert_eq!(state.lock().unwrap().signals, [TerminalSignal::Interrupt]);
        finish(backend, peer, &state);
    }

    #[test]
    fn default_cleanup_fallback_grants_no_adapter_authority() {
        struct Adapter;
        impl TerminalTmuxProcess for Adapter {
            fn validate(&mut self, _: &TerminalTmuxIdentity) -> Result<()> {
                panic!("no validation fallback")
            }
            fn signal(&mut self, _: TerminalSignal) -> Result<()> {
                panic!("no signal fallback")
            }
            fn is_absent(&mut self) -> Result<bool> {
                panic!("no observation fallback")
            }
            fn outcome(&mut self) -> Result<Option<TerminalPtyStatus>> {
                panic!("no outcome fallback")
            }
        }
        assert_eq!(
            Adapter.signal_retained_cleanup(&identity(), TerminalSignal::Kill),
            Err(TerminalTmuxError::Identity)
        );
    }

    #[test]
    fn failed_close_attempts_retained_cleanup_without_repeating_prior_delivery() {
        for force in [false, true] {
            for phase in ["abort", "validate", "absence", "signal"] {
                let (mut backend, peer, state) = fixture();
                state.lock().unwrap().close_failure = Some(phase);
                let original_error = match phase {
                    "validate" => TerminalTmuxError::Identity,
                    "signal" => TerminalTmuxError::Command,
                    _ => TerminalTmuxError::Cleanup,
                };
                assert_eq!(backend.close_inner(force, &mut |_| {}), Err(original_error));
                let expected = if force {
                    TerminalSignal::Kill
                } else {
                    TerminalSignal::Terminate
                };
                {
                    let state = state.lock().unwrap();
                    assert!(state.namespace_present && !state.absent);
                    if phase == "signal" {
                        assert_eq!(state.signals, [expected]);
                        assert!(state.cleanup_signals.is_empty());
                    } else {
                        assert_eq!(state.cleanup_signals, [expected]);
                        assert!(state.signals.is_empty());
                    }
                }
                state.lock().unwrap().close_failure = None;
                finish(backend, peer, &state);
            }
        }
    }

    #[test]
    fn failed_close_never_turns_identity_rejection_into_retained_authority() {
        let (mut backend, peer, state) = fixture();
        state.lock().unwrap().allowed = false;
        assert!(backend.close_inner(true, &mut |_| {}).is_err());
        assert!(state.lock().unwrap().signals.is_empty());
        assert!(state.lock().unwrap().cleanup_signals.is_empty());
        state.lock().unwrap().allowed = true;
        finish(backend, peer, &state);
    }

    #[test]
    fn close_drains_before_namespace_retirement_and_is_idempotent() {
        let (mut backend, mut peer, state) = fixture();
        peer.write_all(b"last tail").unwrap();
        drop(peer);
        let mut output = Vec::new();
        let receipt = backend
            .close_inner(false, &mut |bytes| output.extend_from_slice(bytes))
            .unwrap();
        assert_eq!(output, b"last tail");
        assert_eq!(receipt.status, TerminalPtyStatus::Signalled(15));
        assert!(!receipt.output_incomplete);
        assert_eq!(state.lock().unwrap().signals, [TerminalSignal::Terminate]);
        let count = state.lock().unwrap().commands.len();
        assert_eq!(
            backend.close_inner(true, &mut |_| panic!()).unwrap(),
            receipt
        );
        assert_eq!(state.lock().unwrap().commands.len(), count);
    }

    #[test]
    fn unknown_exit_retains_namespace_for_same_owner_cleanup_retry() {
        let (mut backend, peer, state) = fixture();
        drop(peer);
        state.lock().unwrap().outcome_available = false;
        assert_eq!(
            backend.close_inner(true, &mut |_| {}),
            Err(TerminalTmuxError::Protocol)
        );
        assert!(state.lock().unwrap().namespace_present);
        assert!(!backend.retired);
        state.lock().unwrap().outcome_available = true;
        backend.close_inner(true, &mut |_| {}).unwrap();
        assert!(!state.lock().unwrap().namespace_present);
    }

    #[test]
    fn namespace_retirement_failure_keeps_cleanup_retryable() {
        let (mut backend, peer, state) = fixture();
        drop(peer);
        state
            .lock()
            .unwrap()
            .fails
            .push_back(TerminalTmuxCommand::KillSession);
        assert_eq!(
            backend.close_inner(true, &mut |_| {}),
            Err(TerminalTmuxError::Cleanup)
        );
        assert!(!backend.retired);
        backend.close_inner(true, &mut |_| {}).unwrap();
        assert!(backend.retired);
        assert_eq!(state.lock().unwrap().signals, [TerminalSignal::Kill]);
    }

    struct SocketFixture {
        root: PathBuf,
        listener: UnixListener,
    }
    impl SocketFixture {
        fn new() -> Self {
            let mut random = [0; 8];
            getrandom::fill(&mut random).unwrap();
            let root =
                std::env::temp_dir().join(format!("mg-tmux-{:x}", u64::from_ne_bytes(random)));
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            let listener = UnixListener::bind(root.join("t.sock")).unwrap();
            std::fs::set_permissions(root.join("t.sock"), std::fs::Permissions::from_mode(0o600))
                .unwrap();
            Self { root, listener }
        }
        fn control(&self, executable: &str) -> NativeTerminalTmuxControl {
            let directory = rustix::fs::open(
                &self.root,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .unwrap();
            NativeTerminalTmuxControl::new(
                executable.into(),
                ValidatedBackgroundEnvironment::new(Vec::new()).unwrap(),
                directory,
                &self.root.join("t.sock"),
                identity(),
            )
            .unwrap()
        }
    }
    impl Drop for SocketFixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn native_argv_preserves_exact_bytes_and_fixed_socket_target_without_shell_interpolation() {
        let fixture = SocketFixture::new();
        let control = fixture.control("/bin/echo");
        let (args, input) = control
            .arguments(&TerminalTmuxCommand::Load(b"\0\xff\n'$(bad)'".to_vec()))
            .unwrap();
        assert_eq!(input, b"\0\xff\n'$(bad)'");
        assert_eq!(args[0], "-N");
        assert_eq!(args[1], "-S");
        assert!(args.contains(&OsString::from("/dev/null")));
        assert_eq!(
            &args[5..],
            &[
                OsString::from("load-buffer"),
                "-b".into(),
                format!("machine-god-{}", identity().namespace).into(),
                "-".into()
            ]
        );
        let (args, input) = control.arguments(&TerminalTmuxCommand::Paste).unwrap();
        assert!(input.is_empty());
        assert!(!args.contains(&"-r".into()));
        assert_eq!(args.last().unwrap(), "%7");
        let (args, _) = control
            .arguments(&TerminalTmuxCommand::KillSession)
            .unwrap();
        assert_eq!(
            args.last().unwrap(),
            &OsString::from(format!("={}", identity().session))
        );
        assert_eq!(NAMESPACE_OPTION, "@machine_god_terminal_namespace");
    }

    #[test]
    fn socket_replacement_is_rejected_before_spawn_and_missing_server_is_not_restarted() {
        let fixture = SocketFixture::new();
        let mut control = fixture.control("/no-such-executable");
        std::fs::remove_file(fixture.root.join("t.sock")).unwrap();
        control
            .begin(
                TerminalTmuxCommand::HasSession,
                Instant::now() + COMMAND_TIMEOUT,
            )
            .unwrap();
        assert!(matches!(
            control.poll(),
            Poll::Ready(Ok(TerminalTmuxReply { success: false, .. }))
        ));
        assert_eq!(
            control.begin(TerminalTmuxCommand::Paste, Instant::now() + COMMAND_TIMEOUT),
            Err(TerminalTmuxError::Identity)
        );
        let _replacement = UnixListener::bind(fixture.root.join("t.sock")).unwrap();
        std::fs::set_permissions(
            fixture.root.join("t.sock"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_eq!(
            control.begin(
                TerminalTmuxCommand::Inspect,
                Instant::now() + COMMAND_TIMEOUT
            ),
            Err(TerminalTmuxError::Identity)
        );
        assert!(control.pending.is_none());
        let _ = fixture.listener.local_addr().unwrap();
    }

    fn collect(process: &mut CommandProcess) -> Result<TerminalTmuxReply> {
        loop {
            match process.poll() {
                Poll::Ready(result) => return result,
                Poll::Pending => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }

    /// A foreground, directly owned server on a new private socket. Tests never
    /// address the ambient/default tmux server, even during panic cleanup.
    struct RealServer {
        root: PathBuf,
        executable: PathBuf,
        child: Child,
    }
    impl RealServer {
        fn start() -> Option<Self> {
            let executable =
                if let Some(path) = std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY") {
                    let path = PathBuf::from(path);
                    assert!(
                        path.is_absolute() && path.is_file(),
                        "explicit tmux test executable is unavailable"
                    );
                    Some(path)
                } else {
                    [
                        "/opt/homebrew/bin/tmux",
                        "/usr/bin/tmux",
                        "/usr/local/bin/tmux",
                    ]
                    .into_iter()
                    .map(PathBuf::from)
                    .find(|path| path.is_file())
                };
            let Some(executable) = executable else {
                eprintln!("real tmux test skipped: no tmux executable installed");
                return None;
            };
            let mut random = [0; 8];
            getrandom::fill(&mut random).unwrap();
            let root = PathBuf::from("/tmp")
                .join(format!("mg-real-tmux-{:x}", u64::from_ne_bytes(random)));
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            let child = Command::new(&executable)
                .args(["-D", "-S"])
                .arg(root.join("t.sock"))
                .args(["-f", "/dev/null"])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("TERM", "xterm-256color")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut server = Self {
                root,
                executable,
                child,
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            while !server.root.join("t.sock").exists() {
                assert!(
                    server.child.try_wait().unwrap().is_none(),
                    "private server exited before readiness"
                );
                assert!(
                    Instant::now() < deadline,
                    "private server readiness timeout"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            Some(server)
        }
        fn command(&self, arguments: &[&str]) -> TerminalTmuxReply {
            let mut command = Command::new(&self.executable);
            command
                .args(["-N", "-S"])
                .arg(self.root.join("t.sock"))
                .args(["-f", "/dev/null"])
                .args(arguments)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("TERM", "xterm-256color");
            let mut process = CommandProcess::spawn(
                command,
                Vec::new(),
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
            let result = collect(&mut process);
            process.abort().unwrap();
            result.unwrap()
        }
        fn control(&self, id: TerminalTmuxIdentity) -> NativeTerminalTmuxControl {
            NativeTerminalTmuxControl::new(
                self.executable.clone(),
                ValidatedBackgroundEnvironment::new(vec![
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    ("TERM".into(), "xterm-256color".into()),
                ])
                .unwrap(),
                rustix::fs::open(
                    &self.root,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .unwrap(),
                &self.root.join("t.sock"),
                id,
            )
            .unwrap()
        }
    }
    impl Drop for RealServer {
        fn drop(&mut self) {
            // The directly retained foreground server identity is never rebuilt
            // from a recorded PID. It owns only this fixture's unique namespace.
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "One owned private server covers the complete command adapter lifecycle."
    )]
    fn real_private_tmux_controller_writes_pastes_resizes_observes_and_closes() {
        let Some(mut server) = RealServer::start() else {
            return;
        };
        let namespace = identity().namespace;
        let session = format!("machine-god-{namespace}");
        let reply = server.command(&[
            "new-session",
            "-d",
            "-s",
            &session,
            "-x",
            "80",
            "-y",
            "24",
            "-P",
            "-F",
            "#{pane_id}|#{pane_pid}",
            "/bin/sh",
            "-c",
            r"stty -echo -icanon; printf '\033[?2004hREADY'; exec /bin/cat -v",
        ]);
        assert!(reply.success);
        let created = String::from_utf8(reply.output).unwrap();
        let (pane, pid) = created.trim_end().split_once('|').unwrap();
        let id = TerminalTmuxIdentity::new(
            namespace,
            pane.to_owned(),
            NonZeroU32::new(pid.parse().unwrap()).unwrap(),
        )
        .unwrap();
        assert!(
            server
                .command(&[
                    "set-option",
                    "-t",
                    &session,
                    NAMESPACE_OPTION,
                    id.namespace()
                ])
                .success
        );
        assert!(
            server
                .command(&["set-option", "-t", &session, "remain-on-exit", "on"])
                .success
        );
        let mut control = server.control(id.clone());
        let invoke = |control: &mut NativeTerminalTmuxControl, command| {
            success(run(control, command, Instant::now() + Duration::from_secs(10)).unwrap())
                .unwrap()
        };
        assert!(compatible_version(&invoke(
            &mut control,
            TerminalTmuxCommand::Probe
        )));
        let initial = invoke(&mut control, TerminalTmuxCommand::Inspect);
        assert_eq!(
            inspect(&id, &initial, || Ok(None)).unwrap().status,
            TerminalPtyStatus::Running
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = server.command(&["capture-pane", "-p", "-t", id.pane()]);
            assert!(screen.success);
            if screen.output.windows(5).any(|bytes| bytes == b"READY") {
                break;
            }
            assert!(Instant::now() < deadline, "pane readiness timeout");
            std::thread::sleep(Duration::from_millis(2));
        }
        // Full binary input reaches exactly one tmux buffer, with each native
        // pipe write independently bounded. No payload becomes command argv.
        let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(MAX_INPUT_BYTES).collect();
        invoke(&mut control, TerminalTmuxCommand::Load(bytes.clone()));
        let buffer = format!("machine-god-{}", id.namespace());
        let loaded = server.command(&["save-buffer", "-b", &buffer, "-"]);
        assert!(loaded.success);
        assert_eq!(loaded.output, bytes);
        invoke(&mut control, TerminalTmuxCommand::DeleteBuffer);
        invoke(
            &mut control,
            TerminalTmuxCommand::Load(b"ordinary-write".to_vec()),
        );
        invoke(&mut control, TerminalTmuxCommand::Paste);
        invoke(
            &mut control,
            TerminalTmuxCommand::Load(b"bracketed-paste".to_vec()),
        );
        invoke(&mut control, TerminalTmuxCommand::PasteBracketed);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = server.command(&["capture-pane", "-p", "-t", id.pane()]);
            let screen = String::from_utf8(screen.output).unwrap();
            if screen.contains("ordinary-write^[[200~bracketed-paste^[[201~") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "paste was not delivered exactly once: {screen:?}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let dimensions = TerminalDimensions::new(40, 120).unwrap();
        invoke(
            &mut control,
            TerminalTmuxCommand::Resize(dimensions.clone()),
        );
        assert_eq!(
            inspect(
                &id,
                &invoke(&mut control, TerminalTmuxCommand::Inspect),
                || Ok(None)
            )
            .unwrap()
            .dimensions,
            dimensions
        );
        invoke(&mut control, TerminalTmuxCommand::KillSession);
        assert!(
            !run(
                &mut control,
                TerminalTmuxCommand::HasSession,
                Instant::now() + Duration::from_secs(10)
            )
            .unwrap()
            .success
        );
        assert!(server.command(&["kill-server"]).success);
        let deadline = Instant::now() + Duration::from_secs(10);
        while server.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "private server did not exit");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(server.child.wait().unwrap().success());
    }
    #[test]
    fn owned_command_io_roundtrips_the_full_binary_input_and_reaps() {
        let bytes: Vec<u8> = (0..=u8::MAX).cycle().take(MAX_INPUT_BYTES).collect();
        let mut command = Command::new("/bin/cat");
        command.env_clear();
        let mut process =
            CommandProcess::spawn(command, bytes.clone(), Instant::now() + COMMAND_TIMEOUT)
                .unwrap();
        assert!(matches!(process.poll(), Poll::Pending));
        assert!(process.input_offset <= MAX_BACKGROUND_INPUT_BYTES);
        let reply = collect(&mut process).unwrap();
        assert!(reply.success);
        assert_eq!(reply.output, bytes);
        assert!(process.reaped);
        assert!(process.child.try_wait().unwrap().is_some());
    }

    #[test]
    fn owned_command_bounds_output_stderr_time_and_collects_every_child() {
        for (script, expected, duration) in [
            (
                "exec /usr/bin/yes x",
                TerminalTmuxError::Capacity,
                COMMAND_TIMEOUT,
            ),
            (
                "exec /usr/bin/yes x >&2",
                TerminalTmuxError::Capacity,
                COMMAND_TIMEOUT,
            ),
            (
                "while :; do :; done",
                TerminalTmuxError::Timeout,
                Duration::from_millis(50),
            ),
        ] {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", script]).env_clear();
            let mut process =
                CommandProcess::spawn(command, Vec::new(), Instant::now() + duration).unwrap();
            assert!(matches!(collect(&mut process), Err(error) if error == expected));
            process.abort().unwrap();
            assert!(process.reaped);
            assert!(process.child.try_wait().unwrap().is_some());
            assert!(process.output_bytes.len() <= MAX_COMMAND_OUTPUT);
        }
    }
}
