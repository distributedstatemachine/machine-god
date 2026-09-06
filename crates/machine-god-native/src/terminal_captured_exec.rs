//! Captured bash/zsh execution on one collected native worker. Process ownership,
//! launch framing and head/tail retention reuse the terminal's existing primitives.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::NativeOwnedWorkerSpawner;
use crate::background_process::{
    BackgroundProcessExit, OwnedBackgroundProcess, TerminalChildGuard,
    ValidatedBackgroundEnvironment,
};
use crate::terminal::PipeCapture;
use crate::terminal_helper::{
    COMMIT, DescriptorIo, LaunchFrame, READY, TerminalHelperErrorKind, TerminalPtyDimensions,
    TerminalPtyHelper, decode_helper_deadline, encode_helper_deadline, read_frame, read_gate,
    validate_pty_directory, write_gate,
};
use crate::terminal_shell::TerminalShell;
use machine_god_core::{
    BoxFuture, CancellationToken, MAX_TERMINAL_ACTION_OUTPUT_BYTES, MAX_TERMINAL_EXEC_DURATION,
    TerminalExecCapturedOutput, TerminalExecRequest, TerminalExecResult, TerminalExecStatus,
    TerminalProfile,
};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::OFlags;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::mem::MaybeUninit;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

#[doc(hidden)]
pub const TERMINAL_CAPTURED_HELPER_ARGUMENT: &str = "--machine-god-terminal-captured-helper";
const EXEC_DESCRIPTOR_TOKEN: u8 = 0xc1;
const EXEC_FAILED: u8 = 0xe1;
const CAPTURED_DEADLINE_ENV: &str = "MACHINE_GOD_CAPTURED_DEADLINE";

/// Redacted captured-execution failure; command text and environment are never included.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalCapturedExecError {
    /// The request or explicit launch authority failed validation.
    Invalid,
    /// All configured execution permits are occupied.
    Capacity,
    /// Execution was cancelled; native cleanup retains any unresolved ownership.
    Cancelled,
    /// A native launch, capture or cleanup operation failed.
    Process,
    /// The owned worker could not complete normally.
    Worker,
}

impl std::fmt::Display for TerminalCapturedExecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "invalid captured terminal request",
            Self::Capacity => "captured terminal execution capacity exhausted",
            Self::Cancelled => "captured terminal execution cancelled",
            Self::Process => "captured terminal process operation failed",
            Self::Worker => "captured terminal worker failed",
        })
    }
}
impl std::error::Error for TerminalCapturedExecError {}

/// Owns only explicit immutable launch configuration and bounded admission.
/// Construction and unpolled calls perform no IO, account lookup or spawning.
pub struct TerminalCapturedExec {
    helper: Arc<TerminalPtyHelper>,
    timeout: Duration,
    maximum_active: usize,
    active: Arc<AtomicUsize>,
}

/// Produced and consumed on the same collected effect worker. In particular,
/// the retained directory never travels back through an asynchronous reply.
pub(crate) struct TerminalCapturedAuthority {
    pub(crate) request: TerminalExecRequest,
    pub(crate) shell: TerminalShell,
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) cwd: OwnedFd,
}

impl std::fmt::Debug for TerminalCapturedExec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalCapturedExec")
            .finish_non_exhaustive()
    }
}

impl TerminalCapturedExec {
    /// Creates an inert executor for an explicit absolute private-helper program.
    /// Timeout is 1ms–600s and simultaneous executions are bounded to 1–16.
    ///
    /// # Errors
    /// Returns [`TerminalCapturedExecError::Invalid`] for invalid launch configuration.
    pub fn new(
        helper_program: PathBuf,
        helper_arguments: Vec<OsString>,
        timeout: Duration,
        maximum_active: usize,
    ) -> Result<Self, TerminalCapturedExecError> {
        if timeout < Duration::from_millis(1)
            || timeout > MAX_TERMINAL_EXEC_DURATION
            || !(1..=16).contains(&maximum_active)
        {
            return Err(TerminalCapturedExecError::Invalid);
        }
        Ok(Self {
            helper: Arc::new(
                TerminalPtyHelper::new(helper_program, helper_arguments)
                    .map_err(|_| TerminalCapturedExecError::Invalid)?,
            ),
            timeout,
            maximum_active,
            active: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// The caller resolves and authorizes the exact shell, environment and cwd
    /// descriptor before calling. The request's cwd is display/binding data,
    /// never opened here. Native identity validation occurs only on the worker.
    ///
    /// # Errors
    /// Returns a redacted validation, capacity, cancellation, native or worker error.
    #[must_use]
    pub fn execute(
        &self,
        request: TerminalExecRequest,
        shell: TerminalShell,
        environment: Vec<(OsString, OsString)>,
        cwd: OwnedFd,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalExecResult, TerminalCapturedExecError>> {
        self.execute_prepared(
            move |_, _, _| {
                Ok(TerminalCapturedAuthority {
                    request,
                    shell,
                    environment,
                    cwd,
                })
            },
            cancellation,
            CancellationToken::new(),
        )
    }

    /// Prepare authority and execute on one collected worker under one deadline
    /// beginning at first poll. The host stop is inspected by native work even
    /// when the caller leaves its response future unpolled after submission.
    pub(crate) fn execute_prepared(
        &self,
        prepare: impl FnOnce(
            Instant,
            &CancellationToken,
            &CancellationToken,
        ) -> Result<TerminalCapturedAuthority, TerminalCapturedExecError>
        + Send
        + 'static,
        cancellation: CancellationToken,
        host_stop: CancellationToken,
    ) -> BoxFuture<'static, Result<TerminalExecResult, TerminalCapturedExecError>> {
        let helper = Arc::clone(&self.helper);
        let active = Arc::clone(&self.active);
        let maximum = self.maximum_active;
        let timeout = self.timeout;
        Box::pin(async move {
            let deadline = Instant::now() + timeout;
            if stopped(&cancellation, &[&host_stop]) {
                return Err(TerminalCapturedExecError::Cancelled);
            }
            active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < maximum).then_some(count + 1)
                })
                .map_err(|_| TerminalCapturedExecError::Capacity)?;
            let permit = Permit(active);
            let stop = CancellationToken::new();
            let _cancel_on_drop = CancelOnDrop(stop.clone());
            let result_cancellation = cancellation.clone();
            let result_host_stop = host_stop.clone();
            let receipt = NativeOwnedWorkerSpawner::new()
                .run(move || {
                    let result = (|| {
                        if stopped(&cancellation, &[&stop, &host_stop]) {
                            return Err(TerminalCapturedExecError::Cancelled);
                        }
                        if Instant::now() >= deadline {
                            return empty_timeout(Instant::now()).into_terminal_result();
                        }
                        let TerminalCapturedAuthority {
                            request,
                            shell,
                            environment,
                            cwd,
                        } = prepare(deadline, &cancellation, &host_stop)?;
                        request
                            .validate()
                            .map_err(|_| TerminalCapturedExecError::Invalid)?;
                        let environment = ValidatedBackgroundEnvironment::new(environment)
                            .map_err(|_| TerminalCapturedExecError::Invalid)?;
                        if request.profile.unwrap_or(TerminalProfile::User) != shell.profile() {
                            return Err(TerminalCapturedExecError::Invalid);
                        }
                        run(
                            &helper,
                            &request,
                            &shell,
                            &environment,
                            cwd,
                            deadline,
                            MAX_TERMINAL_ACTION_OUTPUT_BYTES,
                            &cancellation,
                            &[&stop, &host_stop],
                        )
                        .and_then(CapturedOutcome::into_terminal_result)
                    })();
                    (result, permit)
                })
                .await
                .map_err(|_| TerminalCapturedExecError::Worker)?;
            if stopped(&result_cancellation, &[&result_host_stop]) {
                return Err(TerminalCapturedExecError::Cancelled);
            }
            receipt.0
        })
    }

    /// Already-collected worker only. The caller supplies the original deadline;
    /// admission and queued work never restart it. Native cleanup is shared.
    #[cfg(any(test, feature = "ai-gateway-http"))]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_probe_on_worker(
        &self,
        request: &TerminalExecRequest,
        shell: &TerminalShell,
        environment: &ValidatedBackgroundEnvironment,
        cwd: OwnedFd,
        deadline: Instant,
        output_limit: usize,
        cancellation: &CancellationToken,
        stop: &[&CancellationToken],
    ) -> Result<TerminalCapturedProbeOutcome, TerminalCapturedExecError> {
        if stopped(cancellation, stop) {
            return Err(TerminalCapturedExecError::Cancelled);
        }
        request
            .validate()
            .map_err(|_| TerminalCapturedExecError::Invalid)?;
        if output_limit == 0
            || output_limit > 16 * 1024
            || request.profile.unwrap_or(TerminalProfile::User) != shell.profile()
        {
            return Err(TerminalCapturedExecError::Invalid);
        }
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < self.maximum_active).then_some(count + 1)
            })
            .map_err(|_| TerminalCapturedExecError::Capacity)?;
        let _permit = Permit(Arc::clone(&self.active));
        let outcome = run(
            &self.helper,
            request,
            shell,
            environment,
            cwd,
            deadline,
            output_limit,
            cancellation,
            stop,
        )?;
        Ok(TerminalCapturedProbeOutcome {
            output_bytes: outcome
                .stdout
                .total_bytes
                .saturating_add(outcome.stderr.total_bytes),
            truncated: outcome.stdout.total_bytes > outcome.stdout.bytes.len() as u64
                || outcome.stderr.total_bytes > outcome.stderr.bytes.len() as u64,
            status: outcome.status,
        })
    }
}

#[cfg(any(test, feature = "ai-gateway-http"))]
pub(crate) struct TerminalCapturedProbeOutcome {
    pub(crate) status: TerminalExecStatus,
    pub(crate) output_bytes: u64,
    pub(crate) truncated: bool,
}

/// Internal capture may have a stricter limit than the public exec contract.
struct CapturedOutcome {
    status: TerminalExecStatus,
    stdout: TerminalExecCapturedOutput,
    stderr: TerminalExecCapturedOutput,
    duration: Duration,
}
impl CapturedOutcome {
    fn into_terminal_result(self) -> Result<TerminalExecResult, TerminalCapturedExecError> {
        let result = TerminalExecResult {
            status: self.status,
            stdout: self.stdout,
            stderr: self.stderr,
            duration: self.duration,
        };
        result
            .validate()
            .map_err(|_| TerminalCapturedExecError::Process)?;
        Ok(result)
    }
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn stopped(cancellation: &CancellationToken, stop: &[&CancellationToken]) -> bool {
    cancellation.is_cancelled() || stop.iter().any(|token| token.is_cancelled())
}

/// Adds cancellation-on-abandoned-future to the shared bounded gate codec.
struct Gate<'a> {
    stream: &'a mut UnixStream,
    stop: &'a [&'a CancellationToken],
}
impl Read for Gate<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.stop.iter().any(|token| token.is_cancelled()) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.stream.read(bytes)
    }
}
impl Write for Gate<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.stop.iter().any(|token| token.is_cancelled()) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.stream.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Keep the owned launch/capture/cleanup transition linear and all authority explicit.
fn run(
    helper: &TerminalPtyHelper,
    request: &TerminalExecRequest,
    shell: &TerminalShell,
    environment: &ValidatedBackgroundEnvironment,
    cwd: OwnedFd,
    deadline: Instant,
    output_limit: usize,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<CapturedOutcome, TerminalCapturedExecError> {
    let started = Instant::now();
    if stopped(cancellation, stop) {
        return Err(TerminalCapturedExecError::Cancelled);
    }
    let arguments = shell
        .captured_arguments(&request.command)
        .map_err(|_| TerminalCapturedExecError::Invalid)?;
    validate_pty_directory(&cwd).map_err(|_| TerminalCapturedExecError::Invalid)?;
    let frame = LaunchFrame::encode(
        shell
            .program()
            .to_str()
            .ok_or(TerminalCapturedExecError::Invalid)?,
        &arguments,
        environment,
        TerminalPtyDimensions {
            rows: 1,
            columns: 1,
        },
    )
    .map_err(|_| TerminalCapturedExecError::Invalid)?;
    let (mut stderr, child_gate) =
        UnixStream::pair().map_err(|_| TerminalCapturedExecError::Process)?;
    stderr
        .set_nonblocking(true)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let (stdout, child_stdout) = std::io::pipe().map_err(|_| TerminalCapturedExecError::Process)?;
    let (exec_error, child_exec_error) =
        std::io::pipe().map_err(|_| TerminalCapturedExecError::Process)?;
    let flags =
        rustix::fs::fcntl_getfl(&exec_error).map_err(|_| TerminalCapturedExecError::Process)?;
    rustix::fs::fcntl_setfl(&exec_error, flags | OFlags::NONBLOCK)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let flags = rustix::fs::fcntl_getfl(&stdout).map_err(|_| TerminalCapturedExecError::Process)?;
    rustix::fs::fcntl_setfl(&stdout, flags | OFlags::NONBLOCK)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let mut guard = TerminalChildGuard::reserve(cancellation)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let helper_deadline = match encode_helper_deadline(deadline, MAX_TERMINAL_EXEC_DURATION) {
        Ok(stamp) => stamp,
        Err(error) if error.kind == TerminalHelperErrorKind::Timeout => {
            return Ok(empty_timeout(started));
        }
        Err(_) => return Err(TerminalCapturedExecError::Process),
    };
    let mut command = Command::new(helper.program());
    command
        .args(helper.arguments())
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env(CAPTURED_DEADLINE_ENV, helper_deadline)
        .stdin(Stdio::from(OwnedFd::from(child_gate)))
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::from(cwd));
    if stopped(cancellation, stop) {
        return Err(TerminalCapturedExecError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Ok(empty_timeout(started));
    }
    guard
        .spawn(&mut command)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    drop(command);
    if let Err(error) =
        send_exec_descriptor(&stderr, &child_exec_error, deadline, cancellation, stop)
    {
        return if stopped(cancellation, stop) {
            Err(TerminalCapturedExecError::Cancelled)
        } else if Instant::now() >= deadline {
            Ok(empty_timeout(started))
        } else {
            Err(error)
        };
    }
    drop(child_exec_error);
    let mut gate = Gate {
        stream: &mut stderr,
        stop,
    };
    let handshake = (|| {
        write_gate(&mut gate, &frame, deadline, cancellation).map_err(|_| ())?;
        let mut ready = [0];
        read_gate(&mut gate, &mut ready, deadline, cancellation).map_err(|_| ())?;
        if ready != [READY] {
            return Err(());
        }
        Ok::<_, ()>(())
    })();
    if handshake.is_err() {
        return if stopped(cancellation, stop) {
            Err(TerminalCapturedExecError::Cancelled)
        } else if Instant::now() >= deadline {
            Ok(empty_timeout(started))
        } else {
            Err(TerminalCapturedExecError::Process)
        };
    }
    let mut process = ProcessGuard {
        process: guard
            .into_session()
            .map_err(|_| TerminalCapturedExecError::Process)?,
        closed: false,
    };
    process
        .process
        .activate_signal_controller()
        .map_err(|_| TerminalCapturedExecError::Process)?;
    if stopped(cancellation, stop) {
        return Err(TerminalCapturedExecError::Cancelled);
    }
    if write_gate(&mut gate, &[COMMIT], deadline, cancellation).is_err() {
        return if stopped(cancellation, stop) {
            Err(TerminalCapturedExecError::Cancelled)
        } else if Instant::now() >= deadline {
            Ok(empty_timeout(started))
        } else {
            Err(TerminalCapturedExecError::Process)
        };
    }
    loop {
        if stopped(cancellation, stop) {
            return Err(TerminalCapturedExecError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Ok(empty_timeout(started));
        }
        let mut byte = [0];
        match rustix::io::read(&exec_error, &mut byte) {
            Ok(0) => break,
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => {}
            Ok(_) | Err(_) => return Err(TerminalCapturedExecError::Process),
        }
        if Instant::now() >= deadline {
            return Ok(empty_timeout(started));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut stdout_capture = PipeCapture::default();
    let mut stderr_capture = PipeCapture::default();
    let mut total = 0u64;
    let mut status = loop {
        if stopped(cancellation, stop) {
            return Err(TerminalCapturedExecError::Cancelled);
        }
        // Alternating fixed read attempts prevent either producer starving the
        // other stream or cancellation/deadline observation.
        drain_once(&stdout, &mut stdout_capture, &mut total, output_limit)?;
        drain_once(&stderr, &mut stderr_capture, &mut total, output_limit)?;
        if total > output_limit as u64 {
            break TerminalExecStatus::OutputLimit {};
        }
        if Instant::now() >= deadline {
            break TerminalExecStatus::TimedOut {};
        }
        if let Some(status) = process
            .process
            .terminal_poll()
            .map_err(|_| TerminalCapturedExecError::Process)?
        {
            break convert_status(status);
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    // The existing owner captures/pins its process tree, closes authority before
    // reap and quarantines unresolved cleanup on Drop. No PID is reconstructed.
    process.close(matches!(
        status,
        TerminalExecStatus::Exited { .. } | TerminalExecStatus::Signaled { .. }
    ))?;
    if !matches!(status, TerminalExecStatus::OutputLimit {}) {
        for _ in 0..64 {
            let timed_out = matches!(status, TerminalExecStatus::TimedOut {});
            let stdout_read = drain_bounded(
                &stdout,
                &mut stdout_capture,
                &mut total,
                timed_out,
                output_limit,
            )?;
            let stderr_read = drain_bounded(
                &stderr,
                &mut stderr_capture,
                &mut total,
                timed_out,
                output_limit,
            )?;
            if total > output_limit as u64 {
                status = TerminalExecStatus::OutputLimit {};
                break;
            }
            if !stdout_read && !stderr_read {
                break;
            }
        }
    }
    if stopped(cancellation, stop) {
        return Err(TerminalCapturedExecError::Cancelled);
    }
    let stdout = stdout_capture
        .finish()
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let stderr = stderr_capture
        .finish()
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let result = CapturedOutcome {
        status,
        stdout: TerminalExecCapturedOutput {
            bytes: stdout.bytes().to_vec(),
            total_bytes: stdout.total_bytes(),
        },
        stderr: TerminalExecCapturedOutput {
            bytes: stderr.bytes().to_vec(),
            total_bytes: stderr.total_bytes(),
        },
        duration: started.elapsed().min(MAX_TERMINAL_EXEC_DURATION),
    };
    Ok(result)
}

struct ProcessGuard {
    process: OwnedBackgroundProcess,
    closed: bool,
}
impl ProcessGuard {
    fn close(&mut self, force: bool) -> Result<(), TerminalCapturedExecError> {
        self.process
            .terminal_close(force, |_| {})
            .map_err(|_| TerminalCapturedExecError::Process)?;
        self.closed = true;
        Ok(())
    }
}
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.close(false);
        }
    }
}

fn drain_once(
    fd: &impl AsFd,
    capture: &mut PipeCapture,
    total: &mut u64,
    output_limit: usize,
) -> Result<bool, TerminalCapturedExecError> {
    drain_bounded(fd, capture, total, false, output_limit)
}
fn drain_bounded(
    fd: &impl AsFd,
    capture: &mut PipeCapture,
    total: &mut u64,
    timed_out: bool,
    output_limit: usize,
) -> Result<bool, TerminalCapturedExecError> {
    let mut buffer = [0u8; 16 * 1024];
    let limit = if timed_out || output_limit < MAX_TERMINAL_ACTION_OUTPUT_BYTES {
        let observation_limit = output_limit as u64 + u64::from(!timed_out);
        usize::try_from(observation_limit.saturating_sub(*total))
            .unwrap_or(0)
            .min(buffer.len())
    } else {
        buffer.len()
    };
    if limit == 0 {
        return Ok(false);
    }
    match rustix::io::read(fd, &mut buffer[..limit]) {
        Ok(0) | Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => Ok(false),
        Ok(count) => {
            capture.push(&buffer[..count]);
            *total = total.saturating_add(count as u64);
            Ok(true)
        }
        Err(_) => Err(TerminalCapturedExecError::Process),
    }
}
fn convert_status(status: BackgroundProcessExit) -> TerminalExecStatus {
    match status {
        BackgroundProcessExit::Exited(exit_code) => TerminalExecStatus::Exited { exit_code },
        BackgroundProcessExit::Signaled(signal) => TerminalExecStatus::Signaled { signal },
    }
}
fn empty_timeout(started: Instant) -> CapturedOutcome {
    CapturedOutcome {
        status: TerminalExecStatus::TimedOut {},
        stdout: TerminalExecCapturedOutput {
            bytes: Vec::new(),
            total_bytes: 0,
        },
        stderr: TerminalExecCapturedOutput {
            bytes: Vec::new(),
            total_bytes: 0,
        },
        duration: started.elapsed().min(MAX_TERMINAL_EXEC_DURATION),
    }
}

/// Private single-threaded CLI entrypoint, selected before ordinary config.
/// The shared frame is bounded; no requested shell starts before COMMIT.
///
/// # Errors
/// Returns a redacted error for malformed launch framing or native launch failure.
#[doc(hidden)]
pub fn run_terminal_captured_helper() -> Result<(), TerminalCapturedExecError> {
    let stamp =
        std::env::var(CAPTURED_DEADLINE_ENV).map_err(|_| TerminalCapturedExecError::Invalid)?;
    let deadline = decode_helper_deadline(&stamp, MAX_TERMINAL_EXEC_DURATION)
        .map_err(|_| TerminalCapturedExecError::Invalid)?;
    let cancellation = CancellationToken::new();
    let input = std::io::stdin();
    let cwd = std::io::stderr();
    validate_pty_directory(&cwd).map_err(|_| TerminalCapturedExecError::Invalid)?;
    let flags =
        rustix::fs::fcntl_getfl(input.as_fd()).map_err(|_| TerminalCapturedExecError::Process)?;
    rustix::fs::fcntl_setfl(input.as_fd(), flags | OFlags::NONBLOCK)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let _exec_error = ExecErrorGuard(receive_exec_descriptor(&input, deadline)?);
    let mut gate = DescriptorIo(input.as_fd());
    let frame = read_frame(&mut gate, deadline, &cancellation)
        .map_err(|_| TerminalCapturedExecError::Invalid)?;
    rustix::process::fchdir(cwd.as_fd()).map_err(|_| TerminalCapturedExecError::Process)?;
    rustix::process::setsid().map_err(|_| TerminalCapturedExecError::Process)?;
    write_gate(&mut gate, &[READY], deadline, &cancellation)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let mut commit = [0];
    read_gate(&mut gate, &mut commit, deadline, &cancellation)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    if commit != [COMMIT] {
        return Err(TerminalCapturedExecError::Invalid);
    }
    let stderr = rustix::io::fcntl_dupfd_cloexec(input.as_fd(), 3)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    let flags = rustix::fs::fcntl_getfl(&stderr).map_err(|_| TerminalCapturedExecError::Process)?;
    rustix::fs::fcntl_setfl(&stderr, flags - OFlags::NONBLOCK)
        .map_err(|_| TerminalCapturedExecError::Process)?;
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
        .stdin(Stdio::null())
        .stderr(Stdio::from(stderr));
    crate::terminal_helper::check_deadline(deadline, &cancellation)
        .map_err(|_| TerminalCapturedExecError::Process)?;
    Err({
        let _ = shell.exec();
        TerminalCapturedExecError::Process
    })
}

/// An exec-success receipt cannot share stderr bytes: successful exec closes
/// this explicit CLOEXEC descriptor; every returning helper error writes failure.
struct ExecErrorGuard(OwnedFd);
impl Drop for ExecErrorGuard {
    fn drop(&mut self) {
        while matches!(
            rustix::io::write(&self.0, &[EXEC_FAILED]),
            Err(rustix::io::Errno::INTR)
        ) {}
    }
}

fn send_exec_descriptor(
    gate: &UnixStream,
    descriptor: &impl AsFd,
    deadline: Instant,
    cancellation: &CancellationToken,
    stop: &[&CancellationToken],
) -> Result<(), TerminalCapturedExecError> {
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    loop {
        if stopped(cancellation, stop) {
            return Err(TerminalCapturedExecError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(TerminalCapturedExecError::Process);
        }
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let descriptors = [descriptor.as_fd()];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        if !ancillary.push(SendAncillaryMessage::ScmRights(&descriptors)) {
            return Err(TerminalCapturedExecError::Process);
        }
        match sendmsg(
            gate,
            &[io::IoSlice::new(&[EXEC_DESCRIPTOR_TOKEN])],
            &mut ancillary,
            SendFlags::empty(),
        ) {
            Ok(1) => return Ok(()),
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => {
                std::thread::sleep(Duration::from_millis(2));
            }
            _ => return Err(TerminalCapturedExecError::Process),
        }
    }
}

fn receive_exec_descriptor(
    gate: &impl AsFd,
    deadline: Instant,
) -> Result<OwnedFd, TerminalCapturedExecError> {
    use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg};
    #[cfg(target_os = "linux")]
    let (receive_flags, allowed_flags) = (RecvFlags::CMSG_CLOEXEC, ReturnFlags::CMSG_CLOEXEC);
    #[cfg(target_os = "macos")]
    let (receive_flags, allowed_flags) = (RecvFlags::empty(), ReturnFlags::empty());
    loop {
        if Instant::now() >= deadline {
            return Err(TerminalCapturedExecError::Process);
        }
        let mut token = [0];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let message = match recvmsg(
            gate,
            &mut [io::IoSliceMut::new(&mut token)],
            &mut ancillary,
            receive_flags,
        ) {
            Ok(message) => message,
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(_) => return Err(TerminalCapturedExecError::Process),
        };
        if message.bytes != 1
            || token != [EXEC_DESCRIPTOR_TOKEN]
            || !(message.flags - allowed_flags).is_empty()
        {
            return Err(TerminalCapturedExecError::Invalid);
        }
        let mut received = None;
        for message in ancillary.drain() {
            let RecvAncillaryMessage::ScmRights(descriptors) = message else {
                return Err(TerminalCapturedExecError::Invalid);
            };
            for descriptor in descriptors {
                if received.is_some() {
                    return Err(TerminalCapturedExecError::Invalid);
                }
                rustix::io::fcntl_setfd(&descriptor, rustix::io::FdFlags::CLOEXEC)
                    .map_err(|_| TerminalCapturedExecError::Process)?;
                received = Some(descriptor);
            }
        }
        return received.ok_or(TerminalCapturedExecError::Invalid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_helper::START_TIMEOUT;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU64;

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        root: PathBuf,
        executor: TerminalCapturedExec,
        harness: bool,
    }
    impl Fixture {
        fn delayed(timeout: Duration) -> Self {
            let mut fixture = Self::new(timeout);
            fixture.executor.helper = Arc::new(
                TerminalPtyHelper::new(
                    std::env::current_exe().unwrap(),
                    vec![
                        "--exact".into(),
                        "terminal_captured_exec::tests::captured_delayed_helper_child".into(),
                        "--ignored".into(),
                        "--nocapture".into(),
                        "--quiet".into(),
                    ],
                )
                .unwrap(),
            );
            fixture.harness = true;
            fixture
        }

        fn new(timeout: Duration) -> Self {
            let root = std::env::temp_dir().join(format!(
                "mg-captured-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            let (helper, harness) = match std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
                Some(binary) => (
                    TerminalPtyHelper::new(
                        binary.into(),
                        vec![TERMINAL_CAPTURED_HELPER_ARGUMENT.into()],
                    )
                    .unwrap(),
                    false,
                ),
                None => (
                    TerminalPtyHelper::new(
                        std::env::current_exe().unwrap(),
                        vec![
                            "--exact".into(),
                            "terminal_captured_exec::tests::captured_helper_child".into(),
                            "--ignored".into(),
                            "--nocapture".into(),
                            "--quiet".into(),
                        ],
                    )
                    .unwrap(),
                    true,
                ),
            };
            Self {
                root,
                executor: TerminalCapturedExec::new(
                    helper.program().to_owned(),
                    helper.arguments().to_vec(),
                    timeout,
                    2,
                )
                .unwrap(),
                harness,
            }
        }
        fn future(
            &self,
            command: String,
            program: &str,
            clean: bool,
            extra: Vec<(OsString, OsString)>,
            cancel: CancellationToken,
        ) -> BoxFuture<'static, Result<TerminalExecResult, TerminalCapturedExecError>> {
            let shell = TerminalShell::from_executable(Path::new(program), clean).unwrap();
            let mut environment = vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), self.root.as_os_str().to_owned()),
                ("ZDOTDIR".into(), self.root.as_os_str().to_owned()),
            ];
            environment.extend(extra);
            self.executor.execute(
                TerminalExecRequest {
                    command,
                    cwd: self.root.to_str().unwrap().into(),
                    profile: Some(shell.profile()),
                },
                shell,
                environment,
                rustix::fs::open(
                    &self.root,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .unwrap(),
                cancel,
            )
        }
        fn run(&self, command: &str) -> TerminalExecResult {
            futures_executor::block_on(self.future(
                command.into(),
                "/bin/bash",
                true,
                Vec::new(),
                CancellationToken::new(),
            ))
            .unwrap_or_else(|error| panic!("captured command {command:?} failed: {error}"))
        }
        fn authority(&self, command: &str) -> TerminalCapturedAuthority {
            TerminalCapturedAuthority {
                request: TerminalExecRequest {
                    command: command.into(),
                    cwd: self.root.to_str().unwrap().into(),
                    profile: Some(TerminalProfile::Clean),
                },
                shell: TerminalShell::from_executable(Path::new("/bin/bash"), true).unwrap(),
                environment: vec![("PATH".into(), "/usr/bin:/bin".into())],
                cwd: rustix::fs::open(
                    &self.root,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .unwrap(),
            }
        }
        fn stdout<'a>(&self, result: &'a TerminalExecResult) -> &'a [u8] {
            if self.harness {
                result
                    .stdout
                    .bytes
                    .strip_prefix(b"\nrunning 1 test\n")
                    .expect("exact pinned test helper prelude")
            } else {
                &result.stdout.bytes
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let until = Instant::now() + Duration::from_secs(10);
            while self.executor.active.load(Ordering::Acquire) != 0 && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                self.executor.active.load(Ordering::Acquire),
                0,
                "worker cleanup complete"
            );
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    #[ignore = "private captured helper subprocess"]
    fn captured_helper_child() {
        std::process::exit(if run_terminal_captured_helper().is_ok() {
            0
        } else {
            125
        });
    }

    #[test]
    #[ignore = "private delayed helper subprocess selected explicitly"]
    fn captured_delayed_helper_child() {
        // No subprocess is created during this delay: the parent owns and can
        // terminate this exact helper before it accepts a frame or COMMIT.
        std::thread::sleep(Duration::from_millis(2250));
        captured_helper_child();
    }

    #[test]
    fn captured_exec_configured_deadline_covers_delayed_helper_without_reset() {
        let fixture = Fixture::delayed(Duration::from_secs(5));
        let result = fixture.run("printf delayed; printf effect > effect");
        assert_eq!(result.status, TerminalExecStatus::Exited { exit_code: 0 });
        assert_eq!(fixture.stdout(&result), b"delayed");
        assert!(result.duration >= Duration::from_secs(2));
        assert_eq!(
            std::fs::read(fixture.root.join("effect")).unwrap(),
            b"effect"
        );

        let fixture = Fixture::delayed(Duration::from_millis(100));
        let result = fixture.run("printf forbidden > forbidden");
        assert_eq!(result.status, TerminalExecStatus::TimedOut {});
        assert!(!fixture.root.join("forbidden").exists());
        assert_eq!(fixture.executor.active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn captured_exec_preparation_is_inert_and_host_stop_prevents_submission() {
        let fixture = Fixture::new(Duration::from_secs(5));
        let prepared = Arc::new(AtomicUsize::new(0));
        for cancelled in [false, true] {
            let calls = Arc::clone(&prepared);
            let authority = fixture.authority("printf forbidden > forbidden");
            let stop = CancellationToken::new();
            let future = fixture.executor.execute_prepared(
                move |_, _, _| {
                    calls.fetch_add(1, Ordering::AcqRel);
                    Ok(authority)
                },
                CancellationToken::new(),
                stop.clone(),
            );
            if cancelled {
                stop.cancel();
                assert_eq!(
                    futures_executor::block_on(future),
                    Err(TerminalCapturedExecError::Cancelled)
                );
            } else {
                drop(future);
            }
        }
        assert_eq!(prepared.load(Ordering::Acquire), 0);
        assert_eq!(fixture.executor.active.load(Ordering::Acquire), 0);
        assert!(!fixture.root.join("forbidden").exists());
    }

    #[test]
    fn captured_exec_preparation_consumes_original_deadline() {
        let fixture = Fixture::new(Duration::from_millis(100));
        let authority = fixture.authority("printf forbidden > forbidden");
        let result = futures_executor::block_on(fixture.executor.execute_prepared(
            move |deadline, _, _| {
                std::thread::sleep(Duration::from_millis(150));
                assert!(Instant::now() >= deadline);
                Ok(authority)
            },
            CancellationToken::new(),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(result.status, TerminalExecStatus::TimedOut {});
        assert!(!fixture.root.join("forbidden").exists());
    }

    #[test]
    fn captured_exec_host_stop_cleans_native_process_without_response_poll() {
        let fixture = Fixture::new(Duration::from_secs(30));
        let authority = fixture.authority("printf '%s' \"$$\" > leader; exec /bin/sleep 30");
        let stop = CancellationToken::new();
        let mut future = fixture.executor.execute_prepared(
            move |_, _, _| Ok(authority),
            CancellationToken::new(),
            stop.clone(),
        );
        futures_executor::block_on(async {
            assert!(futures_util::poll!(&mut future).is_pending());
        });
        let until = Instant::now() + Duration::from_secs(5);
        while !fixture.root.join("leader").exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        let pid = std::fs::read_to_string(fixture.root.join("leader"))
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let pid = rustix::process::Pid::from_raw(pid).unwrap();
        stop.cancel();
        let until = Instant::now() + Duration::from_secs(10);
        while rustix::process::test_kill_process(pid).is_ok() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
        // Completed but unconsumed data still occupies its bounded reply slot.
        assert_eq!(fixture.executor.active.load(Ordering::Acquire), 1);
        assert_eq!(
            futures_executor::block_on(future),
            Err(TerminalCapturedExecError::Cancelled)
        );
        assert_eq!(fixture.executor.active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn captured_exec_preserves_binary_streams_environment_and_exact_exit() {
        let fixture = Fixture::new(Duration::from_secs(5));
        let result=futures_executor::block_on(fixture.future("printf '%s' \"$CAPTURED_MARK\"; printf '\\000\\377\\033' ; printf '\\376\\000' >&2; read ignored || :; exit 37".into(),
            "/bin/bash",true,vec![("CAPTURED_MARK".into(),"exact".into())],CancellationToken::new())).unwrap();
        assert_eq!(result.status, TerminalExecStatus::Exited { exit_code: 37 });
        assert_eq!(fixture.stdout(&result), b"exact\0\xff\x1b");
        assert_eq!(result.stderr.bytes, b"\xfe\0");
        assert_eq!(result.stdout.total_bytes, result.stdout.bytes.len() as u64);
        assert_eq!(result.stderr.total_bytes, 2);
    }

    #[test]
    fn captured_exec_distinguishes_signal_from_numeric_exit_and_timeout() {
        let fixture = Fixture::new(Duration::from_secs(5));
        assert_eq!(
            fixture.run("kill -KILL $$").status,
            TerminalExecStatus::Signaled { signal: 9 }
        );
        assert_eq!(
            fixture.run("exit 137").status,
            TerminalExecStatus::Exited { exit_code: 137 }
        );
        let fixture = Fixture::new(Duration::from_millis(100));
        let result = fixture.run("printf before; exec /bin/sleep 30");
        assert_eq!(result.status, TerminalExecStatus::TimedOut {});
        assert_eq!(fixture.stdout(&result), b"before");
        let fixture = Fixture::new(Duration::from_millis(1));
        assert_eq!(
            fixture.run("exec /bin/sleep 30").status,
            TerminalExecStatus::TimedOut {}
        );
    }

    #[test]
    fn captured_exec_distinguishes_exec_failure_from_exit_125() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new(Duration::from_secs(5));
        assert_eq!(
            fixture.run("exit 125").status,
            TerminalExecStatus::Exited { exit_code: 125 }
        );
        let unavailable = fixture.root.join("bash");
        for present in [false, true] {
            if present {
                std::fs::write(&unavailable, b"#!/bin/sh\nexit 0\n").unwrap();
                std::fs::set_permissions(&unavailable, std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            assert_eq!(
                futures_executor::block_on(fixture.future(
                    "exit 0".into(),
                    unavailable.to_str().unwrap(),
                    true,
                    Vec::new(),
                    CancellationToken::new()
                )),
                Err(TerminalCapturedExecError::Process)
            );
        }
    }

    #[test]
    fn captured_exec_descriptor_bootstrap_is_bounded_and_cloexec() {
        use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
        for (token, count) in [
            (EXEC_DESCRIPTOR_TOKEN, 0),
            (0, 1),
            (EXEC_DESCRIPTOR_TOKEN, 2),
            (EXEC_DESCRIPTOR_TOKEN, 1),
        ] {
            let (sender, receiver) = UnixStream::pair().unwrap();
            let (_read, write) = std::io::pipe().unwrap();
            let descriptors = [write.as_fd(), write.as_fd()];
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
            let mut ancillary = SendAncillaryBuffer::new(&mut space);
            if count != 0 {
                assert!(ancillary.push(SendAncillaryMessage::ScmRights(&descriptors[..count])));
            }
            assert_eq!(
                sendmsg(
                    &sender,
                    &[io::IoSlice::new(&[token])],
                    &mut ancillary,
                    SendFlags::empty()
                )
                .unwrap(),
                1
            );
            let result = receive_exec_descriptor(&receiver, Instant::now() + START_TIMEOUT);
            if count == 1 && token == EXEC_DESCRIPTOR_TOKEN {
                let descriptor = result.unwrap();
                assert!(
                    rustix::io::fcntl_getfd(&descriptor)
                        .unwrap()
                        .contains(rustix::io::FdFlags::CLOEXEC)
                );
            } else {
                assert!(matches!(result, Err(TerminalCapturedExecError::Invalid)));
            }
        }
    }

    #[test]
    fn captured_exec_uses_retained_cwd_even_after_path_replacement() {
        let fixture = Fixture::new(Duration::from_secs(5));
        std::fs::write(fixture.root.join("value"), b"owned").unwrap();
        let future = fixture.future(
            "/bin/cat value".into(),
            "/bin/bash",
            true,
            Vec::new(),
            CancellationToken::new(),
        );
        let moved = fixture.root.with_extension("moved");
        std::fs::rename(&fixture.root, &moved).unwrap();
        std::fs::create_dir(&fixture.root).unwrap();
        std::fs::write(fixture.root.join("value"), b"replacement").unwrap();
        let result = futures_executor::block_on(future).unwrap();
        assert_eq!(fixture.stdout(&result), b"owned");
        std::fs::remove_dir_all(moved).unwrap();
    }

    #[test]
    fn captured_exec_accepts_exact_maximum_command_and_rejects_one_over() {
        let fixture = Fixture::new(Duration::from_secs(5));
        let mut command = "printf exact\n#".to_owned();
        command.push_str(
            &"x".repeat(machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES - command.len()),
        );
        assert_eq!(fixture.stdout(&fixture.run(&command)), b"exact");
        command.push('x');
        assert_eq!(
            futures_executor::block_on(fixture.future(
                command,
                "/bin/bash",
                true,
                Vec::new(),
                CancellationToken::new()
            )),
            Err(TerminalCapturedExecError::Invalid)
        );
    }

    #[test]
    fn captured_exec_shell_profiles_drive_actual_startup_arguments() {
        let fixture = Fixture::new(Duration::from_secs(5));
        std::fs::write(
            fixture.root.join(".zshrc"),
            "alias captured_alias='printf profile'\n",
        )
        .unwrap();
        for program in ["/bin/bash", "/bin/zsh"] {
            for clean in [false, true] {
                let command = if program.ends_with("zsh") {
                    "if (( $+aliases[captured_alias] )); then captured_alias; else printf clean; fi"
                } else {
                    "case $- in *i*) exit 91;; esac; if shopt -q expand_aliases; then printf user; else printf clean; fi"
                };
                let result = futures_executor::block_on(fixture.future(
                    command.into(),
                    program,
                    clean,
                    Vec::new(),
                    CancellationToken::new(),
                ))
                .unwrap();
                assert_eq!(result.status, TerminalExecStatus::Exited { exit_code: 0 });
                assert_eq!(
                    fixture.stdout(&result),
                    if clean {
                        b"clean".as_slice()
                    } else if program.ends_with("zsh") {
                        b"profile".as_slice()
                    } else {
                        b"user".as_slice()
                    }
                );
            }
        }
    }

    #[test]
    fn captured_exec_output_limit_keeps_head_tail_and_independent_totals() {
        let fixture = Fixture::new(Duration::from_secs(5));
        let result = fixture.run("printf err >&2; /usr/bin/head -c 2097152 /dev/zero");
        assert_eq!(result.status, TerminalExecStatus::OutputLimit {});
        assert!(
            result.stdout.total_bytes + result.stderr.total_bytes
                > MAX_TERMINAL_ACTION_OUTPUT_BYTES as u64
        );
        assert_eq!(result.stderr.bytes, b"err");
        assert_eq!(result.stderr.total_bytes, 3);
        assert_eq!(
            result.stdout.bytes.len(),
            machine_god_core::MAX_TERMINAL_EXEC_STREAM_BYTES
        );
        assert!(fixture.stdout(&result).iter().all(|byte| *byte == 0));
        assert!(result.stdout.truncated());
    }

    #[test]
    fn captured_exec_unpolled_cancelled_and_dropped_futures_leave_no_process() {
        let fixture = Fixture::new(Duration::from_secs(10));
        let command = "printf '%s' \"$$\" > leader; exec /bin/sleep 30";
        drop(fixture.future(
            command.into(),
            "/bin/bash",
            true,
            Vec::new(),
            CancellationToken::new(),
        ));
        assert!(!fixture.root.join("leader").exists());
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            futures_executor::block_on(fixture.future(
                command.into(),
                "/bin/bash",
                true,
                Vec::new(),
                cancellation
            )),
            Err(TerminalCapturedExecError::Cancelled)
        );
        assert!(!fixture.root.join("leader").exists());
        for cancel in [false, true] {
            let cancellation = CancellationToken::new();
            let mut future = fixture.future(
                command.into(),
                "/bin/bash",
                true,
                Vec::new(),
                cancellation.clone(),
            );
            futures_executor::block_on(async {
                assert!(futures_util::poll!(&mut future).is_pending());
            });
            let until = Instant::now() + Duration::from_secs(5);
            while !fixture.root.join("leader").exists() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            let pid = std::fs::read_to_string(fixture.root.join("leader"))
                .unwrap()
                .parse::<i32>()
                .unwrap();
            if cancel {
                cancellation.cancel();
                assert_eq!(
                    futures_executor::block_on(future),
                    Err(TerminalCapturedExecError::Cancelled)
                );
            } else {
                drop(future);
            }
            let until = Instant::now() + Duration::from_secs(10);
            while fixture.executor.active.load(Ordering::Acquire) != 0 && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(fixture.executor.active.load(Ordering::Acquire), 0);
            assert_eq!(
                rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap()),
                Err(rustix::io::Errno::SRCH)
            );
            std::fs::remove_file(fixture.root.join("leader")).unwrap();
        }
    }

    #[test]
    fn captured_exec_validates_environment_and_bounds_active_workers() {
        let fixture = Fixture::new(Duration::from_secs(10));
        assert_eq!(
            futures_executor::block_on(fixture.future(
                "printf forbidden > forbidden".into(),
                "/bin/bash",
                true,
                vec![("INVALID".into(), "nul\0value".into())],
                CancellationToken::new()
            )),
            Err(TerminalCapturedExecError::Invalid)
        );
        assert!(!fixture.root.join("forbidden").exists());
        let mut futures = Vec::new();
        for _ in 0..2 {
            let mut future = fixture.future(
                "exec /bin/sleep 30".into(),
                "/bin/bash",
                true,
                Vec::new(),
                CancellationToken::new(),
            );
            futures_executor::block_on(async {
                assert!(futures_util::poll!(&mut future).is_pending());
            });
            futures.push(future);
        }
        assert_eq!(fixture.executor.active.load(Ordering::Acquire), 2);
        assert_eq!(
            futures_executor::block_on(fixture.future(
                "printf forbidden > forbidden".into(),
                "/bin/bash",
                true,
                Vec::new(),
                CancellationToken::new()
            )),
            Err(TerminalCapturedExecError::Capacity)
        );
        assert!(!fixture.root.join("forbidden").exists());
        drop(futures);
    }
}
