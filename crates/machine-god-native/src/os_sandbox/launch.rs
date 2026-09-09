use std::fmt;
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use machine_god_core::CancellationToken;
use rustix::fd::AsFd;
use rustix::fs::{FileType, Mode, OFlags, Stat};

use crate::{NativeSandboxMode, PermissionMode};

/// Pinned fx permits one primary and sixteen additional workspace directories.
pub const MAX_NATIVE_SANDBOX_ROOTS: usize = 17;
pub const MAX_NATIVE_SANDBOX_ROOT_PATH_BYTES: usize = 4096;
/// Every UTF-8 path byte can require two SBPL bytes, plus fixed rule syntax.
pub const MAX_NATIVE_SANDBOX_PROFILE_BYTES: usize =
    MAX_NATIVE_SANDBOX_ROOTS * (2 * MAX_NATIVE_SANDBOX_ROOT_PATH_BYTES + 40) + 1024;
pub const NATIVE_SANDBOX_EXECUTABLE: &str = "/usr/bin/sandbox-exec";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSandboxError {
    Invalid,
    Unsupported,
    Unavailable,
    Changed,
    Cancelled,
    Timeout,
}
impl fmt::Display for NativeSandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native sandbox launch failed")
    }
}
impl std::error::Error for NativeSandboxError {}
type Result<T> = std::result::Result<T, NativeSandboxError>;

/// A caller-supplied descriptor and exact canonical spelling, not ambient cwd.
#[derive(Clone)]
pub struct NativeSandboxRoot {
    directory: Arc<File>,
    path: PathBuf,
}
impl fmt::Debug for NativeSandboxRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSandboxRoot").finish_non_exhaustive()
    }
}
impl NativeSandboxRoot {
    /// Construction checks only bounded path syntax. Native validation belongs
    /// to capture/revalidation on the caller's owned effect worker.
    /// # Errors
    /// Rejects nonabsolute, noncanonical lexical, non-UTF-8 or oversized paths.
    pub fn new(directory: File, canonical_path: PathBuf) -> Result<Self> {
        validate_path(&canonical_path)?;
        Ok(Self {
            directory: Arc::new(directory),
            path: canonical_path,
        })
    }

    #[must_use]
    pub fn canonical_path(&self) -> &Path {
        &self.path
    }
}

/// A taken job's immutable configured/effective policy and retained authority.
/// Changing host preferences cannot mutate an existing snapshot.
#[derive(Clone)]
pub struct NativeSandboxLaunch {
    configured: NativeSandboxMode,
    effective: NativeSandboxMode,
    roots: Arc<[NativeSandboxRoot]>,
    executable: Option<Arc<File>>,
    profile: Arc<str>,
    workspace_scope: Option<Arc<crate::NativeWorkspaceTurnScope>>,
}
impl fmt::Debug for NativeSandboxLaunch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSandboxLaunch")
            .field("configured", &self.configured)
            .field("effective", &self.effective)
            .finish_non_exhaustive()
    }
}
impl NativeSandboxLaunch {
    /// Capture on an owned blocking worker. The executable, if required, must
    /// be the explicitly opened immutable system `/usr/bin/sandbox-exec` file.
    /// No root, HOME, executable or environment is discovered automatically.
    /// The supplied deadline is never reset.
    /// # Errors
    /// Rejects unsupported OS isolation, incomplete/replaced authority, excessive
    /// roots, cancellation, timeout or an unavailable protected system launcher.
    #[allow(clippy::too_many_arguments)] // All authority and the existing deadline are explicit.
    pub fn capture(
        configured: NativeSandboxMode,
        permission_mode: PermissionMode,
        roots: Vec<NativeSandboxRoot>,
        executable: Option<File>,
        allow_localhost_listen: bool,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        check(deadline, cancellation)?;
        if roots.len() > MAX_NATIVE_SANDBOX_ROOTS {
            return Err(NativeSandboxError::Invalid);
        }
        let effective = if permission_mode == PermissionMode::Yolo {
            NativeSandboxMode::None
        } else {
            configured
        };
        if effective == NativeSandboxMode::Os && !cfg!(target_os = "macos") {
            return Err(NativeSandboxError::Unsupported);
        }
        if effective == NativeSandboxMode::Os && (roots.is_empty() || executable.is_none()) {
            return Err(NativeSandboxError::Unavailable);
        }
        let profile = if effective == NativeSandboxMode::Os {
            build_profile(&roots, allow_localhost_listen)?
        } else {
            String::new()
        };
        let result = Self {
            configured,
            effective,
            roots: roots.into(),
            executable: executable.map(Arc::new),
            profile: profile.into(),
            workspace_scope: None,
        };
        result.revalidate(deadline, cancellation)?;
        Ok(result)
    }

    pub(crate) fn with_shared_workspace_scope(
        mut self,
        scope: Arc<crate::NativeWorkspaceTurnScope>,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        check(deadline, cancellation)?;
        self.workspace_scope = Some(scope);
        self.validate_workspace_scope()?;
        check(deadline, cancellation)?;
        Ok(self)
    }

    /// Called only while constructing an installed monitor's independent grant.
    /// This pure transfer keeps immutable OS authority, not the source turn alive.
    /// Ordinary process launches must retain their live-turn proof until release.
    #[cfg(any(test, feature = "ai-gateway-http"))]
    pub(crate) fn for_installed_monitor(&self) -> Result<Self> {
        self.validate_workspace_scope()?;
        let mut retained = self.clone();
        retained.workspace_scope = None;
        Ok(retained)
    }

    fn validate_workspace_scope(&self) -> Result<()> {
        if self
            .workspace_scope
            .as_ref()
            .is_some_and(|scope| !scope.is_live())
        {
            Err(NativeSandboxError::Unavailable)
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub const fn configured(&self) -> NativeSandboxMode {
        self.configured
    }
    #[must_use]
    pub const fn effective(&self) -> NativeSandboxMode {
        self.effective
    }
    #[must_use]
    pub fn roots(&self) -> &[NativeSandboxRoot] {
        &self.roots
    }

    /// Revalidate immediately before native launch/release on its owned worker.
    /// This is a bounded final check, not an atomic check-and-exec guarantee.
    /// # Errors
    /// Rejects changed authority, unavailable isolation, cancellation or expiry.
    pub fn revalidate(&self, deadline: Instant, cancellation: &CancellationToken) -> Result<()> {
        check(deadline, cancellation)?;
        self.validate_workspace_scope()?;
        if self.effective == NativeSandboxMode::None {
            return Ok(());
        }
        if !cfg!(target_os = "macos") {
            return Err(NativeSandboxError::Unsupported);
        }
        for root in self.roots.iter() {
            validate_binding(&root.directory, &root.path, true, deadline, cancellation)?;
        }
        let executable = self
            .executable
            .as_ref()
            .ok_or(NativeSandboxError::Unavailable)?;
        validate_binding(
            executable,
            Path::new(NATIVE_SANDBOX_EXECUTABLE),
            false,
            deadline,
            cancellation,
        )?;
        check(deadline, cancellation)?;
        self.validate_workspace_scope()
    }

    /// Bounded, effect-free argv wrapping. The original shell remains an exact
    /// argument; no command, path or profile is interpolated into shell source.
    pub(crate) fn wrap(
        &self,
        program: String,
        arguments: Vec<String>,
    ) -> Result<(String, Vec<String>)> {
        if self.effective == NativeSandboxMode::None {
            return Ok((program, arguments));
        }
        if !cfg!(target_os = "macos") {
            return Err(NativeSandboxError::Unsupported);
        }
        let mut wrapped = Vec::with_capacity(arguments.len() + 3);
        wrapped.extend(["-p".into(), self.profile.to_string(), program]);
        wrapped.extend(arguments);
        Ok((NATIVE_SANDBOX_EXECUTABLE.into(), wrapped))
    }

    pub(crate) fn command(
        &self,
        program: &std::ffi::OsStr,
        arguments: &[std::ffi::OsString],
    ) -> Result<std::process::Command> {
        let mut command = if self.effective == NativeSandboxMode::None {
            std::process::Command::new(program)
        } else {
            if !cfg!(target_os = "macos") {
                return Err(NativeSandboxError::Unsupported);
            }
            let mut command = std::process::Command::new(NATIVE_SANDBOX_EXECUTABLE);
            command.arg("-p").arg(self.profile.as_ref()).arg(program);
            command
        };
        command.args(arguments);
        Ok(command)
    }
}

fn validate_path(path: &Path) -> Result<()> {
    if path.as_os_str().as_bytes().len() > MAX_NATIVE_SANDBOX_ROOT_PATH_BYTES {
        return Err(NativeSandboxError::Invalid);
    }
    let text = path.to_str().ok_or(NativeSandboxError::Invalid)?;
    if !path.is_absolute()
        || text.len() > MAX_NATIVE_SANDBOX_ROOT_PATH_BYTES
        || text.contains('\0')
        || (text != "/"
            && text[1..]
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | "..")))
    {
        return Err(NativeSandboxError::Invalid);
    }
    Ok(())
}

fn check(deadline: Instant, cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(NativeSandboxError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(NativeSandboxError::Timeout)
    } else {
        Ok(())
    }
}

fn call<T>(
    deadline: Instant,
    cancellation: &CancellationToken,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    check(deadline, cancellation)?;
    let result = operation();
    check(deadline, cancellation)?;
    result
}

fn same_identity(left: &Stat, right: &Stat) -> bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && FileType::from_raw_mode(left.st_mode) == FileType::from_raw_mode(right.st_mode)
}

fn validate_binding(
    file: &File,
    path: &Path,
    directory: bool,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<()> {
    let held = call(deadline, cancellation, || {
        rustix::fs::fstat(file).map_err(|_| NativeSandboxError::Unavailable)
    })?;
    let kind = FileType::from_raw_mode(held.st_mode);
    if held.st_nlink == 0
        || (directory && !kind.is_dir())
        || (!directory
            && (!kind.is_file()
                || held.st_uid != 0
                || held.st_mode & 0o022 != 0
                || held.st_mode & 0o111 == 0))
    {
        return Err(NativeSandboxError::Changed);
    }
    let canonical = call(deadline, cancellation, || {
        std::fs::canonicalize(path).map_err(|_| NativeSandboxError::Unavailable)
    })?;
    if canonical != path {
        return Err(NativeSandboxError::Changed);
    }
    let named = call(deadline, cancellation, || {
        rustix::fs::open(
            path,
            OFlags::RDONLY
                | OFlags::CLOEXEC
                | OFlags::NOFOLLOW
                | OFlags::NONBLOCK
                | if directory {
                    OFlags::DIRECTORY
                } else {
                    OFlags::empty()
                },
            Mode::empty(),
        )
        .map_err(|_| NativeSandboxError::Unavailable)
    })?;
    let named = call(deadline, cancellation, || {
        rustix::fs::fstat(named.as_fd()).map_err(|_| NativeSandboxError::Unavailable)
    })?;
    if !same_identity(&held, &named) {
        return Err(NativeSandboxError::Changed);
    }
    Ok(())
}

pub(super) fn build_profile(roots: &[NativeSandboxRoot], localhost: bool) -> Result<String> {
    let mut result = String::from("(version 1)\n(deny default)\n(allow file-read*)\n");
    for root in roots {
        result.push_str("(allow file-write* (subpath ");
        quote(
            &mut result,
            root.path.to_str().ok_or(NativeSandboxError::Invalid)?,
        );
        result.push_str("))\n");
    }
    result.push_str("(allow file-write* (subpath \"/tmp\"))\n(allow file-write* (subpath \"/private/tmp\"))\n(allow file-write* (subpath \"/dev\"))\n(allow process-exec)\n(allow process-fork)\n(allow sysctl-read)\n(allow mach-lookup)\n(allow network-outbound)\n(allow signal)\n(allow iokit-open)\n");
    if localhost {
        result.push_str("(allow network-bind (local ip \"localhost:*\"))\n(allow network-inbound (local ip \"localhost:*\"))\n");
    }
    if result.len() > MAX_NATIVE_SANDBOX_PROFILE_BYTES {
        return Err(NativeSandboxError::Invalid);
    }
    Ok(result)
}

pub(super) fn quote(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other => output.push(other),
        }
    }
    output.push('"');
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) static NATIVE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
