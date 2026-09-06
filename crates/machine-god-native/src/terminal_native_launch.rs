//! Worker-only composition of the existing owned native terminal transports.
//! Selection is inert; preparation takes explicit descriptors and one deadline.

use crate::terminal_helper::{MAX_STARTUP_TIMEOUT, startup_directory_identity};
use crate::terminal_native_backend::TerminalNativeBackend;
use crate::terminal_pty::{TerminalPtyDimensions, TerminalPtyHelper};
use crate::terminal_shell::{TerminalShell, TerminalShellError};
use crate::terminal_startup::{
    PreparedTerminalBootstrap, PreparedTerminalStartup, PublishedTerminalBootstrap,
    TerminalStartupControl, TerminalStartupError, TerminalStartupRequest,
};
use crate::terminal_tmux_helper::TerminalTmuxLaunchError;
use crate::terminal_tmux_startup::{PreparedTerminalTmuxLaunch, TerminalTmuxLaunchRequest};
use machine_god_core::{
    CancellationToken, TerminalBackend, TerminalDimensions, TerminalProfile, TerminalShellSpec,
    TerminalStartRequest,
};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType};
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::time::Instant;

/// Explicit captured host configuration. Construction performs no discovery,
/// filesystem validation, helper execution, or process creation.
pub(crate) struct TerminalNativeLaunchConfig {
    pub(crate) account_shell: Option<PathBuf>,
    pub(crate) pty_helper: TerminalPtyHelper,
    pub(crate) marker_helper: TerminalPtyHelper,
    pub(crate) tmux: Option<TerminalNativeTmuxConfig>,
}
pub(crate) struct TerminalNativeTmuxConfig {
    pub(crate) executable: PathBuf,
    pub(crate) pane_helper: TerminalPtyHelper,
    pub(crate) capture_helper: TerminalPtyHelper,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalNativeLaunchError {
    Invalid,
    Shell(TerminalShellError),
    UnavailableBackend,
    Cancelled,
    Timeout,
    Process,
    Startup(TerminalStartupError),
    Tmux(TerminalTmuxLaunchError),
}
impl fmt::Display for TerminalNativeLaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native terminal launch failed")
    }
}
impl std::error::Error for TerminalNativeLaunchError {}
type Result<T> = std::result::Result<T, TerminalNativeLaunchError>;

/// Resolved, effect-free request data. It is not launch permission or native
/// process authority. Waits and monitors remain the registry owner's work.
pub(crate) struct ResolvedTerminalNativeLaunch {
    shell: TerminalShell,
    command: Option<String>,
    cwd: String,
    backend: TerminalBackend,
    dimensions: TerminalDimensions,
}
impl ResolvedTerminalNativeLaunch {
    pub(crate) fn resolve(
        config: &TerminalNativeLaunchConfig,
        request: &TerminalStartRequest,
    ) -> Result<Self> {
        request
            .validate()
            .map_err(|_| TerminalNativeLaunchError::Invalid)?;
        let shell = match request.shell.as_ref() {
            Some(TerminalShellSpec::Executable { path, clean_start }) => {
                TerminalShell::from_executable(path.as_ref(), *clean_start)
            }
            Some(TerminalShellSpec::UserLogin {}) | None => TerminalShell::from_account_shell(
                config.account_shell.as_deref(),
                request.profile,
                None,
            ),
        }
        .map_err(TerminalNativeLaunchError::Shell)?;
        if request.backend == TerminalBackend::Tmux {
            let tmux = config
                .tmux
                .as_ref()
                .ok_or(TerminalNativeLaunchError::UnavailableBackend)?;
            let path = tmux
                .executable
                .to_str()
                .ok_or(TerminalNativeLaunchError::Invalid)?;
            if !tmux.executable.is_absolute() || path.len() > 4096 || path.contains('\0') {
                return Err(TerminalNativeLaunchError::Invalid);
            }
        }
        // Pinned native_session.zig default_dimensions is 24 rows, 80 columns.
        let dimensions = request
            .dimensions
            .clone()
            .map_or_else(|| TerminalDimensions::new(24, 80), Ok)
            .map_err(|_| TerminalNativeLaunchError::Invalid)?;
        Ok(Self {
            shell,
            command: request.command.clone(),
            cwd: request.cwd.clone(),
            backend: request.backend,
            dimensions,
        })
    }

    #[cfg(test)]
    pub(crate) fn shell(&self) -> &TerminalShell {
        &self.shell
    }
    #[cfg(test)]
    pub(crate) fn dimensions(&self) -> &TerminalDimensions {
        &self.dimensions
    }

    /// Blocking worker only. All resources remain owned through failures and
    /// the eventual COMMIT; this function does not acknowledge startup markers.
    pub(crate) fn prepare(
        self,
        config: &TerminalNativeLaunchConfig,
        authority: TerminalNativeLaunchAuthority,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<PreparedTerminalNativeLaunch> {
        check_deadline(deadline, cancellation)?;
        validate_cwd(&authority.cwd, &self.cwd)?;
        let cwd = rustix::io::fcntl_dupfd_cloexec(&authority.cwd, 3).map_err(process_error)?;
        let dimensions = TerminalPtyDimensions {
            rows: self.dimensions.rows(),
            columns: self.dimensions.columns(),
        };
        let mut identity = TerminalNativeLaunchIdentity {
            backend: self.backend,
            backend_identity: String::new(),
            shell: self
                .shell
                .program()
                .to_str()
                .ok_or(TerminalNativeLaunchError::Invalid)?
                .into(),
            profile: self.shell.profile(),
            cwd: self.cwd.clone(),
            command: self.command.clone(),
            dimensions: self.dimensions,
        };
        let transport = match self.backend {
            TerminalBackend::Native => {
                identity.backend_identity = native_identity()?;
                let request = TerminalStartupRequest {
                    shell: self.shell,
                    command: self.command,
                    environment: authority.environment,
                    cwd: authority.cwd,
                    artifacts: authority.artifacts,
                    artifact_path: authority.artifact_path,
                    marker_helper: copy_helper(&config.marker_helper)?,
                    dimensions,
                    timeout: MAX_STARTUP_TIMEOUT,
                };
                PreparedTransport::Pty(Box::new(
                    PreparedTerminalStartup::prepare_until(
                        &config.pty_helper,
                        request,
                        deadline,
                        cancellation,
                    )
                    .map_err(startup_error)?,
                ))
            }
            TerminalBackend::Tmux => {
                let tmux = config
                    .tmux
                    .as_ref()
                    .ok_or(TerminalNativeLaunchError::UnavailableBackend)?;
                let bootstrap = PreparedTerminalBootstrap::new(
                    &self.shell,
                    self.command.as_deref(),
                    rustix::io::fcntl_dupfd_cloexec(&authority.artifacts, 3)
                        .map_err(process_error)?,
                    authority.artifact_path.clone(),
                    &config.marker_helper,
                    deadline,
                    cancellation,
                )
                .map_err(startup_error)?;
                let request = TerminalTmuxLaunchRequest {
                    executable: tmux.executable.clone(),
                    helper: copy_helper(&tmux.pane_helper)?,
                    capture_helper: copy_helper(&tmux.capture_helper)?,
                    program: bootstrap.program().into(),
                    arguments: bootstrap.arguments().to_vec(),
                    initial_source: bootstrap.startup_source().map(str::to_owned),
                    environment: authority.environment,
                    cwd: authority.cwd,
                    cwd_path: self.cwd.into(),
                    artifacts: authority.artifacts,
                    artifact_path: authority.artifact_path,
                    dimensions,
                    timeout: MAX_STARTUP_TIMEOUT,
                };
                let prepared =
                    PreparedTerminalTmuxLaunch::prepare_until(request, deadline, cancellation)
                        .map_err(tmux_error)?;
                identity.backend_identity = format!(
                    "tmux:{}:{}",
                    prepared.identity().namespace(),
                    prepared.identity().pane()
                );
                let bootstrap = bootstrap.publish(cancellation).map_err(startup_error)?;
                PreparedTransport::Tmux(Box::new(PreparedTmuxLaunch {
                    prepared,
                    bootstrap,
                }))
            }
        };
        check_deadline(deadline, cancellation)?;
        Ok(PreparedTerminalNativeLaunch {
            transport,
            identity,
            deadline,
            cwd,
        })
    }
}

/// Explicit worker authority, not inferred from model paths or ambient state.
pub(crate) struct TerminalNativeLaunchAuthority {
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) cwd: OwnedFd,
    pub(crate) artifacts: OwnedFd,
    pub(crate) artifact_path: PathBuf,
}

/// Display-only launch facts. Backend identity is never reconstructed into a
/// process capability; the returned backend alone retains native authority.
pub(crate) struct TerminalNativeLaunchIdentity {
    pub(crate) backend: TerminalBackend,
    pub(crate) backend_identity: String,
    pub(crate) shell: String,
    pub(crate) profile: TerminalProfile,
    pub(crate) cwd: String,
    pub(crate) command: Option<String>,
    pub(crate) dimensions: TerminalDimensions,
}
impl fmt::Debug for TerminalNativeLaunchIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalNativeLaunchIdentity")
            .finish_non_exhaustive()
    }
}
enum PreparedTransport {
    Pty(Box<PreparedTerminalStartup>),
    Tmux(Box<PreparedTmuxLaunch>),
}
struct PreparedTmuxLaunch {
    prepared: PreparedTerminalTmuxLaunch,
    bootstrap: PublishedTerminalBootstrap,
}
pub(crate) struct PreparedTerminalNativeLaunch {
    transport: PreparedTransport,
    identity: TerminalNativeLaunchIdentity,
    deadline: Instant,
    cwd: OwnedFd,
}
impl PreparedTerminalNativeLaunch {
    #[cfg(test)]
    pub(crate) fn identity(&self) -> &TerminalNativeLaunchIdentity {
        &self.identity
    }

    /// Blocking worker only. Preserve the returned control until its durable
    /// startup acknowledgements finish; dropping a tool future is not cleanup.
    pub(crate) fn commit(
        self,
        cancellation: &CancellationToken,
    ) -> Result<(
        TerminalNativeBackend,
        TerminalStartupControl,
        TerminalNativeLaunchIdentity,
    )> {
        check_deadline(self.deadline, cancellation)?;
        validate_cwd(&self.cwd, &self.identity.cwd)?;
        check_deadline(self.deadline, cancellation)?;
        let (backend, control) = match self.transport {
            PreparedTransport::Pty(prepared) => {
                let (backend, control) = prepared.commit(cancellation).map_err(startup_error)?;
                (TerminalNativeBackend::Pty(Box::new(backend)), control)
            }
            PreparedTransport::Tmux(prepared) => {
                let PreparedTmuxLaunch {
                    prepared,
                    bootstrap,
                } = *prepared;
                let backend = prepared.commit_owned(cancellation).map_err(tmux_error)?;
                let (backend, control) = bootstrap.attach(backend);
                (TerminalNativeBackend::Tmux(Box::new(backend)), control)
            }
        };
        Ok((backend, control, self.identity))
    }
}

fn check_deadline(deadline: Instant, cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(TerminalNativeLaunchError::Cancelled);
    }
    let now = Instant::now();
    if deadline <= now {
        return Err(TerminalNativeLaunchError::Timeout);
    }
    if deadline.duration_since(now) > MAX_STARTUP_TIMEOUT {
        return Err(TerminalNativeLaunchError::Invalid);
    }
    Ok(())
}
fn startup_error(error: TerminalStartupError) -> TerminalNativeLaunchError {
    match error {
        TerminalStartupError::Cancelled => TerminalNativeLaunchError::Cancelled,
        TerminalStartupError::Timeout => TerminalNativeLaunchError::Timeout,
        other => TerminalNativeLaunchError::Startup(other),
    }
}
fn tmux_error(error: TerminalTmuxLaunchError) -> TerminalNativeLaunchError {
    match error {
        TerminalTmuxLaunchError::Cancelled => TerminalNativeLaunchError::Cancelled,
        TerminalTmuxLaunchError::Timeout => TerminalNativeLaunchError::Timeout,
        other => TerminalNativeLaunchError::Tmux(other),
    }
}
fn process_error(_: impl fmt::Debug) -> TerminalNativeLaunchError {
    TerminalNativeLaunchError::Process
}
fn copy_helper(helper: &TerminalPtyHelper) -> Result<TerminalPtyHelper> {
    TerminalPtyHelper::new(helper.program().to_owned(), helper.arguments().to_vec())
        .map_err(process_error)
}
fn validate_cwd(descriptor: &OwnedFd, path: &str) -> Result<()> {
    let held = rustix::fs::fstat(descriptor).map_err(process_error)?;
    let observed = rustix::fs::statat(rustix::fs::CWD, path, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(process_error)?;
    if FileType::from_raw_mode(held.st_mode) != FileType::Directory
        || FileType::from_raw_mode(observed.st_mode) != FileType::Directory
        || startup_directory_identity(&held) != startup_directory_identity(&observed)
    {
        return Err(TerminalNativeLaunchError::Invalid);
    }
    Ok(())
}
fn native_identity() -> Result<String> {
    use std::fmt::Write;
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(process_error)?;
    let mut identity = String::with_capacity(39);
    identity.push_str("native:");
    for byte in random {
        write!(identity, "{byte:02x}").map_err(process_error)?;
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_pty::TerminalPtyStatus;
    use crate::terminal_session::TerminalSessionBackend;
    use crate::terminal_startup::TerminalStartupEvent;
    use machine_god_core::TerminalReturnCondition;
    use rustix::fs::{Mode, OFlags};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Duration;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let id = native_identity().unwrap();
            let path = PathBuf::from("/tmp").join(format!("mg-nl-{}", &id[7..19]));
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
        fn empty(&self) -> bool {
            std::fs::read_dir(&self.0).unwrap().next().is_none()
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn helper(mode: &str, entry: &str) -> TerminalPtyHelper {
        if let Some(program) = std::env::var_os("MACHINE_GOD_TERMINAL_RELEASE_BINARY") {
            return TerminalPtyHelper::new(program.into(), vec![mode.into()]).unwrap();
        }
        let program = std::env::current_exe().unwrap();
        if mode == crate::terminal_tmux_helper::TERMINAL_TMUX_HELPER_ARGUMENT {
            let script = format!(
                "export MG_TMUX_KIND=\"$1\" MG_TMUX_SOCKET=\"$2\" MG_TMUX_NONCE=\"$3\" MG_TMUX_CWD=\"$4\"; if [ \"$1\" = exec ]; then exec 2>&1; exec 1>/dev/null; fi; exec '{}' --exact terminal_tmux_startup::tests::helper_entry --nocapture",
                program.to_str().unwrap().replace('\'', "'\\''")
            );
            return TerminalPtyHelper::new(
                "/bin/sh".into(),
                vec!["-c".into(), script.into(), "helper".into()],
            )
            .unwrap();
        }
        TerminalPtyHelper::new(
            program,
            vec![
                "--exact".into(),
                entry.into(),
                "--test-threads=1".into(),
                "--quiet".into(),
            ],
        )
        .unwrap()
    }
    fn config() -> TerminalNativeLaunchConfig {
        #[cfg(target_os = "linux")]
        {
            static SUBREAPER: std::sync::Once = std::sync::Once::new();
            SUBREAPER.call_once(|| {
                rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1)).unwrap()
            });
        }
        let executable = if let Some(path) = std::env::var_os("MACHINE_GOD_TERMINAL_TMUX_BINARY") {
            let path = PathBuf::from(path);
            assert!(path.is_absolute() && path.is_file());
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
        TerminalNativeLaunchConfig {
            account_shell: Some("/bin/bash".into()),
            pty_helper: helper(
                crate::terminal_helper::TERMINAL_PTY_HELPER_ARGUMENT,
                "terminal_pty::tests::helper_entry",
            ),
            marker_helper: helper(
                crate::terminal_helper::TERMINAL_STARTUP_MARKER_ARGUMENT,
                "terminal_startup::tests::marker_helper_entry",
            ),
            tmux: executable.map(|executable| TerminalNativeTmuxConfig {
                executable,
                pane_helper: helper(
                    crate::terminal_tmux_helper::TERMINAL_TMUX_HELPER_ARGUMENT,
                    "",
                ),
                capture_helper: helper(
                    crate::terminal_tmux_helper::TERMINAL_TMUX_HELPER_ARGUMENT,
                    "",
                ),
            }),
        }
    }
    fn authority(cwd: &Directory, artifacts: &Directory) -> TerminalNativeLaunchAuthority {
        TerminalNativeLaunchAuthority {
            environment: vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TERM".into(), "xterm-256color".into()),
                ("HOME".into(), cwd.0.clone().into_os_string()),
                ("ZDOTDIR".into(), cwd.0.clone().into_os_string()),
            ],
            cwd: cwd.fd(),
            artifacts: artifacts.fd(),
            artifact_path: artifacts.0.clone(),
        }
    }
    fn request(
        cwd: &Directory,
        backend: TerminalBackend,
        command: Option<String>,
    ) -> TerminalStartRequest {
        let mut request = TerminalStartRequest::interactive(cwd.0.to_str().unwrap()).unwrap();
        request.backend = backend;
        request.profile = Some(TerminalProfile::Clean);
        request.return_when = command.as_ref().map(|_| TerminalReturnCondition::Started);
        request.command = command;
        request
    }
    fn prepare(
        config: &TerminalNativeLaunchConfig,
        request: &TerminalStartRequest,
        cwd: &Directory,
        artifacts: &Directory,
        deadline: Instant,
    ) -> PreparedTerminalNativeLaunch {
        ResolvedTerminalNativeLaunch::resolve(config, request)
            .unwrap()
            .prepare(
                config,
                authority(cwd, artifacts),
                deadline,
                &CancellationToken::new(),
            )
            .unwrap()
    }
    fn read(backend: &mut TerminalNativeBackend, output: &mut Vec<u8>) {
        let mut bytes = [0; 4096];
        let result = backend.read(&mut bytes).unwrap();
        output.extend_from_slice(&bytes[..result.bytes_read]);
        assert!(output.len() < 1024 * 1024);
    }
    fn event(
        backend: &mut TerminalNativeBackend,
        control: &mut TerminalStartupControl,
        expected: TerminalStartupEvent,
        output: &mut Vec<u8>,
        deadline: Instant,
    ) {
        loop {
            read(backend, output);
            if let Some(event) = control
                .poll(Instant::now(), &CancellationToken::new())
                .unwrap()
            {
                assert_eq!(event, expected);
                return;
            }
            assert!(Instant::now() < deadline, "output: {output:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn shell_ack(backend: &mut TerminalNativeBackend, control: &mut TerminalStartupControl) {
        backend.restore_startup_echo().unwrap();
        assert!(
            control
                .acknowledge_shell_ready(Instant::now(), &CancellationToken::new())
                .unwrap()
        );
    }
    fn send(backend: &mut TerminalNativeBackend, bytes: &[u8], paste: bool, deadline: Instant) {
        let mut offset = 0;
        while offset != bytes.len() {
            let end = bytes.len().min(offset + backend.input_write_limit());
            let receipt = backend
                .write_with_paste(&bytes[offset..end], paste)
                .unwrap();
            offset += receipt.bytes_written();
            assert!(Instant::now() < deadline);
            if receipt.bytes_written() == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    fn finish(
        backend: &mut TerminalNativeBackend,
        output: &mut Vec<u8>,
        expected: i32,
        deadline: Instant,
    ) {
        while backend.status().unwrap() == TerminalPtyStatus::Running {
            read(backend, output);
            assert!(Instant::now() < deadline, "output: {output:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
        let receipt = backend
            .close(false, &mut |bytes| output.extend_from_slice(bytes))
            .unwrap();
        assert_eq!(receipt.status, TerminalPtyStatus::Exited(expected));
        // macOS PTY close conservatively preserves the existing TIOCSIG gap;
        // tmux has its separate authenticated successful-capture receipt.
        if matches!(backend, TerminalNativeBackend::Tmux(_)) {
            assert!(!receipt.output_incomplete);
        }
    }

    #[test]
    fn resolution_is_inert_and_preserves_normalized_selectors_and_defaults() {
        let mut config = config();
        config.account_shell = Some("/nonexistent/account/bash".into());
        let mut request = TerminalStartRequest::interactive("/not/a/real/directory").unwrap();
        let resolved = ResolvedTerminalNativeLaunch::resolve(&config, &request).unwrap();
        assert_eq!(
            resolved.shell().program(),
            Path::new("/nonexistent/account/bash")
        );
        assert_eq!(resolved.shell().profile(), TerminalProfile::User);
        assert_eq!(
            resolved.dimensions(),
            &TerminalDimensions::new(24, 80).unwrap()
        );
        config.account_shell = None;
        assert!(matches!(
            ResolvedTerminalNativeLaunch::resolve(&config, &request),
            Err(TerminalNativeLaunchError::Shell(
                TerminalShellError::MissingLoginShell
            ))
        ));
        request.shell = Some(TerminalShellSpec::Executable {
            path: "/unavailable/zsh".into(),
            clean_start: true,
        });
        assert_eq!(
            ResolvedTerminalNativeLaunch::resolve(&config, &request)
                .unwrap()
                .shell()
                .profile(),
            TerminalProfile::Clean
        );
        request.profile = Some(TerminalProfile::User);
        assert!(matches!(
            ResolvedTerminalNativeLaunch::resolve(&config, &request),
            Err(TerminalNativeLaunchError::Invalid)
        ));
        request.profile = None;
        request.backend = TerminalBackend::Tmux;
        config.tmux = None;
        assert!(matches!(
            ResolvedTerminalNativeLaunch::resolve(&config, &request),
            Err(TerminalNativeLaunchError::UnavailableBackend)
        ));
        request.backend = TerminalBackend::Native;
        request.command = Some("x".repeat(machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES + 1));
        request.return_when = Some(TerminalReturnCondition::Started);
        assert!(ResolvedTerminalNativeLaunch::resolve(&config, &request).is_err());
    }

    #[test]
    fn invalid_authority_cancelled_and_unbounded_deadlines_have_no_launch_artifacts() {
        let config = config();
        let cwd = Directory::new();
        let artifacts = Directory::new();
        let other = Directory::new();
        for backend in [TerminalBackend::Native, TerminalBackend::Tmux] {
            if backend == TerminalBackend::Tmux && config.tmux.is_none() {
                continue;
            }
            let request = request(&cwd, backend, Some("touch executed".into()));
            for kind in 0..5 {
                let mut authority = authority(&cwd, &artifacts);
                let cancellation = CancellationToken::new();
                let mut deadline = Instant::now() + Duration::from_secs(10);
                match kind {
                    0 => authority.cwd = other.fd(),
                    1 => authority
                        .environment
                        .push(("BAD=KEY".into(), "value".into())),
                    2 => {
                        cancellation.cancel();
                    }
                    3 => deadline = Instant::now(),
                    _ => deadline = Instant::now() + MAX_STARTUP_TIMEOUT + Duration::from_secs(1),
                }
                let result = ResolvedTerminalNativeLaunch::resolve(&config, &request)
                    .unwrap()
                    .prepare(&config, authority, deadline, &cancellation);
                assert!(result.is_err());
                assert!(cwd.empty() && artifacts.empty());
            }
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "Real backend/shell/profile/start matrix shares one gate and cleanup driver."
    )]
    fn real_factory_preserves_both_backends_profiles_defaults_and_startup_gates() {
        let mut config = config();
        for backend_kind in [TerminalBackend::Native, TerminalBackend::Tmux] {
            if backend_kind == TerminalBackend::Tmux && config.tmux.is_none() {
                continue;
            }
            for shell in ["/bin/bash", "/bin/zsh"] {
                if !Path::new(shell).is_file() {
                    continue;
                }
                config.account_shell = Some(shell.into());
                for clean in [false, true] {
                    for commandless in [false, true] {
                        let cwd = Directory::new();
                        let artifacts = Directory::new();
                        let profile = if shell.ends_with("bash") {
                            ".bash_profile"
                        } else {
                            ".zprofile"
                        };
                        std::fs::write(cwd.0.join(profile), "export FROM_PROFILE=user; printf PROFILE_OUTPUT; for fd in 3 4 5 6 7 8 9; do eval \"exec $fd>&-\"; done\n").unwrap();
                        let source = format!(
                            "test \"${{FROM_PROFILE-unset}}\" = {} || exit 7; stty size; printf done > executed; exit 23",
                            if clean { "unset" } else { "user" }
                        );
                        let mut request =
                            request(&cwd, backend_kind, (!commandless).then_some(source.clone()));
                        request.profile = Some(if clean {
                            TerminalProfile::Clean
                        } else {
                            TerminalProfile::User
                        });
                        if !commandless {
                            request.dimensions = Some(TerminalDimensions::new(37, 99).unwrap());
                        }
                        let deadline = Instant::now() + Duration::from_secs(15);
                        let prepared = prepare(&config, &request, &cwd, &artifacts, deadline);
                        assert_eq!(prepared.identity().command, request.command);
                        let expected_identity = prepared.identity().backend_identity.clone();
                        let (mut backend, mut control, identity) =
                            prepared.commit(&CancellationToken::new()).unwrap();
                        assert_eq!(identity.backend, backend_kind);
                        assert_eq!(identity.backend_identity, expected_identity);
                        assert_eq!(identity.shell, shell);
                        assert_eq!(identity.profile, request.profile.unwrap());
                        assert_eq!(identity.cwd, request.cwd);
                        assert_eq!(
                            identity.dimensions.rows(),
                            if commandless { 24 } else { 37 }
                        );
                        assert!(!cwd.0.join("executed").exists());
                        assert_eq!(backend.write(b"unreleased\n").unwrap().bytes_written(), 0);
                        let mut output = Vec::new();
                        event(
                            &mut backend,
                            &mut control,
                            TerminalStartupEvent::ShellReady,
                            &mut output,
                            deadline,
                        );
                        shell_ack(&mut backend, &mut control);
                        if commandless {
                            send(
                                &mut backend,
                                format!("{source}\n").as_bytes(),
                                false,
                                deadline,
                            );
                        } else {
                            event(
                                &mut backend,
                                &mut control,
                                TerminalStartupEvent::CommandStarted,
                                &mut output,
                                deadline,
                            );
                            assert!(!cwd.0.join("executed").exists());
                            assert!(
                                control
                                    .release_command(Instant::now(), &CancellationToken::new())
                                    .unwrap()
                            );
                        }
                        finish(&mut backend, &mut output, 23, deadline);
                        control.retry_cleanup().unwrap();
                        assert!(artifacts.empty());
                        assert_eq!(
                            std::fs::read_to_string(cwd.0.join("executed")).unwrap(),
                            "done"
                        );
                        let output = String::from_utf8_lossy(&output);
                        assert_eq!(output.contains("PROFILE_OUTPUT"), !clean);
                        assert!(output.contains(if commandless { "24 80" } else { "37 99" }));
                    }
                }
            }
        }
    }

    #[test]
    fn real_factory_retains_full_command_and_large_paste() {
        let config = config();
        for backend_kind in [TerminalBackend::Native, TerminalBackend::Tmux] {
            if backend_kind == TerminalBackend::Tmux && config.tmux.is_none() {
                continue;
            }
            let cwd = Directory::new();
            let artifacts = Directory::new();
            let mut command = String::from(
                "stty raw -echo; printf PASTE_READY; /usr/bin/head -c 65536 > received; exit 23; #",
            );
            command.push_str(
                &"x".repeat(machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES - command.len()),
            );
            let request = request(&cwd, backend_kind, Some(command));
            let deadline = Instant::now() + Duration::from_secs(15);
            let prepared = prepare(&config, &request, &cwd, &artifacts, deadline);
            assert_eq!(prepared.identity().command.as_ref().unwrap().len(), 65_536);
            let (mut backend, mut control, _) = prepared.commit(&CancellationToken::new()).unwrap();
            let mut output = Vec::new();
            event(
                &mut backend,
                &mut control,
                TerminalStartupEvent::ShellReady,
                &mut output,
                deadline,
            );
            shell_ack(&mut backend, &mut control);
            event(
                &mut backend,
                &mut control,
                TerminalStartupEvent::CommandStarted,
                &mut output,
                deadline,
            );
            assert!(
                control
                    .release_command(Instant::now(), &CancellationToken::new())
                    .unwrap()
            );
            while !output.windows(11).any(|bytes| bytes == b"PASTE_READY") {
                read(&mut backend, &mut output);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let paste = vec![b'x'; 65_536];
            send(&mut backend, &paste, true, deadline);
            finish(&mut backend, &mut output, 23, deadline);
            control.retry_cleanup().unwrap();
            assert_eq!(std::fs::read(cwd.0.join("received")).unwrap(), paste);
            assert!(artifacts.empty());
        }
    }

    #[test]
    fn real_factory_rechecks_exact_cwd_before_commit() {
        let config = config();
        for backend_kind in [TerminalBackend::Native, TerminalBackend::Tmux] {
            if backend_kind == TerminalBackend::Tmux && config.tmux.is_none() {
                continue;
            }
            let cwd = Directory::new();
            let artifacts = Directory::new();
            let parked = Directory::new();
            let request = request(&cwd, backend_kind, Some("touch executed".into()));
            let prepared = prepare(
                &config,
                &request,
                &cwd,
                &artifacts,
                Instant::now() + Duration::from_secs(10),
            );
            std::fs::rename(&cwd.0, &parked.0).unwrap();
            std::fs::create_dir(&cwd.0).unwrap();
            assert!(matches!(
                prepared.commit(&CancellationToken::new()),
                Err(TerminalNativeLaunchError::Invalid)
            ));
            assert!(cwd.empty() && parked.empty() && artifacts.empty());
        }
    }

    #[test]
    fn real_factory_late_cancel_and_deadline_do_not_extend_or_release_command() {
        let config = config();
        for backend_kind in [TerminalBackend::Native, TerminalBackend::Tmux] {
            if backend_kind == TerminalBackend::Tmux && config.tmux.is_none() {
                continue;
            }
            for expire in [false, true] {
                let cwd = Directory::new();
                let artifacts = Directory::new();
                let request = request(&cwd, backend_kind, Some("touch executed".into()));
                let deadline = Instant::now() + Duration::from_secs(2);
                let prepared = prepare(&config, &request, &cwd, &artifacts, deadline);
                let cancellation = CancellationToken::new();
                if expire {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                } else {
                    cancellation.cancel();
                }
                let result = prepared.commit(&cancellation);
                assert!(matches!(
                    (result, expire),
                    (Err(TerminalNativeLaunchError::Cancelled), false)
                        | (Err(TerminalNativeLaunchError::Timeout), true)
                ));
                assert!(cwd.empty() && artifacts.empty());
            }
            let cwd = Directory::new();
            let artifacts = Directory::new();
            let request = request(&cwd, backend_kind, Some("touch executed".into()));
            let deadline = Instant::now() + Duration::from_secs(2);
            let prepared = prepare(&config, &request, &cwd, &artifacts, deadline);
            let (mut backend, mut control, _) = prepared.commit(&CancellationToken::new()).unwrap();
            let mut output = Vec::new();
            event(
                &mut backend,
                &mut control,
                TerminalStartupEvent::ShellReady,
                &mut output,
                deadline,
            );
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            assert_eq!(
                control.acknowledge_shell_ready(Instant::now(), &CancellationToken::new()),
                Err(TerminalStartupError::Timeout)
            );
            backend.close(true, &mut |_| {}).unwrap();
            control.retry_cleanup().unwrap();
            assert!(cwd.empty() && artifacts.empty());
        }
    }
}
