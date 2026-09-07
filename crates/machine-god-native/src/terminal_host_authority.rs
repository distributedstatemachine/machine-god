//! Frozen host selection and worker-only descriptor resolution for terminal actions.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::TERMINAL_CAPTURED_HELPER_ARGUMENT;
use crate::background_process::ValidatedBackgroundEnvironment;
use crate::terminal_action_tool::{TerminalActionHostIdentity, TerminalActionInvocation};
use crate::terminal_helper::{
    TERMINAL_PTY_HELPER_ARGUMENT, TERMINAL_STARTUP_MARKER_ARGUMENT, TerminalPtyHelper,
    validate_startup_directory,
};
use crate::terminal_native_launch::{
    TerminalNativeLaunchAuthority, TerminalNativeLaunchConfig, TerminalNativeTmuxConfig,
};
use crate::terminal_shell::TerminalShell;
use crate::terminal_tmux_helper::TERMINAL_TMUX_HELPER_ARGUMENT;
use machine_god_core::{
    CancellationToken, MAX_TERMINAL_ACTION_TEXT_BYTES, TerminalActionRequest, TerminalExecRequest,
    ToolError, ToolErrorKind,
};
use rustix::fd::OwnedFd;
use rustix::fs::{FileType, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const MAX_PATH_BYTES: usize = MAX_TERMINAL_ACTION_TEXT_BYTES;

/// Selection is explicit; `CurrentUser` performs its one lookup only on the worker.
pub(crate) enum TerminalHostAccountShell {
    Explicit(Option<PathBuf>),
    #[cfg(test)]
    CurrentUser,
}

/// All authority is supplied by the host. Paths are binding/display data, not
/// substitutes for the already-opened workspace and private artifact descriptors.
pub(crate) struct TerminalHostAuthorityInputs {
    pub(crate) workspace: OwnedFd,
    pub(crate) workspace_path: PathBuf,
    pub(crate) default_cwd: PathBuf,
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) account_shell: TerminalHostAccountShell,
    pub(crate) cli_executable: PathBuf,
    pub(crate) tmux_executable: Option<PathBuf>,
    pub(crate) artifacts: OwnedFd,
    pub(crate) artifact_path: PathBuf,
}

/// Inert pending capture. Construction validates bounded data but performs no
/// stat/open/dup, account lookup, environment discovery or helper execution.
pub(crate) struct TerminalHostAuthority {
    inputs: TerminalHostAuthorityInputs,
    environment: ValidatedBackgroundEnvironment,
}

/// Immutable captured host inputs. Only worker methods allocate descriptors.
pub(crate) struct CapturedTerminalHostAuthority {
    workspace: OwnedFd,
    workspace_path: PathBuf,
    default_cwd: PathBuf,
    artifacts: OwnedFd,
    artifact_path: PathBuf,
    environment: ValidatedBackgroundEnvironment,
    launch: Arc<TerminalNativeLaunchConfig>,
    identity: TerminalActionHostIdentity,
}

/// Command requests carry their exact acquired cwd. This value stays on the
/// owned effect worker through launch; it is not an async poll-thread response.
/// Non-command actions carry no descriptor. A supplied list workspace filter
/// performs worker-only path resolution but never acquires command authority.
pub(crate) struct ResolvedTerminalHostInvocation {
    pub(crate) request: TerminalActionRequest,
    pub(crate) cwd: Option<OwnedFd>,
}

impl TerminalHostAuthority {
    pub(crate) fn new(mut inputs: TerminalHostAuthorityInputs) -> Result<Self, ToolError> {
        for path in [
            &inputs.workspace_path,
            &inputs.default_cwd,
            &inputs.artifact_path,
            &inputs.cli_executable,
        ] {
            path_text(path)?;
        }
        if let Some(path) = &inputs.tmux_executable {
            path_text(path)?;
        }
        if let TerminalHostAccountShell::Explicit(Some(path)) = &inputs.account_shell {
            TerminalShell::from_account_shell(Some(path), None, None).map_err(|_| invalid())?;
        }
        let environment =
            ValidatedBackgroundEnvironment::new(std::mem::take(&mut inputs.environment))
                .map_err(|_| invalid())?;
        Ok(Self {
            inputs,
            environment,
        })
    }

    /// Owned blocking worker only. One caller deadline covers every boundary;
    /// filesystem/name-service syscalls themselves are indivisible, not preempted.
    pub(crate) fn capture_on_worker(
        self,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<CapturedTerminalHostAuthority, ToolError> {
        check(deadline, cancellation)?;
        let inputs = self.inputs;
        exact_directory(&inputs.workspace, &inputs.workspace_path)?;
        check(deadline, cancellation)?;
        exact_directory(&inputs.artifacts, &inputs.artifact_path)?;
        validate_startup_directory(&inputs.artifacts, &inputs.artifact_path)
            .map_err(|_| unavailable())?;
        check(deadline, cancellation)?;
        let mut selection = Sha256::new();
        hash_field(&mut selection, b"machine-god-terminal-shell-selection-v1");
        hash_field(&mut selection, std::env::consts::OS.as_bytes());
        let account_shell = match inputs.account_shell {
            TerminalHostAccountShell::Explicit(path) => {
                hash_field(&mut selection, b"explicit-account");
                path
            }
            #[cfg(test)]
            TerminalHostAccountShell::CurrentUser => {
                hash_field(&mut selection, b"current-user-account");
                Some(
                    TerminalShell::for_current_user(None, None)
                        .map_err(|_| unavailable())?
                        .program()
                        .to_owned(),
                )
            }
        };
        check(deadline, cancellation)?;
        hash_field(
            &mut selection,
            account_shell
                .as_deref()
                .map_or(b"", |p| p.as_os_str().as_bytes()),
        );
        if let Some(path) = &account_shell {
            let shell =
                TerminalShell::from_account_shell(Some(path), None, None).map_err(|_| invalid())?;
            hash_field(&mut selection, shell.program().as_os_str().as_bytes());
        }
        #[cfg(target_os = "macos")]
        let inventory = crate::process_inventory_helper::ProcessInventoryHelper::new_service(
            inputs.cli_executable.clone(),
            vec![crate::PROCESS_INVENTORY_SERVICE_ARGUMENT.into()],
        )
        .map_err(|_| invalid())?;
        let helper = |flag: &str| {
            let helper = configured_helper(&inputs.cli_executable, flag)?;
            #[cfg(target_os = "macos")]
            let helper = helper.with_inventory_helper(inventory.clone());
            Ok::<_, ToolError>(helper)
        };
        #[cfg(target_os = "macos")]
        hash_field(
            &mut selection,
            crate::PROCESS_INVENTORY_SERVICE_ARGUMENT.as_bytes(),
        );
        hash_field(&mut selection, inputs.cli_executable.as_os_str().as_bytes());
        for flag in [
            TERMINAL_PTY_HELPER_ARGUMENT,
            TERMINAL_STARTUP_MARKER_ARGUMENT,
            TERMINAL_CAPTURED_HELPER_ARGUMENT,
        ] {
            hash_field(&mut selection, flag.as_bytes());
        }
        hash_field(
            &mut selection,
            inputs
                .tmux_executable
                .as_deref()
                .map_or(b"", |p| p.as_os_str().as_bytes()),
        );
        hash_field(&mut selection, TERMINAL_TMUX_HELPER_ARGUMENT.as_bytes());
        let launch = TerminalNativeLaunchConfig {
            account_shell,
            pty_helper: helper(TERMINAL_PTY_HELPER_ARGUMENT)?,
            marker_helper: helper(TERMINAL_STARTUP_MARKER_ARGUMENT)?,
            tmux: inputs
                .tmux_executable
                .clone()
                .map(|executable| {
                    Ok(TerminalNativeTmuxConfig {
                        executable,
                        pane_helper: helper(TERMINAL_TMUX_HELPER_ARGUMENT)?,
                        capture_helper: helper(TERMINAL_TMUX_HELPER_ARGUMENT)?,
                    })
                })
                .transpose()?,
        };
        let identity = TerminalActionHostIdentity {
            workspace: path_text(&inputs.workspace_path)?.into(),
            default_cwd: path_text(&inputs.default_cwd)?.into(),
            environment_sha256: environment_hash(&self.environment),
            shell_selection_sha256: format!("{:x}", selection.finalize()),
        };
        let authority = CapturedTerminalHostAuthority {
            workspace: inputs.workspace,
            workspace_path: inputs.workspace_path,
            default_cwd: inputs.default_cwd,
            artifacts: inputs.artifacts,
            artifact_path: inputs.artifact_path,
            environment: self.environment,
            launch: Arc::new(launch),
            identity,
        };
        // Validate the configured default with the same native resolver used by
        // actions; do not retain a descriptor to reopen by path during launch.
        let (canonical, _) = authority.resolve_directory(".", deadline, cancellation)?;
        if canonical != authority.identity.default_cwd {
            return Err(invalid());
        }
        check(deadline, cancellation)?;
        Ok(authority)
    }
}

impl CapturedTerminalHostAuthority {
    pub(crate) const fn identity(&self) -> &TerminalActionHostIdentity {
        &self.identity
    }

    pub(crate) fn launch_config(&self) -> Arc<TerminalNativeLaunchConfig> {
        Arc::clone(&self.launch)
    }

    pub(crate) fn exec_shell(
        &self,
        request: &TerminalExecRequest,
    ) -> Result<TerminalShell, ToolError> {
        request.validate().map_err(|_| invalid())?;
        TerminalShell::from_account_shell(
            self.launch.account_shell.as_deref(),
            request.profile,
            None,
        )
        .map_err(|_| invalid())
    }

    pub(crate) fn environment_on_worker(&self) -> Vec<(OsString, OsString)> {
        self.environment.entries().to_vec()
    }

    pub(crate) fn resolve_on_worker(
        &self,
        invocation: TerminalActionInvocation,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedTerminalHostInvocation, ToolError> {
        check(deadline, cancellation)?;
        let invocation = invocation.resolve_workspace_filter(|raw| {
            self.resolve_workspace_filter(raw, deadline, cancellation)
        })?;
        let mut cwd = None;
        let request = invocation.resolve_cwd(|raw| {
            let (canonical, descriptor) = self.resolve_directory(raw, deadline, cancellation)?;
            cwd = Some(descriptor);
            Ok(canonical)
        })?;
        check(deadline, cancellation)?;
        Ok(ResolvedTerminalHostInvocation { request, cwd })
    }

    fn resolve_workspace_filter(
        &self,
        raw: &str,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<String, ToolError> {
        check(deadline, cancellation)?;
        exact_directory(&self.workspace, &self.workspace_path)?;
        let cleaned = raw.trim_matches([' ', '\t', '\r', '\n']);
        if cleaned.is_empty() || raw.len() > MAX_PATH_BYTES || raw.contains('\0') {
            return Err(invalid());
        }
        let original = if cleaned == "~" || cleaned.starts_with("~/") {
            let home = self
                .environment
                .entries()
                .iter()
                .find(|(name, _)| name == "HOME")
                .map(|(_, value)| Path::new(value))
                .filter(|home| home.is_absolute())
                .ok_or_else(invalid)?;
            home.join(
                cleaned
                    .strip_prefix("~/")
                    .unwrap_or("")
                    .trim_start_matches('/'),
            )
        } else {
            if cleaned.starts_with('~') {
                return Err(invalid());
            }
            self.default_cwd.join(cleaned)
        };
        // Lexical intent controls only whether an explicitly foreign predicate
        // is allowed. Never use this spelling for filesystem resolution: doing
        // so would change symlink/parent semantics.
        let mut lexical = PathBuf::new();
        for component in original.components() {
            match component {
                Component::ParentDir => {
                    lexical.pop();
                }
                Component::CurDir => {}
                component => lexical.push(component.as_os_str()),
            }
        }
        let external = cleaned.starts_with(['/', '~']) || !lexical.starts_with(&self.default_cwd);
        check(deadline, cancellation)?;
        let canonical = std::fs::canonicalize(original).map_err(|_| unavailable())?;
        check(deadline, cancellation)?;
        if !external && !canonical.starts_with(&self.default_cwd) {
            return Err(invalid());
        }
        exact_directory(&self.workspace, &self.workspace_path)?;
        check(deadline, cancellation)?;
        Ok(path_text(&canonical)?.to_owned())
    }

    /// Called after start admission on the owned effect worker. The supplied cwd
    /// is transferred unchanged; the artifact directory is duplicated, never reopened.
    pub(crate) fn launch_authority_on_worker(
        &self,
        cwd: OwnedFd,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<TerminalNativeLaunchAuthority, ToolError> {
        check(deadline, cancellation)?;
        let artifacts =
            rustix::io::fcntl_dupfd_cloexec(&self.artifacts, 3).map_err(|_| unavailable())?;
        check(deadline, cancellation)?;
        let authority = TerminalNativeLaunchAuthority {
            environment: self.environment_on_worker(),
            cwd,
            artifacts,
            artifact_path: self.artifact_path.clone(),
        };
        check(deadline, cancellation)?;
        Ok(authority)
    }

    pub(crate) fn resolve_directory(
        &self,
        raw: &str,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<(String, OwnedFd), ToolError> {
        check(deadline, cancellation)?;
        if raw.is_empty() || raw.len() > MAX_PATH_BYTES || raw.contains('\0') {
            return Err(invalid());
        }
        exact_directory(&self.workspace, &self.workspace_path)?;
        check(deadline, cancellation)?;
        // join preserves symlink/.. order. Never collect lexical components first.
        let original = self.default_cwd.join(raw);
        let canonical = std::fs::canonicalize(&original).map_err(|_| unavailable())?;
        check(deadline, cancellation)?;
        let relative = canonical
            .strip_prefix(&self.workspace_path)
            .map_err(|_| invalid())?;
        let canonical_text = path_text(&canonical)?.to_owned();
        let original_fd = rustix::fs::open(&original, directory_flags(), Mode::empty())
            .map_err(|_| unavailable())?;
        check(deadline, cancellation)?;
        let mut retained =
            rustix::io::fcntl_dupfd_cloexec(&self.workspace, 3).map_err(|_| unavailable())?;
        for component in relative.components() {
            check(deadline, cancellation)?;
            let Component::Normal(component) = component else {
                return Err(invalid());
            };
            retained = rustix::fs::openat(
                &retained,
                component,
                directory_flags() | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|_| unavailable())?;
        }
        check(deadline, cancellation)?;
        same_directory(&original_fd, &retained)?;
        exact_directory(&self.workspace, &self.workspace_path)?;
        check(deadline, cancellation)?;
        Ok((canonical_text, retained))
    }
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NONBLOCK
}

fn exact_directory(descriptor: &OwnedFd, path: &Path) -> Result<(), ToolError> {
    if std::fs::canonicalize(path)
        .map_err(|_| unavailable())?
        .as_os_str()
        != path.as_os_str()
    {
        return Err(invalid());
    }
    let named = rustix::fs::stat(path).map_err(|_| unavailable())?;
    let retained = rustix::fs::fstat(descriptor).map_err(|_| unavailable())?;
    compare_directory(&named, &retained)
}

fn same_directory(left: &OwnedFd, right: &OwnedFd) -> Result<(), ToolError> {
    compare_directory(
        &rustix::fs::fstat(left).map_err(|_| unavailable())?,
        &rustix::fs::fstat(right).map_err(|_| unavailable())?,
    )
}

fn compare_directory(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> Result<(), ToolError> {
    if left.st_dev != right.st_dev
        || left.st_ino != right.st_ino
        || left.st_nlink == 0
        || right.st_nlink == 0
        || !FileType::from_raw_mode(left.st_mode).is_dir()
        || !FileType::from_raw_mode(right.st_mode).is_dir()
    {
        return Err(unavailable());
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<&str, ToolError> {
    let text = path.to_str().ok_or_else(invalid)?;
    if !path.is_absolute() || text.len() > MAX_PATH_BYTES || text.contains('\0') {
        return Err(invalid());
    }
    Ok(text)
}

fn environment_hash(environment: &ValidatedBackgroundEnvironment) -> String {
    let mut digest = Sha256::new();
    hash_field(&mut digest, b"machine-god-terminal-environment-v1");
    // Ordering is retained exactly as launched, including non-UTF-8 bytes.
    for (key, value) in environment.entries() {
        hash_field(&mut digest, key.as_os_str().as_bytes());
        hash_field(&mut digest, value.as_os_str().as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn hash_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn check(deadline: Instant, cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        return Err(ToolError::new(
            ToolErrorKind::Cancelled,
            "terminal_cancelled",
            "terminal authority cancelled",
            false,
        ));
    }
    if Instant::now() >= deadline {
        return Err(ToolError::new(
            ToolErrorKind::Execution,
            "terminal_authority_timeout",
            "terminal authority deadline elapsed",
            false,
        ));
    }
    Ok(())
}

fn configured_helper(program: &Path, flag: &str) -> Result<TerminalPtyHelper, ToolError> {
    let helper =
        TerminalPtyHelper::new(program.to_owned(), vec![flag.into()]).map_err(|_| invalid())?;
    Ok(helper)
}

fn invalid() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "terminal_invalid_authority",
        "invalid terminal host authority",
        false,
    )
}
fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "terminal_authority_unavailable",
        "terminal host authority unavailable",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{TerminalProfile, TerminalShellSpec, TerminalStartRequest};
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::Duration;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-authority-{}-{:032x}",
                std::process::id(),
                u128::from_le_bytes(nonce)
            ));
            std::fs::create_dir(&path).unwrap();
            let path = std::fs::canonicalize(path).unwrap();
            for child in ["workspace", "artifacts", "outside"] {
                std::fs::create_dir(path.join(child)).unwrap();
                std::fs::set_permissions(path.join(child), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
            }
            Self(path)
        }

        fn inputs(&self) -> TerminalHostAuthorityInputs {
            let workspace_path = self.0.join("workspace");
            let artifact_path = self.0.join("artifacts");
            TerminalHostAuthorityInputs {
                workspace: open(&workspace_path),
                default_cwd: workspace_path.clone(),
                workspace_path,
                environment: vec![("PATH".into(), "/bin:/usr/bin".into())],
                account_shell: TerminalHostAccountShell::Explicit(Some("/bin/bash".into())),
                cli_executable: "/explicit/machine-god".into(),
                tmux_executable: Some("/explicit/tmux".into()),
                artifacts: open(&artifact_path),
                artifact_path,
            }
        }

        fn authority(&self) -> CapturedTerminalHostAuthority {
            capture(self.inputs())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn open(path: &Path) -> OwnedFd {
        rustix::fs::open(path, directory_flags(), Mode::empty()).unwrap()
    }
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(10)
    }
    fn capture(inputs: TerminalHostAuthorityInputs) -> CapturedTerminalHostAuthority {
        TerminalHostAuthority::new(inputs)
            .unwrap()
            .capture_on_worker(deadline(), &CancellationToken::new())
            .unwrap()
    }
    fn invocation(raw: &str, start: bool) -> TerminalActionInvocation {
        let draft = if start {
            TerminalActionRequest::Start {
                request: TerminalStartRequest::interactive("/").unwrap(),
            }
        } else {
            TerminalActionRequest::Exec {
                request: TerminalExecRequest {
                    command: "printf hello".into(),
                    cwd: "/".into(),
                    profile: None,
                },
            }
        };
        serde_json::from_value(serde_json::json!({"draft": draft, "requested_cwd": raw})).unwrap()
    }
    fn resolve(
        authority: &CapturedTerminalHostAuthority,
        raw: &str,
        start: bool,
    ) -> ResolvedTerminalHostInvocation {
        authority
            .resolve_on_worker(
                invocation(raw, start),
                deadline(),
                &CancellationToken::new(),
            )
            .unwrap()
    }

    #[test]
    fn inert_configuration_rejects_bad_data_without_validating_native_paths() {
        let fixture = Fixture::new();
        let mut inputs = fixture.inputs();
        inputs.workspace_path = "/does-not-exist/retained-workspace".into();
        inputs.default_cwd = inputs.workspace_path.clone();
        inputs.account_shell = TerminalHostAccountShell::CurrentUser;
        let pending = TerminalHostAuthority::new(inputs).unwrap();
        assert!(
            pending
                .capture_on_worker(deadline(), &CancellationToken::new())
                .is_err()
        );
        for mode in 0..4 {
            let mut inputs = fixture.inputs();
            match mode {
                0 => inputs.cli_executable = "relative".into(),
                1 => inputs.environment.push(("PATH".into(), "duplicate".into())),
                2 => inputs.environment = vec![("bad=key".into(), "secret".into())],
                _ => {
                    inputs.account_shell =
                        TerminalHostAccountShell::Explicit(Some("relative/bash".into()));
                }
            }
            assert!(TerminalHostAuthority::new(inputs).is_err());
        }
    }

    #[test]
    fn native_symlink_parent_and_absolute_paths_resolve_to_exact_owned_directories() {
        let fixture = Fixture::new();
        let root = fixture.0.join("workspace");
        std::fs::create_dir_all(root.join("real/deep")).unwrap();
        std::fs::create_dir(root.join("default")).unwrap();
        symlink("../real/deep", root.join("default/link")).unwrap();
        let mut inputs = fixture.inputs();
        inputs.default_cwd = root.join("default");
        let authority = capture(inputs);
        for start in [false, true] {
            for raw in [
                "link/..".to_owned(),
                root.join("real").to_str().unwrap().to_owned(),
            ] {
                let resolved = resolve(&authority, &raw, start);
                let cwd = resolved.cwd.unwrap();
                same_directory(&cwd, &open(&root.join("real"))).unwrap();
                let canonical = match resolved.request {
                    TerminalActionRequest::Exec { request } => request.cwd,
                    TerminalActionRequest::Start { request } => request.cwd,
                    _ => panic!(),
                };
                assert_eq!(canonical, root.join("real").to_str().unwrap());
            }
        }
        same_directory(
            &resolve(&authority, ".", true).cwd.unwrap(),
            &open(&root.join("default")),
        )
        .unwrap();
        assert_eq!(
            authority.identity().default_cwd,
            root.join("default").to_str().unwrap()
        );
    }

    #[test]
    fn containment_rejects_outside_symlinks_siblings_files_loops_and_missing_components() {
        let fixture = Fixture::new();
        let root = fixture.0.join("workspace");
        symlink("../outside", root.join("escape")).unwrap();
        symlink("loop", root.join("loop")).unwrap();
        std::fs::write(root.join("file"), b"not a directory").unwrap();
        let authority = fixture.authority();
        for raw in [
            "..",
            "../outside",
            "escape",
            "escape/..",
            "missing/..",
            "loop",
            "file",
        ] {
            assert!(
                authority
                    .resolve_on_worker(invocation(raw, true), deadline(), &CancellationToken::new())
                    .is_err(),
                "{raw}"
            );
        }
        let mut inputs = fixture.inputs();
        inputs.default_cwd = fixture.0.join("outside");
        assert!(
            TerminalHostAuthority::new(inputs)
                .unwrap()
                .capture_on_worker(deadline(), &CancellationToken::new())
                .is_err()
        );
    }

    #[test]
    fn workspace_identity_and_artifact_identity_mode_are_checked() {
        let fixture = Fixture::new();
        for mode in 0..4 {
            let mut inputs = fixture.inputs();
            match mode {
                0 => inputs.workspace = open(&fixture.0.join("outside")),
                1 => inputs.artifacts = open(&fixture.0.join("outside")),
                2 => inputs.workspace_path.push("."),
                _ => std::fs::set_permissions(
                    &inputs.artifact_path,
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap(),
            }
            assert!(
                TerminalHostAuthority::new(inputs)
                    .unwrap()
                    .capture_on_worker(deadline(), &CancellationToken::new())
                    .is_err()
            );
        }
    }

    #[test]
    fn retained_command_and_artifact_descriptors_are_never_reopened_for_handoff() {
        let fixture = Fixture::new();
        let root = fixture.0.join("workspace");
        std::fs::create_dir(root.join("selected")).unwrap();
        let authority = fixture.authority();
        let resolved = resolve(&authority, "selected", true);
        std::fs::rename(root.join("selected"), root.join("moved")).unwrap();
        std::fs::create_dir(root.join("selected")).unwrap();
        let artifact = fixture.0.join("artifacts");
        std::fs::rename(&artifact, fixture.0.join("moved-artifacts")).unwrap();
        std::fs::create_dir(&artifact).unwrap();
        let launch = authority
            .launch_authority_on_worker(
                resolved.cwd.unwrap(),
                deadline(),
                &CancellationToken::new(),
            )
            .unwrap();
        same_directory(&launch.cwd, &open(&root.join("moved"))).unwrap();
        same_directory(&launch.artifacts, &open(&fixture.0.join("moved-artifacts"))).unwrap();
        assert!(same_directory(&launch.cwd, &open(&root.join("selected"))).is_err());
        assert!(same_directory(&launch.artifacts, &open(&artifact)).is_err());
    }

    #[test]
    fn workspace_replacement_fails_commands_but_noncommand_resolution_does_no_io() {
        let fixture = Fixture::new();
        let authority = fixture.authority();
        std::fs::rename(fixture.0.join("workspace"), fixture.0.join("old-workspace")).unwrap();
        std::fs::create_dir(fixture.0.join("workspace")).unwrap();
        assert!(
            authority
                .resolve_on_worker(
                    invocation(".", false),
                    deadline(),
                    &CancellationToken::new()
                )
                .is_err()
        );
        let draft = crate::terminal_action_parse::decode_terminal_action(
            &serde_json::json!({"action":"list"}),
            "/",
        )
        .unwrap();
        let invocation =
            serde_json::from_value(serde_json::json!({"draft": draft, "requested_cwd": null}))
                .unwrap();
        let resolved = authority
            .resolve_on_worker(invocation, deadline(), &CancellationToken::new())
            .unwrap();
        assert!(resolved.cwd.is_none());
    }

    #[test]
    fn workspace_filter_resolution_keeps_paths_non_authoritative_and_checks_deadlines() {
        let fixture = Fixture::new();
        let authority = fixture.authority();
        let root = fixture.0.join("workspace");
        symlink("../outside", root.join("escape")).unwrap();
        let invoke = |raw: &str| {
            let draft = crate::terminal_action_parse::decode_terminal_action(
                &serde_json::json!({"action":"list","workspace_root":raw}),
                "/",
            )
            .unwrap();
            serde_json::from_value(serde_json::json!({"draft":draft,"requested_cwd":null})).unwrap()
        };
        assert!(
            authority
                .resolve_on_worker(invoke("escape"), deadline(), &CancellationToken::new())
                .is_err()
        );
        let outside = fixture.0.join("outside");
        let resolved = authority
            .resolve_on_worker(
                invoke(outside.to_str().unwrap()),
                deadline(),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(resolved.cwd.is_none());
        assert!(
            matches!(resolved.request, TerminalActionRequest::List { filters } if filters.workspace_root.as_deref() == outside.to_str())
        );
        assert!(
            authority
                .resolve_on_worker(invoke("."), Instant::now(), &CancellationToken::new())
                .is_err()
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            authority
                .resolve_on_worker(invoke("."), deadline(), &cancelled)
                .is_err()
        );
        std::fs::rename(&root, fixture.0.join("old-workspace")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(
            authority
                .resolve_on_worker(invoke("."), deadline(), &CancellationToken::new())
                .is_err()
        );
    }

    #[test]
    fn fingerprints_bind_exact_nonutf8_environment_shell_policy_and_helpers() {
        let fixture = Fixture::new();
        let baseline = fixture.authority();
        assert!(Arc::ptr_eq(
            &baseline.launch_config(),
            &baseline.launch_config()
        ));
        for mode in 0..6 {
            let mut inputs = fixture.inputs();
            match mode {
                0 => inputs.environment[0].1 = OsString::from_vec(vec![0xff, 0xfe]),
                1 => {
                    inputs.account_shell =
                        TerminalHostAccountShell::Explicit(Some("/bin/zsh".into()));
                }
                2 => inputs.account_shell = TerminalHostAccountShell::Explicit(None),
                3 => inputs.cli_executable = "/different/machine-god".into(),
                4 => inputs.tmux_executable = None,
                _ => inputs.tmux_executable = Some("/different/tmux".into()),
            }
            let other = capture(inputs);
            if mode == 0 {
                assert_ne!(
                    baseline.identity.environment_sha256,
                    other.identity.environment_sha256
                );
                assert_eq!(
                    other.environment_on_worker()[0].1.as_os_str().as_bytes(),
                    &[0xff, 0xfe]
                );
            } else {
                assert_ne!(
                    baseline.identity.shell_selection_sha256,
                    other.identity.shell_selection_sha256
                );
            }
        }
        let left = ValidatedBackgroundEnvironment::new(vec![("a".into(), "bc".into())]).unwrap();
        let right = ValidatedBackgroundEnvironment::new(vec![("ab".into(), "c".into())]).unwrap();
        assert_ne!(environment_hash(&left), environment_hash(&right));
        let entries = vec![("A".into(), "1".into()), ("B".into(), "2".into())];
        let ordered = ValidatedBackgroundEnvironment::new(entries.clone()).unwrap();
        let reversed =
            ValidatedBackgroundEnvironment::new(entries.into_iter().rev().collect()).unwrap();
        assert_ne!(environment_hash(&ordered), environment_hash(&reversed));
        assert_eq!(baseline.identity, fixture.authority().identity);
        assert!(!format!("{:?}", baseline.identity()).contains("workspace"));
    }

    #[test]
    fn frozen_shell_capture_supports_profiles_explicit_start_and_no_ambient_shell() {
        let fixture = Fixture::new();
        let mut inputs = fixture.inputs();
        inputs
            .environment
            .push(("SHELL".into(), "/bin/false".into()));
        let authority = capture(inputs);
        let resolved = resolve(&authority, ".", false);
        let TerminalActionRequest::Exec { mut request } = resolved.request else {
            panic!()
        };
        for profile in [TerminalProfile::User, TerminalProfile::Clean] {
            request.profile = Some(profile);
            let shell = authority.exec_shell(&request).unwrap();
            assert_eq!(shell.profile(), profile);
            assert_eq!(shell.program(), Path::new("/bin/bash"));
        }
        let mut inputs = fixture.inputs();
        inputs.account_shell = TerminalHostAccountShell::Explicit(None);
        let absent = capture(inputs);
        assert!(absent.exec_shell(&request).is_err());
        let mut start =
            TerminalStartRequest::interactive(authority.identity.workspace.clone()).unwrap();
        start.shell = Some(TerminalShellSpec::Executable {
            path: "/bin/zsh".into(),
            clean_start: true,
        });
        let resolved = crate::terminal_native_launch::ResolvedTerminalNativeLaunch::resolve(
            &absent.launch_config(),
            &start,
        )
        .unwrap();
        assert_eq!(resolved.shell().profile(), TerminalProfile::Clean);
        let mut inputs = fixture.inputs();
        inputs.account_shell = TerminalHostAccountShell::CurrentUser;
        let current = capture(inputs);
        let expected = TerminalShell::for_current_user(None, None).unwrap();
        assert_eq!(
            current.launch_config().account_shell.as_deref(),
            Some(expected.program())
        );
    }

    #[test]
    fn cancellation_and_expired_deadlines_prevent_each_worker_handoff() {
        let fixture = Fixture::new();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        for (limit, token) in [
            (deadline(), cancelled),
            (Instant::now(), CancellationToken::new()),
        ] {
            assert!(
                TerminalHostAuthority::new(fixture.inputs())
                    .unwrap()
                    .capture_on_worker(limit, &token)
                    .is_err()
            );
            let authority = fixture.authority();
            assert!(
                authority
                    .resolve_on_worker(invocation(".", true), limit, &token)
                    .is_err()
            );
            assert!(
                authority
                    .launch_authority_on_worker(open(&fixture.0.join("workspace")), limit, &token)
                    .is_err()
            );
        }
    }
}
