use super::{
    Arc, BoxFuture, CancelOnDrop, CancellationToken, Instant, McpStdioConnection, McpStdioError,
    NativeOwnedWorkerScope, Response, Result, Shared, WireLimits, fmt, worker,
};
use crate::background_process::ValidatedBackgroundEnvironment;
use crate::terminal_captured_exec::{GatedArgv, GatedProcess, launch_gated_argv};
use crate::terminal_helper::TerminalPtyHelper;
use std::ffi::OsString;
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_PATH_BYTES: usize = 8192;
const MAX_PATH_ENTRIES: usize = 256;
const MAX_STARTUP: Duration = Duration::from_secs(300);

/// Explicit launch inputs, copied and validated without I/O. Environment entries
/// are the complete selected target environment, not an ambient overlay. PATH is
/// captured separately by the host; relative entries resolve against retained cwd.
/// The helper must be the explicitly selected trusted executable/entrypoint.
pub struct McpStdioLaunch {
    helper: TerminalPtyHelper,
    command: String,
    arguments: Vec<String>,
    environment: ValidatedBackgroundEnvironment,
    search_path: Option<String>,
    cwd: Arc<File>,
    limits: WireLimits,
}
impl fmt::Debug for McpStdioLaunch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpStdioLaunch { <redacted> }")
    }
}
impl McpStdioLaunch {
    #[cfg(test)]
    pub(super) fn with_test_helper(mut self, helper: TerminalPtyHelper) -> Self {
        self.helper = helper;
        self
    }
    /// Builds bounded launch data only. The helper arguments select the existing
    /// captured-helper entrypoint; its private injected mode enables MCP stdin.
    ///
    /// # Errors
    /// Rejects malformed/oversized explicit inputs. Filesystem validation and
    /// executable lookup occur only on the owned worker after connection polling.
    #[allow(
        clippy::too_many_arguments,
        reason = "All launch authority is explicit; no ambient constructor inputs."
    )]
    pub fn new(
        helper_program: PathBuf,
        helper_arguments: Vec<OsString>,
        command: String,
        arguments: Vec<String>,
        environment: Vec<(OsString, OsString)>,
        search_path: Option<String>,
        cwd: Arc<File>,
        limits: WireLimits,
    ) -> Result<Self> {
        if command.is_empty()
            || command.len() > 4096
            || command.contains('\0')
            || search_path.as_ref().is_some_and(|path| {
                path.len() > MAX_PATH_BYTES
                    || path.contains('\0')
                    || path.split(':').count() > MAX_PATH_ENTRIES
            })
        {
            return Err(McpStdioError::Invalid);
        }
        // The existing codec's absolute-program rule is applied after lookup.
        crate::terminal_helper::validate_program_arguments("/mcp", &arguments)
            .map_err(|_| McpStdioError::Invalid)?;
        let helper = TerminalPtyHelper::new(helper_program, helper_arguments)
            .map_err(|_| McpStdioError::Invalid)?;
        let environment =
            ValidatedBackgroundEnvironment::new(environment).map_err(|_| McpStdioError::Invalid)?;
        Ok(Self {
            helper,
            command,
            arguments,
            environment,
            search_path,
            cwd,
            limits: limits.validate().map_err(|_| McpStdioError::Invalid)?,
        })
    }

    /// Injects the explicit shared macOS inventory service, without starting it.
    ///
    /// # Errors
    /// Rejects invalid helper selection.
    #[cfg(target_os = "macos")]
    pub fn with_process_inventory_service(
        mut self,
        program: PathBuf,
        arguments: Vec<OsString>,
    ) -> Result<Self> {
        let inventory = crate::process_inventory_helper::ProcessInventoryHelper::new_service(
            program, arguments,
        )
        .map_err(|_| McpStdioError::Invalid)?;
        self.helper = self.helper.with_inventory_helper(inventory);
        Ok(self)
    }

    /// Opens on first poll only. Startup deadline is finite (at most 300 seconds)
    /// and does not become an arbitrary lifetime limit on the connected server.
    /// `keepalive` stays retained through positive child reap, including deferred
    /// cleanup. Host cancellation remains live after successful startup.
    ///
    /// # Errors
    /// Rejects cancelled/expired startup, missing executable/helper, capacity,
    /// process ownership failure and failed helper exec receipt.
    #[must_use]
    pub fn connect(
        self,
        host: NativeOwnedWorkerScope,
        deadline: Instant,
        cancellation: CancellationToken,
        keepalive: Box<dyn Send>,
    ) -> BoxFuture<'static, Result<McpStdioConnection>> {
        Box::pin(async move {
            check_start(deadline, &cancellation)?;
            if deadline.saturating_duration_since(Instant::now()) > MAX_STARTUP {
                return Err(McpStdioError::Invalid);
            }
            let stop = CancellationToken::new();
            let mut abandon = CancelOnDrop(Some(stop.clone()));
            let shared = Arc::new(Shared::new(self.limits, stop));
            let startup = Arc::new(Response::new());
            let child_scope = NativeOwnedWorkerScope::new();
            let completion = child_scope.completion();
            let worker_shared = shared.clone();
            let worker_startup = startup.clone();
            host.spawn(move || {
                let owner = OwnerWait {
                    scope: child_scope,
                    shared: worker_shared.clone(),
                    startup: worker_startup.clone(),
                };
                let admission = check_start(deadline, &cancellation)
                    .and_then(|()| worker_shared.check())
                    .and_then(|()| {
                        owner
                            .scope
                            .spawn(move || {
                                let _finish = Finish {
                                    shared: worker_shared.clone(),
                                    startup: worker_startup.clone(),
                                };
                                match self.launch(
                                    deadline,
                                    &cancellation,
                                    &worker_shared,
                                    keepalive,
                                ) {
                                    Ok(process) => {
                                        worker_startup.complete(Ok(()));
                                        worker::run(process, &worker_shared, &cancellation);
                                    }
                                    Err(error) => {
                                        worker_shared.finish(error);
                                        worker_startup.complete(Err(error));
                                    }
                                }
                            })
                            .map_err(|_| McpStdioError::Capacity)
                    });
                if let Err(error) = admission {
                    owner.shared.finish(error);
                    owner.startup.complete(Err(error));
                }
                // OwnerWait keeps this host worker enrolled until all nested
                // child-scope cleanup settles, even when admission unwinds.
            })
            .map_err(|_| McpStdioError::Capacity)?;
            startup.wait().await?;
            // A server can send a final response and close immediately after
            // exec. Preserve the admitted connection and queued frame.
            // Move ownership of stop to the returned connection, without firing
            // the abandonment cancellation guard.
            abandon.0 = None;
            Ok(McpStdioConnection { shared, completion })
        })
    }

    fn launch(
        self,
        deadline: Instant,
        cancellation: &CancellationToken,
        shared: &Shared,
        keepalive: Box<dyn Send>,
    ) -> Result<GatedProcess> {
        check_start(deadline, cancellation)?;
        shared.check()?;
        let program = self.resolve_program(deadline, cancellation, shared)?;
        let cwd =
            rustix::io::fcntl_dupfd_cloexec(&*self.cwd, 3).map_err(|_| McpStdioError::Process)?;
        let retained: Box<dyn Send> = Box::new((keepalive, self.cwd.clone()));
        let result = launch_gated_argv(GatedArgv {
            helper: &self.helper,
            program: &program,
            arguments: &self.arguments,
            environment: &self.environment,
            cwd,
            deadline,
            cancellation,
            stop: &[&shared.stop],
            keepalive: Some(retained),
            persistent_stdin: true,
            before_commit: &mut || {
                Ok(!shared.stop.is_cancelled()
                    && !cancellation.is_cancelled()
                    && Instant::now() < deadline)
            },
        });
        match result {
            Ok(Some(process)) => Ok(process),
            Ok(None) => Err(McpStdioError::Deadline),
            Err(_) if cancellation.is_cancelled() || shared.stop.is_cancelled() => {
                Err(McpStdioError::Cancelled)
            }
            Err(_) if Instant::now() >= deadline => Err(McpStdioError::Deadline),
            Err(_) => Err(McpStdioError::Process),
        }
    }

    fn resolve_program(
        &self,
        deadline: Instant,
        cancellation: &CancellationToken,
        shared: &Shared,
    ) -> Result<String> {
        let cwd = cwd_lookup_path(&self.cwd)?;
        let command = Path::new(&self.command);
        if command.is_absolute() || self.command.contains('/') {
            let result = executable(if command.is_absolute() {
                command.to_path_buf()
            } else {
                cwd.join(command)
            });
            check_cwd_path(&self.cwd, &cwd)?;
            return result;
        }
        let path = self.search_path.as_deref().ok_or(McpStdioError::Invalid)?;
        for entry in path.split(':') {
            check_start(deadline, cancellation)?;
            shared.check()?;
            let entry = Path::new(entry);
            let candidate = if entry.is_absolute() {
                entry.join(command)
            } else {
                cwd.join(entry).join(command)
            };
            let result = executable(candidate);
            check_cwd_path(&self.cwd, &cwd)?;
            if let Ok(program) = result {
                return Ok(program);
            }
        }
        Err(McpStdioError::Process)
    }
}

fn cwd_lookup_path(cwd: &File) -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    let path = PathBuf::from(format!("/proc/self/fd/{}", cwd.as_raw_fd()));
    #[cfg(target_os = "macos")]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(OsString::from_vec(
            rustix::fs::getpath(cwd)
                .map_err(|_| McpStdioError::Process)?
                .into_bytes(),
        ))
    };
    check_cwd_path(cwd, &path)?;
    Ok(path)
}
fn check_cwd_path(cwd: &File, path: &Path) -> Result<()> {
    let retained = rustix::fs::fstat(cwd).map_err(|_| McpStdioError::Process)?;
    let observed = rustix::fs::stat(path).map_err(|_| McpStdioError::Process)?;
    if retained.st_dev != observed.st_dev
        || retained.st_ino != observed.st_ino
        || retained.st_nlink == 0
        || !rustix::fs::FileType::from_raw_mode(retained.st_mode).is_dir()
    {
        Err(McpStdioError::Process)
    } else {
        Ok(())
    }
}

fn executable(path: PathBuf) -> Result<String> {
    if path.as_os_str().as_bytes().len() > MAX_PATH_BYTES + 4096 {
        return Err(McpStdioError::Invalid);
    }
    let path = std::fs::canonicalize(path).map_err(|_| McpStdioError::Process)?;
    let metadata = std::fs::metadata(&path).map_err(|_| McpStdioError::Process)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(McpStdioError::Process);
    }
    let path = path
        .into_os_string()
        .into_string()
        .map_err(|_| McpStdioError::Invalid)?;
    crate::terminal_helper::validate_program_arguments(&path, &[])
        .map_err(|_| McpStdioError::Invalid)?;
    Ok(path)
}
fn check_start(deadline: Instant, cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(McpStdioError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(McpStdioError::Deadline)
    } else {
        Ok(())
    }
}

struct Finish {
    shared: Arc<Shared>,
    startup: Arc<Response<()>>,
}
impl Drop for Finish {
    fn drop(&mut self) {
        self.shared.finish(McpStdioError::Process);
        self.startup.complete(Err(McpStdioError::Process));
    }
}
struct OwnerWait {
    scope: NativeOwnedWorkerScope,
    shared: Arc<Shared>,
    startup: Arc<Response<()>>,
}
impl Drop for OwnerWait {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.shared.stop.cancel();
        }
        self.scope.close();
        // This worker belongs to the distinct parent scope, never this scope.
        self.scope
            .completion()
            .wait_on_worker()
            .expect("distinct MCP connection scope");
        self.shared.finish(McpStdioError::Process);
        self.startup.complete(Err(McpStdioError::Process));
    }
}

#[cfg(test)]
mod tests;
