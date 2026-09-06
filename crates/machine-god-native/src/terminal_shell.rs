//! Explicit native account-shell resolution for interactive and captured terminals.

use std::fmt;
use std::path::{Path, PathBuf};

use machine_god_core::TerminalProfile;

const MAX_SHELL_PATH_BYTES: usize = 4096;
const MAX_SHELL_COMMAND_BYTES: usize = 64 * 1024;

/// Fixed, data-free shell selection failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalShellError {
    /// The profile and explicit shell selectors are mutually exclusive.
    ConflictingSelection,
    /// The account database did not provide a usable login shell.
    MissingLoginShell,
    /// A path or command violates the bounded invocation contract.
    InvalidRequest,
    /// Explicit executable selection accepts only bash and zsh.
    UnsupportedShell,
}

impl fmt::Display for TerminalShellError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal shell selection failed")
    }
}

impl std::error::Error for TerminalShellError {}

#[derive(Clone, Copy)]
enum ShellKind {
    Bash,
    Zsh,
    #[cfg(test)]
    LegacySh,
}

/// Resolved executable and startup profile; construction does not execute it.
#[derive(Clone)]
pub struct TerminalShell {
    program: PathBuf,
    kind: ShellKind,
    profile: TerminalProfile,
}

impl fmt::Debug for TerminalShell {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalShell")
            .finish_non_exhaustive()
    }
}

impl TerminalShell {
    /// Private monitor execution preserves pinned legacy `sh -lc` semantics.
    /// The caller resolves this absolute executable through its captured PATH;
    /// public interactive executable selection remains restricted to bash/zsh.
    #[cfg(test)]
    pub(crate) fn legacy_captured_sh(program: PathBuf) -> Result<Self, TerminalShellError> {
        validate_path(&program)?;
        Ok(Self {
            program,
            kind: ShellKind::LegacySh,
            profile: TerminalProfile::User,
        })
    }
    /// Resolves explicitly injected account data without filesystem or process effects.
    ///
    /// # Errors
    /// Rejects conflicting selectors, absent account data, relative/oversized paths,
    /// and unsupported explicit shell executables. Unsupported account shells use
    /// the platform's bash/zsh fallback, matching the pinned terminal resolver.
    pub fn from_account_shell(
        account_shell: Option<&Path>,
        profile: Option<TerminalProfile>,
        shell: Option<&Path>,
    ) -> Result<Self, TerminalShellError> {
        if profile.is_some() && shell.is_some() {
            return Err(TerminalShellError::ConflictingSelection);
        }
        let profile = profile.unwrap_or(TerminalProfile::User);
        let (program, kind) = if let Some(path) = shell {
            validate_path(path)?;
            (
                path.to_owned(),
                shell_kind(path).ok_or(TerminalShellError::UnsupportedShell)?,
            )
        } else {
            let path = account_shell.ok_or(TerminalShellError::MissingLoginShell)?;
            validate_path(path)?;
            if let Some(kind) = shell_kind(path) {
                (path.to_owned(), kind)
            } else {
                let path = fallback_shell();
                (
                    path.to_owned(),
                    shell_kind(path).ok_or(TerminalShellError::UnsupportedShell)?,
                )
            }
        };
        Ok(Self {
            program,
            kind,
            profile,
        })
    }

    /// Resolves an explicit executable and its independent clean-start flag.
    ///
    /// This constructor performs no account lookup or command execution.
    ///
    /// # Errors
    /// Rejects invalid paths and unsupported executable names.
    pub fn from_executable(path: &Path, clean_start: bool) -> Result<Self, TerminalShellError> {
        validate_path(path)?;
        Ok(Self {
            program: path.to_owned(),
            kind: shell_kind(path).ok_or(TerminalShellError::UnsupportedShell)?,
            profile: if clean_start {
                TerminalProfile::Clean
            } else {
                TerminalProfile::User
            },
        })
    }

    /// Resolves the current real user's login shell through the native account database.
    ///
    /// This explicitly effectful lookup can invoke system name-service providers;
    /// hosts must perform it on their bounded blocking executor, not an engine
    /// polling thread. Explicit shell selection does not query the account database.
    /// The `SHELL` environment variable is never consulted.
    ///
    /// # Errors
    /// Returns fixed failures for unavailable account data or invalid selection.
    pub fn for_current_user(
        profile: Option<TerminalProfile>,
        shell: Option<&Path>,
    ) -> Result<Self, TerminalShellError> {
        if shell.is_some() {
            return Self::from_account_shell(None, profile, shell);
        }
        let user = nix::unistd::User::from_uid(nix::unistd::Uid::current())
            .map_err(|_| TerminalShellError::MissingLoginShell)?
            .ok_or(TerminalShellError::MissingLoginShell)?;
        Self::from_account_shell(Some(&user.shell), profile, None)
    }

    /// Returns the exact program path for permission identity and launch.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Returns the selected startup profile.
    #[must_use]
    pub const fn profile(&self) -> TerminalProfile {
        self.profile
    }

    /// Returns interactive startup arguments, excluding argv[0].
    #[must_use]
    pub fn interactive_arguments(&self) -> Vec<String> {
        let flags: &[&str] = match (self.kind, self.profile) {
            (ShellKind::Bash, TerminalProfile::User) => &["--login", "-i"],
            (ShellKind::Bash, TerminalProfile::Clean) => &["--noprofile", "--norc", "-i"],
            (ShellKind::Zsh, TerminalProfile::User) => &["-l", "-i"],
            (ShellKind::Zsh, TerminalProfile::Clean) => &["-f", "-i"],
            #[cfg(test)]
            (ShellKind::LegacySh, _) => &["-li"],
        };
        flags.iter().map(|value| (*value).to_owned()).collect()
    }

    /// Returns captured-execution arguments, with the command as one exact argv item.
    ///
    /// # Errors
    /// Rejects empty, NUL-containing or oversized commands before launching anything.
    pub fn captured_arguments(&self, command: &str) -> Result<Vec<String>, TerminalShellError> {
        if command.is_empty() || command.len() > MAX_SHELL_COMMAND_BYTES || command.contains('\0') {
            return Err(TerminalShellError::InvalidRequest);
        }
        #[cfg(test)]
        if matches!(self.kind, ShellKind::LegacySh) {
            return Ok(vec!["-lc".into(), command.into()]);
        }
        let mut arguments = self.interactive_arguments();
        if self.profile == TerminalProfile::Clean || matches!(self.kind, ShellKind::Bash) {
            arguments.pop();
        }
        if self.profile == TerminalProfile::User && matches!(self.kind, ShellKind::Bash) {
            arguments.extend(["-O".to_owned(), "expand_aliases".to_owned()]);
        }
        arguments.extend(["-c".to_owned(), command.to_owned()]);
        Ok(arguments)
    }
}

fn validate_path(path: &Path) -> Result<(), TerminalShellError> {
    let text = path.to_str().ok_or(TerminalShellError::InvalidRequest)?;
    if !path.is_absolute() || text.len() > MAX_SHELL_PATH_BYTES || text.contains('\0') {
        return Err(TerminalShellError::InvalidRequest);
    }
    Ok(())
}

fn shell_kind(path: &Path) -> Option<ShellKind> {
    match path.file_name()?.to_str()? {
        "bash" => Some(ShellKind::Bash),
        "zsh" => Some(ShellKind::Zsh),
        _ => None,
    }
}

fn fallback_shell() -> &'static Path {
    if cfg!(target_os = "macos") {
        Path::new("/bin/zsh")
    } else {
        Path::new("/bin/bash")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_interactive_and_captured_flags_are_distinct() {
        for (path, profile, interactive, captured) in [
            (
                "/bin/bash",
                TerminalProfile::User,
                vec!["--login", "-i"],
                vec!["--login", "-O", "expand_aliases", "-c", "echo 'a b'\n"],
            ),
            (
                "/bin/bash",
                TerminalProfile::Clean,
                vec!["--noprofile", "--norc", "-i"],
                vec!["--noprofile", "--norc", "-c", "echo 'a b'\n"],
            ),
            (
                "/bin/zsh",
                TerminalProfile::User,
                vec!["-l", "-i"],
                vec!["-l", "-i", "-c", "echo 'a b'\n"],
            ),
            (
                "/bin/zsh",
                TerminalProfile::Clean,
                vec!["-f", "-i"],
                vec!["-f", "-c", "echo 'a b'\n"],
            ),
        ] {
            let shell =
                TerminalShell::from_account_shell(Some(Path::new(path)), Some(profile), None)
                    .unwrap();
            assert_eq!(shell.interactive_arguments(), interactive);
            assert_eq!(shell.captured_arguments("echo 'a b'\n").unwrap(), captured);
            let explicit =
                TerminalShell::from_executable(Path::new(path), profile == TerminalProfile::Clean)
                    .unwrap();
            assert_eq!(explicit.interactive_arguments(), interactive);
        }
    }

    #[test]
    fn account_fallback_and_explicit_selection_preserve_authority() {
        let shell = TerminalShell::from_account_shell(Some(Path::new("/usr/bin/fish")), None, None)
            .unwrap();
        assert_eq!(shell.program(), fallback_shell());
        assert_eq!(shell.profile(), TerminalProfile::User);
        assert_eq!(
            TerminalShell::from_account_shell(None, None, None).unwrap_err(),
            TerminalShellError::MissingLoginShell
        );
        assert_eq!(
            TerminalShell::from_account_shell(None, None, Some(Path::new("/usr/bin/fish")))
                .unwrap_err(),
            TerminalShellError::UnsupportedShell
        );
        assert_eq!(
            TerminalShell::from_account_shell(
                None,
                Some(TerminalProfile::Clean),
                Some(Path::new("/bin/bash"))
            )
            .unwrap_err(),
            TerminalShellError::ConflictingSelection
        );
        assert!(TerminalShell::from_account_shell(Some(Path::new("bash")), None, None).is_err());
        let shell = TerminalShell::for_current_user(None, Some(Path::new("/bin/bash"))).unwrap();
        assert_eq!(format!("{shell:?}"), "TerminalShell { .. }");
        for command in [
            String::new(),
            "x\0y".to_owned(),
            "x".repeat(MAX_SHELL_COMMAND_BYTES + 1),
        ] {
            assert_eq!(
                shell.captured_arguments(&command),
                Err(TerminalShellError::InvalidRequest)
            );
        }
    }
}
