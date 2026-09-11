//! Explicit Git execution on the managed-installation effect worker.
//! No ambient executable, environment or directory selection occurs here.
#![cfg(any(target_os = "linux", target_os = "macos"))]

#[cfg(test)]
mod tests;
mod watchdog;

use crate::background_process::ValidatedBackgroundEnvironment;
use crate::skills_managed::{
    NativeSkillGitRequest, NativeSkillGitRunner, NativeSkillManagedError,
    NativeSkillManagedErrorKind as Kind,
};
use crate::terminal_captured_exec::{
    CapturedArgv, TerminalCapturedExecError, execute_argv_on_worker,
};
use crate::terminal_helper::TerminalPtyHelper;
use machine_god_core::{CancellationToken, TerminalExecStatus};
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_DURATION: Duration = Duration::from_secs(120);
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_CLONE_BYTES: u64 = 256 * 1024 * 1024;

/// Inert, explicitly configured production adapter. Runs only on the caller's
/// owned effect worker. Cleanup owns the original group and positively captured
/// members, not descendants which escape before any ownership observation.
pub struct SystemNativeSkillGitRunner {
    program: String,
    helper: TerminalPtyHelper,
    environment: ValidatedBackgroundEnvironment,
}

impl fmt::Debug for SystemNativeSkillGitRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemNativeSkillGitRunner")
            .finish_non_exhaustive()
    }
}

impl SystemNativeSkillGitRunner {
    /// Constructs explicit launch authority without IO. The helper must implement
    /// the existing captured-exec private protocol. Environment input is a selected
    /// snapshot, not the complete ambient environment; Git/loader injection keys
    /// are deliberately excluded. Credentials and proxies are never logged.
    ///
    /// # Errors
    /// Rejects invalid executable/helper paths or environment keys and bounds.
    pub fn new(
        program: PathBuf,
        helper_program: PathBuf,
        helper_arguments: Vec<OsString>,
        mut environment: Vec<(OsString, OsString)>,
    ) -> Result<Self, NativeSkillManagedError> {
        if !program.is_absolute() {
            return Err(Kind::InvalidSource.into());
        }
        let program = program
            .into_os_string()
            .into_string()
            .map_err(|_| Kind::InvalidSource)?;
        if program.contains('\0') || program.len() > 4096 {
            return Err(Kind::InvalidSource.into());
        }
        if environment.len()
            > crate::background_process::MAX_BACKGROUND_PROCESS_ENVIRONMENT_ENTRIES - 5
            || environment
                .iter()
                .any(|(key, _)| !allowed_environment_key(key))
        {
            return Err(Kind::InvalidSource.into());
        }
        environment.extend(
            [
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_CONFIG_SYSTEM", "/dev/null"),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_TERMINAL_PROMPT", "0"),
                ("GIT_OPTIONAL_LOCKS", "0"),
            ]
            .map(|(key, value)| (key.into(), value.into())),
        );
        Ok(Self {
            program,
            helper: TerminalPtyHelper::new(helper_program, helper_arguments)
                .map_err(|_| Kind::InvalidSource)?,
            environment: ValidatedBackgroundEnvironment::new(environment)
                .map_err(|_| Kind::InvalidSource)?,
        })
    }

    /// Selects the explicit reusable inventory helper, with no fallback if it fails.
    /// # Errors
    /// Rejects invalid helper configuration without launching it.
    #[cfg(target_os = "macos")]
    pub fn with_process_inventory_service(
        mut self,
        program: PathBuf,
        arguments: Vec<OsString>,
    ) -> Result<Self, NativeSkillManagedError> {
        let inventory = crate::process_inventory_helper::ProcessInventoryHelper::new_service(
            program, arguments,
        )
        .map_err(|_| Kind::InvalidSource)?;
        self.helper = self.helper.with_inventory_helper(inventory);
        Ok(self)
    }
}

fn allowed_environment_key(key: &std::ffi::OsStr) -> bool {
    matches!(
        key.to_str(),
        Some(
            "PATH"
                | "HOME"
                | "LANG"
                | "LC_ALL"
                | "USER"
                | "LOGNAME"
                | "TMPDIR"
                | "SSH_AUTH_SOCK"
                | "SSH_AGENT_PID"
                | "SSL_CERT_FILE"
                | "SSL_CERT_DIR"
                | "HTTPS_PROXY"
                | "HTTP_PROXY"
                | "ALL_PROXY"
                | "NO_PROXY"
                | "https_proxy"
                | "http_proxy"
                | "all_proxy"
                | "no_proxy"
        )
    )
}

impl NativeSkillGitRunner for SystemNativeSkillGitRunner {
    fn clone_repository(
        &self,
        request: NativeSkillGitRequest,
        cancellation: &CancellationToken,
    ) -> Result<(), NativeSkillManagedError> {
        let now = Instant::now();
        if cancellation.is_cancelled() {
            return Err(Kind::Cancelled.into());
        }
        if now >= request.deadline {
            return Err(Kind::TimedOut.into());
        }
        if request.deadline.duration_since(now) > MAX_DURATION
            || request.max_output_bytes == 0
            || request.max_output_bytes > MAX_OUTPUT
            || request.clone_rejection_bytes == 0
            || request.clone_rejection_bytes > MAX_CLONE_BYTES
        {
            return Err(Kind::ResourceLimit.into());
        }
        validate_url(&request.url)?;
        let arguments = clone_arguments(&request.url);
        let cwd = rustix::io::fcntl_dupfd_cloexec(&*request.directory, 3)
            .map_err(|_| Kind::Unavailable)?;
        let mut watchdog = watchdog::CloneWatchdog::new(
            Arc::clone(&request.directory),
            request.clone_rejection_bytes,
            request.deadline,
            cancellation,
        );
        watchdog.check(true)?;
        let mut watch_error = None;
        let mut observe = || {
            watchdog.check(false).map_err(|error| {
                watch_error = Some(error);
                TerminalCapturedExecError::Process
            })
        };
        let outcome = execute_argv_on_worker(CapturedArgv {
            helper: &self.helper,
            program: &self.program,
            arguments: &arguments,
            environment: &self.environment,
            cwd,
            deadline: request.deadline,
            started: now,
            output_limit: request.max_output_bytes,
            cancellation,
            stop: &[],
            keepalive: Some(Box::new((request.lease, Arc::clone(&request.directory)))),
            before_commit: &mut || Ok(true),
            observe: &mut observe,
        });
        if let Some(error) = watch_error {
            return Err(error);
        }
        match outcome {
            Ok(TerminalExecStatus::Exited { exit_code: 0 }) => watchdog.check(true),
            Ok(TerminalExecStatus::TimedOut { .. }) => Err(Kind::TimedOut.into()),
            Ok(TerminalExecStatus::OutputLimit { .. }) => Err(Kind::ResourceLimit.into()),
            Err(TerminalCapturedExecError::Cancelled) => Err(Kind::Cancelled.into()),
            Ok(_) | Err(_) => Err(Kind::GitFailed.into()),
        }
    }
}

fn validate_url(url: &str) -> Result<(), NativeSkillManagedError> {
    let source = crate::skills_managed::NativeSkillInstallSource::parse(url, None)?;
    if source.kind() != crate::skills_managed::NativeSkillSourceKind::Git
        || source.source() != url
        || source.filter().is_some()
    {
        return Err(Kind::InvalidSource.into());
    }
    Ok(())
}

fn clone_arguments(url: &str) -> Vec<String> {
    [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "init.templateDir=",
        "-c",
        "protocol.allow=never",
        "-c",
        "protocol.https.allow=always",
        "-c",
        "protocol.http.allow=always",
        "-c",
        "protocol.ssh.allow=always",
        "-c",
        "credential.interactive=false",
        "clone",
        "--depth",
        "1",
        "--no-tags",
        "--single-branch",
        "--no-recurse-submodules",
        "--",
        url,
        ".",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
