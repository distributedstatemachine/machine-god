use super::invalid;
use crate::{TerminalActionInvocation, TerminalShell};
use machine_god_core::{BoxFuture, CancellationToken, PermissionError, TerminalActionRequest};
use std::{fmt, fs::File};

/// Explicit actual-host resolution authority, never a model-selected cwd resolver.
pub trait NativePermissionTerminalResolver: Send + Sync + 'static {
    fn resolve(
        &self,
        invocation: TerminalActionInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<NativePermissionTerminalResolution, PermissionError>>;
}

/// Actual host selection retained for reviewer and exact-action identity.
/// This evidence is not a direct-execution plan or permission to execute.
pub struct NativePermissionTerminalResolution {
    request: TerminalActionRequest,
    cwd: Option<File>,
    shell: Option<TerminalShell>,
    environment_sha256: String,
    shell_selection_sha256: String,
}
impl fmt::Debug for NativePermissionTerminalResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativePermissionTerminalResolution { .. }")
    }
}
impl NativePermissionTerminalResolution {
    /// Constructs evidence supplied by a trusted native resolver, without effects.
    /// # Errors
    /// Rejects malformed actions/digests or missing command cwd/shell ownership.
    pub fn new(
        request: TerminalActionRequest,
        cwd: Option<File>,
        shell: Option<TerminalShell>,
        environment_sha256: String,
        shell_selection_sha256: String,
    ) -> Result<Self, PermissionError> {
        request.validate().map_err(|_| invalid())?;
        let command = matches!(
            request,
            TerminalActionRequest::Exec { .. } | TerminalActionRequest::Start { .. }
        );
        if command != cwd.is_some()
            || command != shell.is_some()
            || [&environment_sha256, &shell_selection_sha256]
                .iter()
                .any(|digest| {
                    digest.len() != 64
                        || !digest
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
        {
            return Err(invalid());
        }
        Ok(Self {
            request,
            cwd,
            shell,
            environment_sha256,
            shell_selection_sha256,
        })
    }
    #[must_use]
    pub const fn action(&self) -> &TerminalActionRequest {
        &self.request
    }
    #[must_use]
    pub const fn cwd(&self) -> Option<&File> {
        self.cwd.as_ref()
    }
    #[must_use]
    pub const fn shell(&self) -> Option<&TerminalShell> {
        self.shell.as_ref()
    }
    #[must_use]
    pub fn environment_sha256(&self) -> &str {
        &self.environment_sha256
    }
    #[must_use]
    pub fn shell_selection_sha256(&self) -> &str {
        &self.shell_selection_sha256
    }
}
