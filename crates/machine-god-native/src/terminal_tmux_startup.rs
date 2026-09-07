//! Private tmux acquisition. No command addresses the ambient tmux server.
//!
//! A pane starts as a nonce-authenticated helper blocked before shell exec.
//! Native incarnation acquisition and a second challenge precede COMMIT. Raw
//! capture has a different one-use nonce and never carries launch authority.

use crate::background_input::BackgroundInputReceipt;
use crate::background_process::terminal_tmux_process::AuthenticatedTerminalProcess;
use crate::background_process::{BackgroundProcessSignal, ValidatedBackgroundEnvironment};
use crate::terminal_helper::{
    COMMIT, READY, check_deadline, read_gate, startup_directory_identity,
    validate_startup_directory, write_gate,
};
use crate::terminal_pty::{
    TerminalPtyClose, TerminalPtyDimensions, TerminalPtyHelper, TerminalPtyRead,
    TerminalPtyRequest, TerminalPtyStatus,
};
use crate::terminal_session::TerminalSessionBackend;
use crate::terminal_tmux::{
    CommandProcess, NAMESPACE_OPTION, NativeTerminalTmuxControl, TerminalTmuxBackend,
    TerminalTmuxCommand, TerminalTmuxControl, TerminalTmuxError, TerminalTmuxIdentity,
    TerminalTmuxProcess, TerminalTmuxReply,
};
#[cfg(test)]
use crate::terminal_tmux_helper::{
    CAPTURE_CHUNK, TERMINAL_TMUX_HELPER_ARGUMENT, run_terminal_tmux_helper,
};
use crate::terminal_tmux_helper::{
    MAX_HELPER_FRAME, PAUSE, PROOF_BYTES, Result, TTY_PROOF_BYTES, TerminalTmuxLaunchError,
    gate_error, process_error, set_echo, validate_tty_proof,
};
use machine_god_core::{CancellationToken, TerminalDimensions, TerminalSignal};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use std::ffi::OsString;
use std::io::Read;
use std::num::NonZeroU32;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::task::Poll;
use std::time::{Duration, Instant};

/// Explicit launch authority, already resolved/authorized by the host. The
/// program/argv may source the ordinary shared startup bootstrap; no shell
/// selector, environment lookup, or model-owned path is interpreted here.
pub(crate) struct TerminalTmuxLaunchRequest {
    pub(crate) executable: PathBuf,
    pub(crate) helper: TerminalPtyHelper,
    pub(crate) capture_helper: TerminalPtyHelper,
    pub(crate) program: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) initial_source: Option<String>,
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) cwd: OwnedFd,
    pub(crate) cwd_path: PathBuf,
    pub(crate) artifacts: OwnedFd,
    pub(crate) artifact_path: PathBuf,
    pub(crate) dimensions: TerminalPtyDimensions,
    pub(crate) timeout: Duration,
}

/// The directly retained foreground server owns one newly created namespace.
/// Failed retirement keeps the same Child and descriptors for another attempt.
struct NativeTerminalTmuxServer {
    helper: TerminalPtyHelper,
    child: Option<Child>,
    executable: PathBuf,
    environment: ValidatedBackgroundEnvironment,
    artifacts: Artifacts,
    socket: PathBuf,
}
impl NativeTerminalTmuxServer {
    fn start(
        helper: TerminalPtyHelper,
        executable: PathBuf,
        environment: ValidatedBackgroundEnvironment,
        artifacts: Artifacts,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        check_deadline(deadline, cancellation).map_err(gate_error)?;
        let socket = artifacts.path.join(format!("t-{}", artifacts.namespace));
        artifacts.validate()?;
        if !matches!(
            rustix::fs::statat(
                &artifacts.directory,
                socket.file_name().ok_or(TerminalTmuxLaunchError::Invalid)?,
                AtFlags::SYMLINK_NOFOLLOW,
            ),
            Err(rustix::io::Errno::NOENT)
        ) {
            return Err(TerminalTmuxLaunchError::Identity);
        }
        let child = crate::terminal_tmux_helper::relative_command(
            &helper,
            &artifacts.directory,
            &artifacts.path,
            &executable,
            &[
                "-D".into(),
                "-S".into(),
                socket
                    .file_name()
                    .ok_or(TerminalTmuxLaunchError::Invalid)?
                    .to_owned(),
                "-f".into(),
                "/dev/null".into(),
            ],
        )?
        .env_clear()
        .envs(environment.entries().iter().cloned())
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(process_error)?;
        let mut server = Self {
            helper,
            child: Some(child),
            executable,
            environment,
            artifacts,
            socket,
        };
        loop {
            check_deadline(deadline, cancellation).map_err(gate_error)?;
            if server
                .child
                .as_mut()
                .ok_or(TerminalTmuxLaunchError::Process)?
                .try_wait()
                .map_err(process_error)?
                .is_some()
            {
                return Err(TerminalTmuxLaunchError::Process);
            }
            match server.artifacts.remember_socket(&server.socket) {
                Ok(()) => break,
                Err(TerminalTmuxLaunchError::Process) => std::thread::sleep(PAUSE),
                Err(error) => return Err(error),
            }
        }
        Ok(server)
    }

    fn command(
        &mut self,
        arguments: &[OsString],
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<TerminalTmuxReply> {
        self.artifacts.validate()?;
        self.artifacts.validate_socket(&self.socket)?;
        check_deadline(deadline, cancellation).map_err(gate_error)?;
        let mut fields = vec![
            OsString::from("-N"),
            "-S".into(),
            self.socket
                .file_name()
                .ok_or(TerminalTmuxLaunchError::Invalid)?
                .to_owned(),
            "-f".into(),
            "/dev/null".into(),
        ];
        fields.extend_from_slice(arguments);
        let mut command = crate::terminal_tmux_helper::relative_command(
            &self.helper,
            &self.artifacts.directory,
            &self.artifacts.path,
            &self.executable,
            &fields,
        )?;
        command
            .env_clear()
            .envs(self.environment.entries().iter().cloned())
            .current_dir("/");
        let mut child =
            CommandProcess::spawn(command, Vec::new(), deadline).map_err(process_error)?;
        loop {
            check_deadline(deadline, cancellation).map_err(gate_error)?;
            match child.poll() {
                Poll::Ready(reply) => {
                    child.abort().map_err(process_error)?;
                    return reply.map_err(process_error);
                }
                Poll::Pending => std::thread::sleep(PAUSE),
            }
        }
    }

    fn retire(&mut self) -> Result<()> {
        if let Some(child) = self.child.as_mut() {
            if child.try_wait().map_err(process_error)?.is_none() {
                child.kill().map_err(process_error)?;
            }
            child.wait().map_err(process_error)?;
            self.child.take();
        }
        self.artifacts.cleanup()
    }
}
impl Drop for NativeTerminalTmuxServer {
    fn drop(&mut self) {
        let _ = self.retire();
    }
}

/// Constructed only by a successful private server/pane handshake. Production
/// acquisition challenges the blocked helper through the native process guard;
/// a deserialized PID cannot create this proof.
struct AuthenticatedTerminalTmuxPane {
    identity: TerminalTmuxIdentity,
    channel: UnixStream,
    nonce: [u8; 32],
    challenged: bool,
}
#[cfg(test)]
impl AuthenticatedTerminalTmuxPane {
    fn challenge(&mut self, deadline: Instant, cancellation: &CancellationToken) -> Result<()> {
        if self.challenged {
            return Err(TerminalTmuxLaunchError::Protocol);
        }
        let mut challenge = [0; 32];
        getrandom::fill(&mut challenge).map_err(process_error)?;
        write_gate(&mut self.channel, &challenge, deadline, cancellation).map_err(gate_error)?;
        let mut response = [0; 32];
        read_gate(&mut self.channel, &mut response, deadline, cancellation).map_err(gate_error)?;
        for (byte, nonce) in response.iter_mut().zip(self.nonce) {
            *byte ^= nonce;
        }
        if response != challenge {
            return Err(TerminalTmuxLaunchError::Identity);
        }
        self.challenged = true;
        Ok(())
    }
}

pub(crate) struct PreparedTerminalTmuxLaunch {
    #[cfg(target_os = "macos")]
    inventory: Option<crate::process_inventory_helper::PreparedProcessInventory>,
    server: NativeTerminalTmuxServer,
    pane: AuthenticatedTerminalTmuxPane,
    capture: Option<UnixStream>,
    capture_completion: CaptureCompletion,
    cwd: OwnedFd,
    cwd_path: PathBuf,
    deadline: Instant,
    initial_source: Option<crate::terminal_helper::TerminalStartupInput>,
    echo: Option<OwnedFd>,
    tty: Option<OwnedFd>,
}
impl PreparedTerminalTmuxLaunch {
    #[cfg(test)]
    fn prepare(
        request: TerminalTmuxLaunchRequest,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        let deadline = Instant::now()
            .checked_add(request.timeout)
            .ok_or(TerminalTmuxLaunchError::Invalid)?;
        Self::prepare_until(request, deadline, cancellation)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "The ordered acquisition keeps every pre-commit native resource under one owner."
    )]
    pub(crate) fn prepare_until(
        request: TerminalTmuxLaunchRequest,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        if request.timeout.is_zero()
            || request.timeout > crate::terminal_helper::MAX_STARTUP_TIMEOUT
            || !request.executable.is_absolute()
            || request.executable.as_os_str().as_bytes().len() > 4096
            || request.executable.as_os_str().as_bytes().contains(&0)
        {
            return Err(TerminalTmuxLaunchError::Invalid);
        }
        let deadline = deadline.min(Instant::now() + request.timeout);
        check_deadline(deadline, cancellation).map_err(gate_error)?;
        #[cfg(target_os = "macos")]
        let inventory = request
            .helper
            .inventory_helper()
            .map(|helper| helper.prepare(deadline, cancellation))
            .transpose()
            .map_err(gate_error)?;
        validate_cwd(&request.cwd, &request.cwd_path)?;
        let cwd_identity =
            startup_directory_identity(&rustix::fs::fstat(&request.cwd).map_err(process_error)?);
        let mut pty_request = TerminalPtyRequest::new(
            request.program,
            request.arguments,
            request.environment.clone(),
            rustix::io::fcntl_dupfd_cloexec(&request.cwd, 3).map_err(process_error)?,
            request.dimensions,
        )
        .map_err(process_error)?;
        if let Some(source) = &request.initial_source {
            pty_request = pty_request
                .with_startup_source(source.clone())
                .map_err(process_error)?;
        }
        let frame = pty_request.frame().map_err(process_error)?;
        let environment =
            ValidatedBackgroundEnvironment::new(request.environment).map_err(process_error)?;
        let mut artifacts = Artifacts::new(request.artifacts, request.artifact_path)?;
        let (pane_listener, pane_nonce, pane_path) =
            artifacts.listener("p", &request.helper, deadline, cancellation)?;
        let (capture_listener, capture_nonce, capture_path) =
            artifacts.listener("c", &request.helper, deadline, cancellation)?;
        let child_arguments = helper_arguments(
            &request.helper,
            "shell",
            &pane_path,
            &pane_nonce,
            &cwd_identity,
        )?;
        let child_frame = helper_frame(&child_arguments)?;
        let helper_arguments = helper_arguments(
            &request.helper,
            "pane",
            &pane_path,
            &pane_nonce,
            &cwd_identity,
        )?;
        let capture_command = helper_command(
            &request.capture_helper,
            "capture",
            &capture_path,
            &capture_nonce,
            "-",
        )?;
        let mut server = NativeTerminalTmuxServer::start(
            request.helper.clone(),
            request.executable,
            environment,
            artifacts,
            deadline,
            cancellation,
        )?;
        let version = server.command(&["-V".into()], deadline, cancellation)?;
        if !version.success || !crate::terminal_tmux::compatible_version(&version.output) {
            return Err(TerminalTmuxLaunchError::Invalid);
        }
        let session = format!("machine-god-{}", server.artifacts.namespace);
        let cwd_format = request
            .cwd_path
            .to_str()
            .ok_or(TerminalTmuxLaunchError::Invalid)?
            .replace('#', "##");
        let mut arguments: Vec<OsString> = [
            "new-session",
            "-d",
            "-s",
            &session,
            "-x",
            &request.dimensions.columns.to_string(),
            "-y",
            &request.dimensions.rows.to_string(),
            "-c",
            &cwd_format,
            "-P",
            "-F",
            "#{pane_id}|#{pane_pid}|#{pane_tty}",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        arguments.extend(helper_arguments);
        let reply = server.command(&arguments, deadline, cancellation)?;
        if !reply.success || reply.output.len() > 512 {
            return Err(TerminalTmuxLaunchError::Protocol);
        }
        let text = std::str::from_utf8(&reply.output)
            .map_err(process_error)?
            .strip_suffix('\n')
            .ok_or(TerminalTmuxLaunchError::Protocol)?;
        let (pane_name, pid) = text
            .split_once('|')
            .ok_or(TerminalTmuxLaunchError::Protocol)?;
        let (pid, tty_path) = pid
            .split_once('|')
            .ok_or(TerminalTmuxLaunchError::Protocol)?;
        let pid = NonZeroU32::new(pid.parse().map_err(process_error)?)
            .ok_or(TerminalTmuxLaunchError::Protocol)?;
        let identity =
            TerminalTmuxIdentity::new(server.artifacts.namespace.clone(), pane_name.into(), pid)
                .map_err(process_error)?;
        let (channel, _) = authenticate(
            &pane_listener,
            &pane_nonce,
            Some(pid),
            deadline,
            cancellation,
        )?;
        let mut pane = AuthenticatedTerminalTmuxPane {
            identity,
            channel,
            nonce: pane_nonce,
            challenged: false,
        };
        let tty = rustix::fs::open(
            tty_path,
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(process_error)?;
        read_tty_proof(&mut pane.channel, &tty, pid, deadline, cancellation)?;
        let echo = if request.initial_source.is_some() {
            Some(rustix::io::fcntl_dupfd_cloexec(&tty, 3).map_err(process_error)?)
        } else {
            None
        };
        write_gate(
            &mut pane.channel,
            &[u8::from(request.initial_source.is_some())],
            deadline,
            cancellation,
        )
        .map_err(gate_error)?;
        write_gate(&mut pane.channel, &frame, deadline, cancellation).map_err(gate_error)?;
        write_gate(&mut pane.channel, &child_frame, deadline, cancellation).map_err(gate_error)?;
        let mut ready = [0];
        read_gate(&mut pane.channel, &mut ready, deadline, cancellation).map_err(gate_error)?;
        if ready != [READY] {
            return Err(TerminalTmuxLaunchError::Protocol);
        }
        for fields in [
            vec![
                "set-option",
                "-t",
                &session,
                NAMESPACE_OPTION,
                pane.identity.namespace(),
            ],
            vec!["set-option", "-t", &session, "remain-on-exit", "on"],
            vec![
                "pipe-pane",
                "-O",
                "-t",
                pane.identity.pane(),
                &capture_command,
            ],
        ] {
            let arguments = fields.into_iter().map(Into::into).collect::<Vec<_>>();
            if !server.command(&arguments, deadline, cancellation)?.success {
                return Err(TerminalTmuxLaunchError::Process);
            }
        }
        let (mut capture, capture_pid) = authenticate(
            &capture_listener,
            &capture_nonce,
            None,
            deadline,
            cancellation,
        )?;
        let completion_nonce = nonce()?;
        write_gate(&mut capture, &completion_nonce, deadline, cancellation).map_err(gate_error)?;
        let (completion, _) = authenticate(
            &capture_listener,
            &completion_nonce,
            Some(capture_pid),
            deadline,
            cancellation,
        )?;
        Ok(Self {
            server,
            #[cfg(target_os = "macos")]
            inventory,
            pane,
            capture: Some(capture),
            capture_completion: CaptureCompletion::new(completion),
            cwd: request.cwd,
            cwd_path: request.cwd_path,
            deadline,
            initial_source: request
                .initial_source
                .as_deref()
                .map(crate::terminal_helper::TerminalStartupInput::new)
                .transpose()
                .map_err(gate_error)?,
            echo,
            tty: Some(tty),
        })
    }

    /// Native authority must already be retained after `pane.challenge`.
    /// Returns the prepared pieces without discarding ownership on cancellation
    /// after COMMIT. The caller installs them into one retained backend owner.
    fn commit(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(NativeTerminalTmuxControl, UnixStream)> {
        if !self.pane.challenged || self.capture.is_none() {
            return Err(TerminalTmuxLaunchError::Protocol);
        }
        validate_cwd(&self.cwd, &self.cwd_path)?;
        let mut control = NativeTerminalTmuxControl::new_with_helper(
            self.server.helper.clone(),
            self.server.executable.clone(),
            self.server.environment.clone(),
            rustix::io::fcntl_dupfd_cloexec(&self.server.artifacts.directory, 3)
                .map_err(process_error)?,
            &self.server.socket,
            self.pane.identity.clone(),
        )
        .map_err(process_error)?;
        if let Some(source) = &self.initial_source {
            for command in [
                TerminalTmuxCommand::Load(source.initial().to_vec()),
                TerminalTmuxCommand::Paste,
            ] {
                control
                    .begin(command, self.deadline)
                    .map_err(process_error)?;
                loop {
                    check_deadline(self.deadline, cancellation).map_err(gate_error)?;
                    match control.poll() {
                        Poll::Ready(Ok(reply)) if reply.success => break,
                        Poll::Ready(_) => return Err(TerminalTmuxLaunchError::Process),
                        Poll::Pending => std::thread::sleep(PAUSE),
                    }
                }
            }
        }
        write_gate(
            &mut self.pane.channel,
            &[COMMIT],
            self.deadline,
            cancellation,
        )
        .map_err(gate_error)?;
        Ok((
            control,
            self.capture
                .take()
                .ok_or(TerminalTmuxLaunchError::Protocol)?,
        ))
    }

    pub(crate) fn identity(&self) -> &TerminalTmuxIdentity {
        &self.pane.identity
    }

    /// Acquires the live helper incarnation before the final shell release.
    /// The resulting backend owns every native process and cleanup obligation.
    pub(crate) fn commit_owned(
        mut self,
        cancellation: &CancellationToken,
    ) -> Result<NativeTerminalTmuxBackend> {
        let authority = AuthenticatedTerminalProcess::authenticate(
            self.pane.identity.pid(),
            &mut self.pane.channel,
            &self.pane.nonce,
            self.deadline,
            cancellation,
        )
        .map_err(process_error)?;
        #[cfg(target_os = "macos")]
        let authority = authority.with_inventory_helper(self.inventory.take());
        let mut authority = authority;
        authority.retain_self_as_anchor().map_err(process_error)?;
        self.pane.challenged = true;
        let (control, capture) = self.commit(cancellation)?;
        let process = NativeTerminalTmuxProcess {
            authority,
            channel: self.pane.channel,
            identity: self.pane.identity,
            outcome: None,
            frame: [0; 3],
            used: 0,
            retiring: false,
        };
        let backend = TerminalTmuxBackend::attach_until(control, process, capture, self.deadline)
            .map_err(process_error)?;
        Ok(NativeTerminalTmuxBackend {
            initial_source: self.initial_source.filter(|source| !source.complete()),
            startup_deadline: self.deadline,
            backend,
            server: self.server,
            echo: self.echo,
            tty: self.tty,
            capture_completion: self.capture_completion,
        })
    }
}

struct NativeTerminalTmuxProcess {
    // Authority must be dropped before the helper channel, including unwind.
    authority: AuthenticatedTerminalProcess,
    channel: UnixStream,
    identity: TerminalTmuxIdentity,
    outcome: Option<TerminalPtyStatus>,
    frame: [u8; 3],
    used: usize,
    retiring: bool,
}
impl NativeTerminalTmuxProcess {
    fn poll_job(&mut self) -> std::result::Result<TerminalPtyStatus, TerminalTmuxError> {
        if let Some(outcome) = self.outcome {
            return Ok(outcome);
        }
        match self.channel.read(&mut self.frame[self.used..]) {
            Ok(0) => return Err(TerminalTmuxError::Protocol),
            Ok(count) => self.used += count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(TerminalPtyStatus::Running);
            }
            Err(_) => return Err(TerminalTmuxError::Protocol),
        }
        if self.used != self.frame.len() {
            return Ok(TerminalPtyStatus::Running);
        }
        let outcome = match self.frame {
            [b'O', 0, code] => TerminalPtyStatus::Exited(i32::from(code)),
            [b'O', 1, signal @ 1..=127] => TerminalPtyStatus::Signalled(i32::from(signal)),
            _ => return Err(TerminalTmuxError::Protocol),
        };
        self.outcome = Some(outcome);
        Ok(outcome)
    }
}
impl TerminalTmuxProcess for NativeTerminalTmuxProcess {
    fn validate(
        &mut self,
        identity: &TerminalTmuxIdentity,
    ) -> std::result::Result<(), TerminalTmuxError> {
        if identity != &self.identity {
            return Err(TerminalTmuxError::Identity);
        }
        self.authority
            .validate_identity(identity.pid())
            .map_err(|_| TerminalTmuxError::Identity)
    }
    fn signal(&mut self, signal: TerminalSignal) -> std::result::Result<(), TerminalTmuxError> {
        let signal = match signal {
            TerminalSignal::Hangup => BackgroundProcessSignal::Hangup,
            TerminalSignal::Interrupt => BackgroundProcessSignal::Interrupt,
            TerminalSignal::Quit => BackgroundProcessSignal::Quit,
            TerminalSignal::Terminate => BackgroundProcessSignal::Terminate,
            TerminalSignal::Kill => BackgroundProcessSignal::Kill,
        };
        self.authority
            .signal(signal)
            .map_err(|_| TerminalTmuxError::Command)
    }
    fn signal_retained_cleanup(
        &mut self,
        identity: &TerminalTmuxIdentity,
        signal: TerminalSignal,
    ) -> std::result::Result<(), TerminalTmuxError> {
        if identity != &self.identity {
            return Err(TerminalTmuxError::Identity);
        }
        let signal = match signal {
            TerminalSignal::Hangup => BackgroundProcessSignal::Hangup,
            TerminalSignal::Interrupt => BackgroundProcessSignal::Interrupt,
            TerminalSignal::Quit => BackgroundProcessSignal::Quit,
            TerminalSignal::Terminate => BackgroundProcessSignal::Terminate,
            TerminalSignal::Kill => BackgroundProcessSignal::Kill,
        };
        self.authority
            .signal_retained(signal)
            .map_err(|_| TerminalTmuxError::Cleanup)
    }
    fn is_absent(&mut self) -> std::result::Result<bool, TerminalTmuxError> {
        if !self.retiring {
            if self.poll_job()? == TerminalPtyStatus::Running
                || !self
                    .authority
                    .jobs_absent()
                    .map_err(|_| TerminalTmuxError::Cleanup)?
            {
                return Ok(false);
            }
            self.authority
                .retire_anchor()
                .map_err(|_| TerminalTmuxError::Cleanup)?;
            self.retiring = true;
        }
        self.authority
            .is_absent()
            .map_err(|_| TerminalTmuxError::Cleanup)
    }
    fn outcome(&mut self) -> std::result::Result<Option<TerminalPtyStatus>, TerminalTmuxError> {
        self.poll_job().map(Some)
    }
    fn job_status(&mut self) -> std::result::Result<Option<TerminalPtyStatus>, TerminalTmuxError> {
        self.poll_job().map(Some)
    }
}

/// One raw backend owner. The startup bootstrap may wrap this value without
/// changing its paste, input settlement, original-tty, or native lifetime rules.
pub(crate) struct NativeTerminalTmuxBackend {
    initial_source: Option<crate::terminal_helper::TerminalStartupInput>,
    startup_deadline: Instant,
    backend: TerminalTmuxBackend<NativeTerminalTmuxControl, NativeTerminalTmuxProcess>,
    server: NativeTerminalTmuxServer,
    echo: Option<OwnedFd>,
    tty: Option<OwnedFd>,
    capture_completion: CaptureCompletion,
}
impl NativeTerminalTmuxBackend {
    fn flush_initial_source(&mut self) -> std::result::Result<(), ()> {
        let Some(source) = self.initial_source.as_mut() else {
            return Ok(());
        };
        if Instant::now() >= self.startup_deadline {
            #[cfg(test)]
            eprintln!(
                "tmux bootstrap source deadline expired: pending_bytes={:?}",
                source.pending().map(<[u8]>::len)
            );
            return Err(());
        }
        if let Some(bytes) = source.pending() {
            let receipt = self.backend.write(bytes)?;
            source.advance(receipt.bytes_written()).map_err(|error| {
                let _ = error;
                #[cfg(test)]
                eprintln!("tmux bootstrap source advance failed: {error:?}");
            })?;
            if receipt.stdin_closed() {
                #[cfg(test)]
                eprintln!("tmux bootstrap input closed");
                return Err(());
            }
        }
        if source.complete() {
            self.initial_source = None;
        }
        Ok(())
    }
}
impl TerminalSessionBackend for NativeTerminalTmuxBackend {
    fn restore_startup_echo(&mut self) -> std::result::Result<(), ()> {
        if let Some(echo) = self.echo.as_ref() {
            set_echo(echo, true).map_err(|_| ())?;
        }
        self.echo.take();
        Ok(())
    }
    fn read(&mut self, bytes: &mut [u8]) -> std::result::Result<TerminalPtyRead, ()> {
        self.flush_initial_source()?;
        let read = self.backend.read(bytes)?;
        if let Some(source) = self.initial_source.as_mut() {
            source.observe(&bytes[..read.bytes_read]);
        }
        Ok(read)
    }
    fn write(&mut self, bytes: &[u8]) -> std::result::Result<BackgroundInputReceipt, ()> {
        if self.initial_source.is_some() {
            return Ok(BackgroundInputReceipt::new(
                0,
                false,
                crate::background_input::BackgroundInputStatus::Backpressure,
            ));
        }
        self.backend.write(bytes)
    }
    fn write_with_paste(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> std::result::Result<BackgroundInputReceipt, ()> {
        if self.initial_source.is_some() {
            return Ok(BackgroundInputReceipt::new(
                0,
                false,
                crate::background_input::BackgroundInputStatus::Backpressure,
            ));
        }
        self.backend.write_with_paste(bytes, paste)
    }
    fn input_write_limit(&self) -> usize {
        self.backend.input_write_limit()
    }
    fn settle_write(
        &mut self,
        bytes: &[u8],
        paste: bool,
    ) -> Poll<std::result::Result<BackgroundInputReceipt, ()>> {
        self.backend.settle_write(bytes, paste)
    }
    fn status(&mut self) -> std::result::Result<TerminalPtyStatus, ()> {
        self.backend.status()
    }
    fn resize(&mut self, dimensions: &TerminalDimensions) -> std::result::Result<(), ()> {
        self.backend.resize(dimensions)?;
        let tty = self.tty.as_ref().ok_or(())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let size = rustix::termios::tcgetwinsize(tty).map_err(|_| ())?;
            if size.ws_row == dimensions.rows() && size.ws_col == dimensions.columns() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(());
            }
            std::thread::sleep(PAUSE);
        }
    }
    fn signal(&mut self, signal: TerminalSignal) -> std::result::Result<(), ()> {
        self.backend.signal(signal)
    }
    fn signal_may_discard_output(&self) -> bool {
        self.backend.signal_may_discard_output()
    }
    fn close(
        &mut self,
        force: bool,
        output: &mut dyn FnMut(&[u8]),
    ) -> std::result::Result<TerminalPtyClose, ()> {
        self.initial_source = None;
        self.echo.take();
        self.tty.take();
        let mut receipt = self.backend.close(force, output)?;
        receipt.output_incomplete |= !self.capture_completion.finish();
        self.server.retire().map_err(|_| ())?;
        Ok(receipt)
    }
}
impl Drop for NativeTerminalTmuxBackend {
    fn drop(&mut self) {
        let _ = self.close(true, &mut |_| {});
    }
}

struct CaptureCompletion {
    channel: UnixStream,
    frame: [u8; 2],
    used: usize,
    complete: Option<bool>,
}
impl CaptureCompletion {
    fn new(channel: UnixStream) -> Self {
        Self {
            channel,
            frame: [0; 2],
            used: 0,
            complete: None,
        }
    }
    fn finish(&mut self) -> bool {
        if let Some(complete) = self.complete {
            return complete;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let complete = loop {
            match self.channel.read(&mut self.frame[self.used..]) {
                Ok(0) => break false,
                Ok(count) => {
                    self.used += count;
                    if self.used == self.frame.len() {
                        break self.frame == [b'C', 0];
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => break false,
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(PAUSE);
        };
        self.complete = Some(complete);
        complete
    }
}

fn authenticate(
    listener: &UnixListener,
    nonce: &[u8; 32],
    expected_pid: Option<NonZeroU32>,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(UnixStream, NonZeroU32)> {
    let mut channel = loop {
        check_deadline(deadline, cancellation).map_err(gate_error)?;
        match listener.accept() {
            Ok((channel, _)) => break channel,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(PAUSE);
            }
            Err(error) => return Err(process_error(error)),
        }
    };
    channel.set_nonblocking(true).map_err(process_error)?;
    let mut proof = [0; PROOF_BYTES];
    read_gate(&mut channel, &mut proof, deadline, cancellation).map_err(gate_error)?;
    if proof[..32] != nonce[..]
        || expected_pid.is_some_and(|pid| proof[32..] != pid.get().to_be_bytes())
        || proof[32..] == [0; 4]
    {
        return Err(TerminalTmuxLaunchError::Identity);
    }
    write_gate(&mut channel, &[READY], deadline, cancellation).map_err(gate_error)?;
    let pid = NonZeroU32::new(u32::from_be_bytes(
        proof[32..].try_into().map_err(process_error)?,
    ))
    .ok_or(TerminalTmuxLaunchError::Protocol)?;
    Ok((channel, pid))
}

fn read_tty_proof(
    channel: &mut UnixStream,
    tty: &OwnedFd,
    expected_pid: NonZeroU32,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<()> {
    let mut proof = [0; TTY_PROOF_BYTES];
    read_gate(channel, &mut proof, deadline, cancellation).map_err(gate_error)?;
    validate_tty_proof(&proof, tty, expected_pid.get())
}

fn helper_frame(arguments: &[OsString]) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    payload.extend_from_slice(
        &u32::try_from(arguments.len())
            .map_err(process_error)?
            .to_be_bytes(),
    );
    for argument in arguments {
        payload.extend_from_slice(
            &u32::try_from(argument.as_bytes().len())
                .map_err(process_error)?
                .to_be_bytes(),
        );
        payload.extend_from_slice(argument.as_bytes());
    }
    if payload.len() > MAX_HELPER_FRAME {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    let mut frame = u32::try_from(payload.len())
        .map_err(process_error)?
        .to_be_bytes()
        .to_vec();
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn validate_cwd(directory: &OwnedFd, path: &Path) -> Result<()> {
    if !path.is_absolute() || path.as_os_str().as_bytes().len() > 4096 {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    let held = rustix::fs::fstat(directory).map_err(process_error)?;
    let observed = rustix::fs::statat(rustix::fs::CWD, path, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(process_error)?;
    if FileType::from_raw_mode(held.st_mode) != FileType::Directory
        || FileType::from_raw_mode(observed.st_mode) != FileType::Directory
        || startup_directory_identity(&held) != startup_directory_identity(&observed)
    {
        return Err(TerminalTmuxLaunchError::Identity);
    }
    Ok(())
}

fn helper_arguments(
    helper: &TerminalPtyHelper,
    kind: &str,
    socket: &Path,
    nonce: &[u8; 32],
    cwd_identity: &str,
) -> Result<Vec<OsString>> {
    let mut arguments = vec![helper.program().as_os_str().to_owned()];
    arguments.extend(helper.arguments().iter().cloned());
    arguments.extend([
        kind.into(),
        socket.as_os_str().to_owned(),
        std::str::from_utf8(nonce).map_err(process_error)?.into(),
        cwd_identity.into(),
    ]);
    if arguments
        .iter()
        .map(|arg| arg.as_bytes().len())
        .sum::<usize>()
        > 8192
    {
        return Err(TerminalTmuxLaunchError::Invalid);
    }
    Ok(arguments)
}
fn helper_command(
    helper: &TerminalPtyHelper,
    kind: &str,
    socket: &Path,
    nonce: &[u8; 32],
    cwd_identity: &str,
) -> Result<String> {
    let arguments = helper_arguments(helper, kind, socket, nonce, cwd_identity)?;
    let mut command = String::from("exec");
    for argument in arguments {
        command.push_str(" '");
        command.push_str(
            &argument
                .to_str()
                .ok_or(TerminalTmuxLaunchError::Invalid)?
                .replace('\'', "'\\''"),
        );
        command.push('\'');
    }
    // pipe-pane expands tmux formats even inside shell quotes. Escape that
    // independent layer so injected helper/artifact paths remain exact data.
    Ok(command.replace('#', "##"))
}

struct Artifacts {
    directory: OwnedFd,
    path: PathBuf,
    namespace: String,
    sockets: Vec<(PathBuf, String)>,
}
impl Artifacts {
    fn new(directory: OwnedFd, path: PathBuf) -> Result<Self> {
        validate_startup_directory(&directory, &path).map_err(gate_error)?;
        if !path.is_absolute()
            || std::fs::canonicalize(&path).map_err(process_error)? != path
            || path.as_os_str().as_bytes().len() > crate::terminal_helper::MAX_STARTUP_PATH_BYTES
        {
            return Err(TerminalTmuxLaunchError::Invalid);
        }
        let namespace = nonce()?;
        Ok(Self {
            directory,
            path,
            namespace: String::from_utf8(namespace.to_vec()).map_err(process_error)?,
            sockets: Vec::new(),
        })
    }
    fn validate(&self) -> Result<()> {
        validate_startup_directory(&self.directory, &self.path)
            .map(|_| ())
            .map_err(gate_error)
    }
    fn listener(
        &mut self,
        kind: &str,
        helper: &TerminalPtyHelper,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<(UnixListener, [u8; 32], PathBuf)> {
        self.validate()?;
        let nonce = nonce()?;
        let path = self.path.join(format!("{kind}-{}", self.namespace));
        let listener = crate::terminal_helper::bind_startup_listener(
            helper,
            false,
            &self.directory,
            &self.path,
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or(TerminalTmuxLaunchError::Invalid)?,
            deadline,
            cancellation,
        );
        let listener = listener.map_err(gate_error)?;
        self.remember_socket(&path)?;
        check_deadline(deadline, cancellation).map_err(gate_error)?;
        rustix::net::listen(&listener, 8).map_err(process_error)?;
        listener.set_nonblocking(true).map_err(process_error)?;
        Ok((listener, nonce, path))
    }
    fn remember_socket(&mut self, path: &Path) -> Result<()> {
        self.validate()?;
        let stat = rustix::fs::statat(
            &self.directory,
            path.file_name().ok_or(TerminalTmuxLaunchError::Invalid)?,
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(process_error)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
            || stat.st_uid != rustix::process::getuid().as_raw()
        {
            return Err(TerminalTmuxLaunchError::Identity);
        }
        // The private directory protects the bind-to-chmod interval.
        rustix::fs::chmodat(
            &self.directory,
            path.file_name().ok_or(TerminalTmuxLaunchError::Invalid)?,
            Mode::RUSR | Mode::WUSR,
            AtFlags::empty(),
        )
        .map_err(process_error)?;
        self.sockets
            .push((path.to_owned(), startup_directory_identity(&stat)));
        Ok(())
    }
    fn validate_socket(&self, path: &Path) -> Result<()> {
        let (_, expected) = self
            .sockets
            .iter()
            .find(|(item, _)| item == path)
            .ok_or(TerminalTmuxLaunchError::Identity)?;
        let stat = rustix::fs::statat(
            &self.directory,
            path.file_name().ok_or(TerminalTmuxLaunchError::Invalid)?,
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(process_error)?;
        if startup_directory_identity(&stat) != *expected
            || FileType::from_raw_mode(stat.st_mode) != FileType::Socket
        {
            return Err(TerminalTmuxLaunchError::Identity);
        }
        Ok(())
    }
    fn cleanup(&mut self) -> Result<()> {
        self.validate()?;
        while let Some((path, expected)) = self.sockets.last() {
            let name = path.file_name().ok_or(TerminalTmuxLaunchError::Cleanup)?;
            match rustix::fs::statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) if startup_directory_identity(&stat) == *expected => {
                    rustix::fs::unlinkat(&self.directory, name, AtFlags::empty())
                        .map_err(process_error)?;
                }
                Err(rustix::io::Errno::NOENT) => {}
                _ => return Err(TerminalTmuxLaunchError::Cleanup),
            }
            self.sockets.pop();
        }
        Ok(())
    }
}
impl Drop for Artifacts {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
fn nonce() -> Result<[u8; 32]> {
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(process_error)?;
    let mut nonce = [0; 32];
    for (index, byte) in random.into_iter().enumerate() {
        nonce[index * 2] = b"0123456789abcdef"[usize::from(byte >> 4)];
        nonce[index * 2 + 1] = b"0123456789abcdef"[usize::from(byte & 15)];
    }
    Ok(nonce)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let nonce = nonce().unwrap();
            let path = std::env::temp_dir().join(format!(
                "mg-tl-{}",
                std::str::from_utf8(&nonce[..12]).unwrap()
            ));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(std::fs::canonicalize(path).unwrap())
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

    fn helper() -> TerminalPtyHelper {
        if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            return TerminalPtyHelper::new(
                PathBuf::from(program),
                vec![TERMINAL_TMUX_HELPER_ARGUMENT.into()],
            )
            .unwrap()
            .with_test_inventory_helper();
        }
        let program = std::env::current_exe().unwrap();
        let script = format!(
            "export MG_TMUX_KIND=\"$1\" MG_TMUX_SOCKET=\"$2\" MG_TMUX_NONCE=\"$3\" MG_TMUX_CWD=\"$4\"; if [ \"$1\" = exec ]; then exec 2>&1; exec 1>/dev/null; fi; exec '{}' --exact terminal_tmux_startup::tests::helper_entry --nocapture",
            program.to_str().unwrap().replace('\'', "'\\''")
        );
        TerminalPtyHelper::new(
            "/bin/sh".into(),
            vec!["-c".into(), script.into(), "helper".into()],
        )
        .unwrap()
        .with_test_inventory_helper()
    }

    fn marker_helper() -> TerminalPtyHelper {
        if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            return TerminalPtyHelper::new(
                PathBuf::from(program),
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
    #[test]
    fn helper_entry() {
        let Some(kind) = std::env::var_os("MG_TMUX_KIND") else {
            return;
        };
        let result = run_terminal_tmux_helper(&[
            kind,
            std::env::var_os("MG_TMUX_SOCKET").unwrap(),
            std::env::var_os("MG_TMUX_NONCE").unwrap(),
            std::env::var_os("MG_TMUX_CWD").unwrap(),
        ]);
        std::process::exit(if result.is_ok() { 0 } else { 125 });
    }

    #[test]
    fn capture_completion_requires_exact_success_and_preserves_failure_on_retry() {
        for (frame, expected) in [
            (&b""[..], false),
            (&b"C"[..], false),
            (&b"C\x01"[..], false),
            (&b"X\x00"[..], false),
            (&b"C\x00"[..], true),
        ] {
            let (channel, mut peer) = UnixStream::pair().unwrap();
            channel.set_nonblocking(true).unwrap();
            peer.write_all(frame).unwrap();
            drop(peer);
            let mut completion = CaptureCompletion::new(channel);
            assert_eq!(completion.finish(), expected);
            assert_eq!(completion.finish(), expected);
        }
    }

    #[test]
    fn tty_receipt_requires_complete_frame_and_original_deadline_and_cancellation() {
        let tty =
            rustix::fs::open("/dev/null", OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty()).unwrap();
        let pid = NonZeroU32::new(41).unwrap();
        for length in [0, 4, TTY_PROOF_BYTES - 1] {
            let (mut channel, mut sender) = UnixStream::pair().unwrap();
            channel.set_nonblocking(true).unwrap();
            sender.write_all(&[0; TTY_PROOF_BYTES][..length]).unwrap();
            drop(sender);
            assert_eq!(
                read_tty_proof(
                    &mut channel,
                    &tty,
                    pid,
                    Instant::now() + Duration::from_secs(1),
                    &CancellationToken::new()
                ),
                Err(TerminalTmuxLaunchError::Protocol)
            );
        }
        let (mut channel, _sender) = UnixStream::pair().unwrap();
        channel.set_nonblocking(true).unwrap();
        assert_eq!(
            read_tty_proof(
                &mut channel,
                &tty,
                pid,
                Instant::now(),
                &CancellationToken::new()
            ),
            Err(TerminalTmuxLaunchError::Timeout)
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            read_tty_proof(
                &mut channel,
                &tty,
                pid,
                Instant::now() + Duration::from_secs(1),
                &cancellation
            ),
            Err(TerminalTmuxLaunchError::Cancelled)
        );
    }
    fn request(directory: &Directory, command: &str) -> Option<TerminalTmuxLaunchRequest> {
        let explicit = std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY");
        let executable = explicit.clone().map(PathBuf::from).or_else(|| {
            [
                "/opt/homebrew/bin/tmux",
                "/usr/bin/tmux",
                "/usr/local/bin/tmux",
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
        })?;
        if explicit.is_some() {
            assert!(executable.is_absolute() && executable.is_file());
        }
        Some(TerminalTmuxLaunchRequest {
            executable,
            helper: helper(),
            capture_helper: helper(),
            program: "/bin/bash".into(),
            arguments: vec![
                "--noprofile".into(),
                "--norc".into(),
                "-c".into(),
                command.into(),
            ],
            initial_source: None,
            environment: vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TERM".into(), "xterm-256color".into()),
            ],
            cwd: directory.fd(),
            cwd_path: directory.0.clone(),
            artifacts: directory.fd(),
            artifact_path: directory.0.clone(),
            dimensions: TerminalPtyDimensions {
                rows: 37,
                columns: 99,
            },
            timeout: Duration::from_secs(10),
        })
    }

    #[cfg(target_os = "linux")]
    struct CleanupObserver(OwnedFd);

    #[cfg(target_os = "linux")]
    impl CleanupObserver {
        fn from_marker(path: &Path, deadline: Instant) -> Self {
            loop {
                if let Ok(text) = std::fs::read_to_string(path)
                    && let Ok(pid) = text.parse::<i32>()
                {
                    return Self(
                        rustix::process::pidfd_open(
                            rustix::process::Pid::from_raw(pid).unwrap(),
                            rustix::process::PidfdFlags::empty(),
                        )
                        .unwrap(),
                    );
                }
                assert!(Instant::now() < deadline, "owned job marker unavailable");
                std::thread::sleep(PAUSE);
            }
        }

        fn exited(&self) -> bool {
            let mut descriptors = [rustix::event::PollFd::new(
                &self.0,
                rustix::event::PollFlags::IN,
            )];
            rustix::event::poll(&mut descriptors, Some(&rustix::event::Timespec::default()))
                .unwrap()
                != 0
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for CleanupObserver {
        fn drop(&mut self) {
            // Independently retain exact fixture cleanup if an assertion fails.
            let _ = rustix::process::pidfd_send_signal(&self.0, rustix::process::Signal::KILL);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_force_close_progresses_at_capture_capacity_before_and_after_shell_exit() {
        for shell_exits in [false, true] {
            let directory = Directory::new();
            let ending = if shell_exits { "exit 0" } else { "wait" };
            let script = format!(
                "sleep 100 & printf '%s' \"$!\" > first; while [[ ! -e release ]]; do read -r -t 0.01 unused; done; sleep 100 & printf '%s' \"$!\" > second; sleep 100 & printf '%s' \"$!\" > third; {ending}"
            );
            let Some(request) = request(&directory, &script) else {
                return;
            };
            let cancellation = CancellationToken::new();
            let mut backend = PreparedTerminalTmuxLaunch::prepare(request, &cancellation)
                .unwrap()
                .commit_owned(&cancellation)
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            let first = CleanupObserver::from_marker(&directory.0.join("first"), deadline);
            {
                let process = backend.backend.process_for_test();
                let identity = process.identity.clone();
                process.validate(&identity).unwrap();
                // Full-identity mismatches never gain retained cleanup authority,
                // including a correct PID paired with a different namespace/pane.
                for wrong in [
                    TerminalTmuxIdentity::new(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                        identity.pane().into(),
                        identity.pid(),
                    )
                    .unwrap(),
                    TerminalTmuxIdentity::new(
                        identity.namespace().into(),
                        "%987".into(),
                        identity.pid(),
                    )
                    .unwrap(),
                    TerminalTmuxIdentity::new(
                        identity.namespace().into(),
                        identity.pane().into(),
                        NonZeroU32::new(std::process::id()).unwrap(),
                    )
                    .unwrap(),
                ] {
                    assert_eq!(
                        process.signal_retained_cleanup(&wrong, TerminalSignal::Kill),
                        Err(TerminalTmuxError::Identity)
                    );
                    assert!(!first.exited());
                }
                process.authority.exhaust_capture_budget_for_test();
            }
            std::fs::write(directory.0.join("release"), b"ready").unwrap();
            let second = CleanupObserver::from_marker(&directory.0.join("second"), deadline);
            let third = CleanupObserver::from_marker(&directory.0.join("third"), deadline);
            if shell_exits {
                while backend.backend.process_for_test().poll_job().unwrap()
                    == TerminalPtyStatus::Running
                {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(PAUSE);
                }
            } else {
                assert_eq!(
                    backend.backend.process_for_test().poll_job().unwrap(),
                    TerminalPtyStatus::Running
                );
            }
            assert!(
                backend.close(true, &mut |_| {}).is_err(),
                "incomplete inventory is not quiescence"
            );
            assert!(
                backend.server.child.is_some(),
                "failed close retains namespace ownership"
            );
            while !first.exited() {
                assert!(
                    Instant::now() < deadline,
                    "retained exact pin was not killed"
                );
                std::thread::sleep(PAUSE);
            }
            assert!(
                !second.exited() || !third.exited(),
                "partial inventory cannot signal unproved jobs"
            );
            // No quota is increased: each retry must free settled pins and make
            // bounded progress until a complete inventory authorizes retirement.
            let receipt = loop {
                if let Ok(receipt) = backend.close(true, &mut |_| {}) {
                    break receipt;
                }
                assert!(
                    Instant::now() < deadline,
                    "retained cleanup did not converge"
                );
                std::thread::sleep(PAUSE);
            };
            assert_ne!(receipt.status, TerminalPtyStatus::Running);
            assert!(second.exited() && third.exited());
            assert!(backend.server.child.is_none());
        }
    }

    #[test]
    fn real_gated_pane_cannot_execute_before_challenge_commit_and_captures_exact_bytes() {
        let directory = Directory::new();
        let Some(request) = request(
            &directory,
            "stty -opost; printf '\\000\\377\\033[31mRAW\\n'; stty size; printf yes > executed; exit 7",
        ) else {
            return;
        };
        let cancellation = CancellationToken::new();
        let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        assert!(!directory.0.join("executed").exists());
        assert!(matches!(
            prepared.commit(&cancellation),
            Err(TerminalTmuxLaunchError::Protocol)
        ));
        prepared
            .pane
            .challenge(prepared.deadline, &cancellation)
            .unwrap();
        assert!(!directory.0.join("executed").exists());
        let (_control, mut capture) = prepared.commit(&cancellation).unwrap();
        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut buffer = [0; 1024];
        loop {
            match capture.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    output.extend_from_slice(&buffer[..count]);
                    if output.ends_with(b"37 99\n") {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "capture did not close");
                    std::thread::sleep(PAUSE);
                }
                Err(error) => panic!("capture failed: {error}"),
            }
        }
        assert_eq!(output, b"\0\xff\x1b[31mRAW\n37 99\n");
        let mut outcome = [0; 3];
        read_gate(
            &mut prepared.pane.channel,
            &mut outcome,
            deadline,
            &cancellation,
        )
        .unwrap();
        assert_eq!(outcome, [b'O', 0, 7]);
        assert_eq!(std::fs::read(directory.0.join("executed")).unwrap(), b"yes");
        let reply = prepared
            .server
            .command(
                &[
                    "display-message".into(),
                    "-p".into(),
                    "-t".into(),
                    prepared.pane.identity.pane().into(),
                    "#{pane_dead}".into(),
                ],
                deadline,
                &cancellation,
            )
            .unwrap();
        assert!(reply.success);
        assert_eq!(
            reply.output, b"0\n",
            "the retained helper is not the completed job"
        );
        drop(capture);
        prepared.server.retire().unwrap();
        assert!(prepared.server.child.is_none());
        assert!(prepared.server.artifacts.sockets.is_empty());
    }
    #[test]
    fn dropping_prepared_pane_and_cancelling_commit_have_no_shell_effects() {
        for cancel in [false, true] {
            let directory = Directory::new();
            let Some(request) = request(&directory, "printf forbidden > executed") else {
                return;
            };
            let cancellation = CancellationToken::new();
            let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
            if cancel {
                prepared
                    .pane
                    .challenge(prepared.deadline, &cancellation)
                    .unwrap();
                cancellation.cancel();
                assert!(matches!(
                    prepared.commit(&cancellation),
                    Err(TerminalTmuxLaunchError::Cancelled)
                ));
            }
            drop(prepared);
            assert!(!directory.0.join("executed").exists());
            assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
        }
    }
    #[test]
    fn helper_rejects_unbounded_and_malformed_private_arguments_before_connect() {
        assert_eq!(
            run_terminal_tmux_helper(&[]),
            Err(TerminalTmuxLaunchError::Invalid)
        );
        for kind in ["other", "pane", "capture"] {
            assert_eq!(
                run_terminal_tmux_helper(&[
                    kind.into(),
                    "relative".into(),
                    "x".repeat(32).into(),
                    "-".into()
                ]),
                Err(TerminalTmuxLaunchError::Invalid)
            );
        }
    }

    #[test]
    fn real_supervisor_distinguishes_fast_signal_from_same_numeric_exit_code() {
        for (script, expected) in [
            ("kill -KILL $$", [b'O', 1, 9]),
            ("exit 137", [b'O', 0, 137]),
        ] {
            let directory = Directory::new();
            let Some(request) = request(&directory, script) else {
                return;
            };
            let cancellation = CancellationToken::new();
            let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
            prepared
                .pane
                .challenge(prepared.deadline, &cancellation)
                .unwrap();
            let (_control, capture) = prepared.commit(&cancellation).unwrap();
            let mut outcome = [0; 3];
            read_gate(
                &mut prepared.pane.channel,
                &mut outcome,
                Instant::now() + Duration::from_secs(10),
                &cancellation,
            )
            .unwrap();
            assert_eq!(outcome, expected);
            drop(capture);
            prepared.server.retire().unwrap();
        }
    }

    #[test]
    fn real_commandless_source_is_queued_before_commit_without_echo_or_a_nested_tty() {
        let directory = Directory::new();
        let Some(mut request) = request(&directory, "unused") else {
            return;
        };
        request.arguments = vec!["--noprofile".into(), "--norc".into(), "-i".into()];
        request.initial_source = Some("printf 'SOURCE\\n'; exit 0\n".into());
        request.environment.push(("PS1".into(), "".into()));
        let cancellation = CancellationToken::new();
        let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        assert!(
            !rustix::termios::tcgetattr(prepared.echo.as_ref().unwrap())
                .unwrap()
                .local_modes
                .contains(rustix::termios::LocalModes::ECHO)
        );
        prepared
            .pane
            .challenge(prepared.deadline, &cancellation)
            .unwrap();
        let (_control, mut capture) = prepared.commit(&cancellation).unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 1024];
        let deadline = Instant::now() + Duration::from_secs(10);
        while !bytes.windows(6).any(|bytes| bytes == b"SOURCE") {
            match capture.read(&mut buffer) {
                Ok(count) if count != 0 => bytes.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(PAUSE);
                }
                _ => panic!("source capture closed early"),
            }
            assert!(Instant::now() < deadline);
        }
        assert!(!String::from_utf8_lossy(&bytes).contains("printf"));
        let mut outcome = [0; 3];
        read_gate(
            &mut prepared.pane.channel,
            &mut outcome,
            deadline,
            &cancellation,
        )
        .unwrap();
        assert_eq!(outcome, [b'O', 0, 0]);
        set_echo(prepared.echo.as_ref().unwrap(), true).unwrap();
        prepared.echo.take();
        drop(capture);
        prepared.server.retire().unwrap();
    }

    #[test]
    fn real_capture_overload_closes_with_explicit_loss_instead_of_blocking_tmux() {
        let directory = Directory::new();
        let Some(request) = request(
            &directory,
            "/usr/bin/head -c 8388608 /dev/zero; printf done > done",
        ) else {
            return;
        };
        let cancellation = CancellationToken::new();
        let mut prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        prepared
            .pane
            .challenge(prepared.deadline, &cancellation)
            .unwrap();
        let (_control, mut capture) = prepared.commit(&cancellation).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !directory.0.join("done").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(PAUSE);
        }
        let mut retained = 0;
        let mut buffer = [0; CAPTURE_CHUNK];
        loop {
            match capture.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => retained += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(PAUSE);
                }
                Err(error) => panic!("capture failure: {error}"),
            }
            assert!(Instant::now() < deadline);
        }
        assert!(retained > 0 && retained < 8_388_608);
        drop(capture);
        prepared.server.retire().unwrap();
    }

    fn read_until(backend: &mut NativeTerminalTmuxBackend, marker: &[u8]) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = Vec::new();
        let mut bytes = [0; 4096];
        while !output.windows(marker.len()).any(|bytes| bytes == marker) {
            let read = backend.read(&mut bytes).unwrap();
            output.extend_from_slice(&bytes[..read.bytes_read]);
            assert!(output.len() <= 64 * 1024);
            assert!(Instant::now() < deadline, "missing marker in {output:?}");
            std::thread::sleep(PAUSE);
        }
        output
    }
    #[test]
    fn real_owned_backend_resizes_writes_reports_actual_job_exit_and_reaps_server() {
        let directory = Directory::new();
        let Some(request) = request(
            &directory,
            "stty -echo; printf READY; IFS= read -r line; stty size; printf 'GOT:%s\\n' \"$line\"; exit 7",
        ) else {
            return;
        };
        let cancellation = CancellationToken::new();
        let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        let mut backend = prepared.commit_owned(&cancellation).unwrap();
        assert_eq!(backend.input_write_limit(), 64 * 1024);
        read_until(&mut backend, b"READY");
        backend
            .resize(&TerminalDimensions::new(40, 120).unwrap())
            .unwrap();
        let input = b"hello\n";
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let receipt = backend.write_with_paste(input, false).unwrap();
            if receipt.bytes_written() == input.len() {
                break;
            }
            assert_eq!(receipt.bytes_written(), 0);
            assert!(Instant::now() < deadline);
            std::thread::sleep(PAUSE);
        }
        let output = read_until(&mut backend, b"GOT:hello");
        assert!(String::from_utf8_lossy(&output).contains("40 120"));
        while backend.status().unwrap() == TerminalPtyStatus::Running {
            assert!(Instant::now() < deadline);
            std::thread::sleep(PAUSE);
        }
        assert_eq!(backend.status().unwrap(), TerminalPtyStatus::Exited(7));
        let receipt = backend.close(false, &mut |_| {}).unwrap();
        assert_eq!(receipt.status, TerminalPtyStatus::Exited(7));
        assert!(!receipt.output_incomplete);
        assert!(backend.server.child.is_none());
        assert!(backend.server.artifacts.sockets.is_empty());
    }

    #[test]
    fn real_owned_backend_preserves_fast_signal_and_cleans_reparented_jobs() {
        for (command, expected) in [
            ("kill -KILL $$", TerminalPtyStatus::Signalled(9)),
            (
                "/bin/sh -c 'trap \"\" HUP TERM; while :; do /bin/sleep 1; done' & printf 'BACKGROUND:%s\\n' \"$!\"; exit 0",
                TerminalPtyStatus::Exited(0),
            ),
        ] {
            let directory = Directory::new();
            let Some(request) = request(&directory, command) else {
                return;
            };
            let cancellation = CancellationToken::new();
            let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
            let mut backend = prepared.commit_owned(&cancellation).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while backend.status().unwrap() == TerminalPtyStatus::Running {
                assert!(Instant::now() < deadline);
                std::thread::sleep(PAUSE);
            }
            assert_eq!(backend.status().unwrap(), expected);
            let mut output = Vec::new();
            let receipt = backend
                .close(false, &mut |bytes| output.extend_from_slice(bytes))
                .unwrap();
            assert_eq!(receipt.status, expected);
            assert!(backend.server.child.is_none());
            if expected == TerminalPtyStatus::Exited(0) {
                assert!(String::from_utf8_lossy(&output).contains("BACKGROUND:"));
            }
        }
    }

    #[test]
    fn real_tmux_cwd_formats_are_literal_and_owned_drop_collects_live_jobs() {
        let directory = Directory::new();
        let cwd = directory.0.join("w#{pid} 'quoted'");
        std::fs::create_dir(&cwd).unwrap();
        let Some(mut request) = request(
            &directory,
            "printf 'CWD:%s\\n' \"$PWD\"; while :; do /bin/sleep 1; done",
        ) else {
            return;
        };
        request.cwd = rustix::fs::open(
            &cwd,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        request.cwd_path = cwd.clone();
        let cancellation = CancellationToken::new();
        let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        let pid = rustix::process::Pid::from_raw(
            i32::try_from(prepared.server.child.as_ref().unwrap().id()).unwrap(),
        )
        .unwrap();
        let mut backend = prepared.commit_owned(&cancellation).unwrap();
        let output = read_until(&mut backend, b"quoted'");
        assert!(String::from_utf8_lossy(&output).contains(cwd.to_str().unwrap()));
        drop(backend);
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn real_failed_close_keeps_exact_artifact_cleanup_for_retry() {
        let directory = Directory::new();
        let Some(request) = request(&directory, "printf READY; while :; do /bin/sleep 1; done")
        else {
            return;
        };
        let cancellation = CancellationToken::new();
        let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        let original = prepared.server.artifacts.sockets[0].0.clone();
        let parked = directory.0.join("parked");
        let mut backend = prepared.commit_owned(&cancellation).unwrap();
        read_until(&mut backend, b"READY");
        std::fs::rename(&original, &parked).unwrap();
        let replacement = crate::terminal_helper::bind_startup_listener(
            &helper(),
            false,
            &directory.fd(),
            &directory.0,
            original.file_name().unwrap().to_str().unwrap(),
            Instant::now() + Duration::from_secs(10),
            &cancellation,
        )
        .unwrap();
        assert!(backend.close(true, &mut |_| {}).is_err());
        assert!(backend.server.child.is_none());
        assert_eq!(backend.server.artifacts.sockets.len(), 1);
        assert!(original.exists() && parked.exists());
        drop(replacement);
        std::fs::remove_file(&original).unwrap();
        std::fs::rename(&parked, &original).unwrap();
        backend.close(true, &mut |_| {}).unwrap();
        assert!(backend.server.artifacts.sockets.is_empty());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn real_capture_overload_remains_an_explicit_gap_after_fast_job_exit() {
        let directory = Directory::new();
        let Some(request) = request(&directory, "/usr/bin/head -c 8388608 /dev/zero; exit 0")
        else {
            return;
        };
        let cancellation = CancellationToken::new();
        let prepared = PreparedTerminalTmuxLaunch::prepare(request, &cancellation).unwrap();
        let mut backend = prepared.commit_owned(&cancellation).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while backend.status().unwrap() == TerminalPtyStatus::Running {
            assert!(Instant::now() < deadline);
            std::thread::sleep(PAUSE);
        }
        let mut received = 0;
        let receipt = backend
            .close(false, &mut |bytes| received += bytes.len())
            .unwrap();
        assert_eq!(receipt.status, TerminalPtyStatus::Exited(0));
        assert!(receipt.output_incomplete);
        assert!(received > 0 && received < 8 * 1024 * 1024);
        assert!(backend.server.child.is_none());
    }

    fn startup_event<B: TerminalSessionBackend>(
        backend: &mut B,
        control: &mut crate::terminal_startup::TerminalStartupControl,
        expected: crate::terminal_startup::TerminalStartupEvent,
        output: &mut Vec<u8>,
        deadline: Instant,
        scenario: &str,
    ) {
        loop {
            let mut bytes = [0; 4096];
            let read = backend.read(&mut bytes).unwrap_or_else(|()| {
                panic!(
                    "{scenario}: read failed awaiting {expected:?}; deadline_remaining={:?}; output_bytes={}; tail={:?}",
                    deadline.checked_duration_since(Instant::now()),
                    output.len(),
                    String::from_utf8_lossy(&output[output.len().saturating_sub(2048)..])
                )
            });
            output.extend_from_slice(&bytes[..read.bytes_read]);
            assert!(output.len() < 64 * 1024);
            if let Some(event) = control
                .poll(Instant::now(), &CancellationToken::new())
                .unwrap()
            {
                assert_eq!(event, expected);
                return;
            }
            assert!(Instant::now() < deadline, "startup output: {output:?}");
            std::thread::sleep(PAUSE);
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "One real matrix exercises the shared startup contract across both shells and profiles."
    )]
    fn real_tmux_shared_bootstrap_preserves_profiles_and_durable_command_gate() {
        use crate::terminal_shell::TerminalShell;
        use crate::terminal_startup::{PreparedTerminalBootstrap, TerminalStartupEvent};
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(shell).exists() {
                continue;
            }
            for clean in [false, true] {
                for commandless in [false, true] {
                    let scenario = format!("shell={shell} clean={clean} commandless={commandless}");
                    let directory = Directory::new();
                    let artifact_root = Directory::new();
                    let mut artifact_path = artifact_root.0.clone();
                    while artifact_path.as_os_str().as_bytes().len() < 850 {
                        artifact_path.push(format!("é{}", "'".repeat(60)));
                        std::fs::create_dir(&artifact_path).unwrap();
                        std::fs::set_permissions(
                            &artifact_path,
                            std::fs::Permissions::from_mode(0o700),
                        )
                        .unwrap();
                    }
                    let artifacts = Directory(artifact_path);
                    let Some(mut request) = request(&directory, "unused") else {
                        return;
                    };
                    let profile = if shell.ends_with("bash") {
                        ".bash_profile"
                    } else {
                        ".zprofile"
                    };
                    std::fs::write(directory.0.join(profile), "set -a; export FROM_PROFILE=user; printf PROFILE_OUTPUT; for fd in 3 4 5 6 7 8 9; do eval \"exec $fd>&-\"; done\n").unwrap();
                    let source = format!(
                        "test \"${{FROM_PROFILE-unset}}\" = {} || exit 7; if /usr/bin/env | /usr/bin/grep '^_mg_bootstrap_' >/dev/null; then exit 8; fi; printf command > executed; exec /bin/sh -c 'exit 23'",
                        if clean { "unset" } else { "user" }
                    );
                    let marker = marker_helper();
                    let cancellation = CancellationToken::new();
                    let deadline = Instant::now() + Duration::from_secs(10);
                    let bootstrap = PreparedTerminalBootstrap::new(
                        &TerminalShell::from_executable(Path::new(shell), clean).unwrap(),
                        (!commandless).then_some(source.as_str()),
                        artifacts.fd(),
                        artifacts.0.clone(),
                        &marker,
                        deadline,
                        &cancellation,
                    )
                    .unwrap();
                    request.program = bootstrap.program().into();
                    request.arguments = bootstrap.arguments().to_vec();
                    request.initial_source = bootstrap.startup_source().map(str::to_owned);
                    request.environment.extend([
                        ("HOME".into(), directory.0.clone().into_os_string()),
                        ("ZDOTDIR".into(), directory.0.clone().into_os_string()),
                    ]);
                    request.artifacts = artifacts.fd();
                    request.artifact_path = artifacts.0.clone();
                    let prepared =
                        PreparedTerminalTmuxLaunch::prepare_until(request, deadline, &cancellation)
                            .unwrap();
                    let published = bootstrap.publish(&cancellation).unwrap();
                    let (mut backend, mut control) =
                        published.attach(prepared.commit_owned(&cancellation).unwrap());
                    let mut output = Vec::new();
                    assert_eq!(backend.write(b"bad input\n").unwrap().bytes_written(), 0);
                    startup_event(
                        &mut backend,
                        &mut control,
                        TerminalStartupEvent::ShellReady,
                        &mut output,
                        deadline,
                        &scenario,
                    );
                    assert!(!directory.0.join("executed").exists());
                    assert!(!String::from_utf8_lossy(&output).contains("_machine_god_ack"));
                    backend.restore_startup_echo().unwrap();
                    assert!(
                        control
                            .acknowledge_shell_ready(Instant::now(), &cancellation)
                            .unwrap()
                    );
                    if commandless {
                        assert!(control.is_complete());
                        let input = format!("{source}\n");
                        loop {
                            let receipt = backend.write(input.as_bytes()).unwrap();
                            if receipt.bytes_written() == input.len() {
                                break;
                            }
                            assert_eq!(receipt.bytes_written(), 0);
                            assert!(Instant::now() < deadline);
                            std::thread::sleep(PAUSE);
                        }
                    } else {
                        startup_event(
                            &mut backend,
                            &mut control,
                            TerminalStartupEvent::CommandStarted,
                            &mut output,
                            deadline,
                            &scenario,
                        );
                        assert!(!directory.0.join("executed").exists());
                        assert!(
                            control
                                .release_command(Instant::now(), &cancellation)
                                .unwrap()
                        );
                    }
                    while backend.status().unwrap() == TerminalPtyStatus::Running {
                        let mut bytes = [0; 4096];
                        let read = backend.read(&mut bytes).unwrap();
                        output.extend_from_slice(&bytes[..read.bytes_read]);
                        assert!(output.len() < 64 * 1024);
                        assert!(Instant::now() < deadline, "output: {output:?}");
                        std::thread::sleep(PAUSE);
                    }
                    assert_eq!(backend.status().unwrap(), TerminalPtyStatus::Exited(23));
                    backend
                        .close(false, &mut |bytes| output.extend_from_slice(bytes))
                        .unwrap();
                    control.retry_cleanup().unwrap();
                    assert_eq!(
                        std::fs::read_to_string(directory.0.join("executed")).unwrap(),
                        "command"
                    );
                    assert_eq!(
                        String::from_utf8_lossy(&output).contains("PROFILE_OUTPUT"),
                        !clean
                    );
                    assert_eq!(std::fs::read_dir(&artifacts.0).unwrap().count(), 0);
                }
            }
        }
    }
}
