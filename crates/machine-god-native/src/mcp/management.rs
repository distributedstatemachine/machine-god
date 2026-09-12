//! Explicit profile management, separate from MCP runtime activation.

use std::{fmt, path::PathBuf, sync::Arc};

use machine_god_core::CancellationToken;

use super::{
    commands::{MAX_MCP_COMMAND_BYTES, McpCommand},
    config::{MAX_SERVER_NAME_BYTES, McpServerConfig, McpTransportConfig},
    store::{
        McpConfigCommitDurability, McpConfigMutation, NativeMcpConfigCommit, NativeMcpConfigStore,
        NativeMcpConfigStoreError,
    },
};

#[cfg(test)]
mod tests;

/// Native profile authority only; construction does not observe the profile.
pub struct NativeMcpManagementService {
    store: Arc<NativeMcpConfigStore>,
}

/// Nonsecret configuration metadata, not a connected-server status.
#[derive(Clone, PartialEq, Eq)]
pub struct McpConfiguredServer {
    pub name: Box<str>,
    pub transport: McpConfiguredTransport,
    pub enabled: bool,
    pub required: bool,
}

/// Configured transport kind, without executable, endpoint or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpConfiguredTransport {
    Stdio,
    Http,
    Sse,
}

/// Runtime activation is independent of filesystem publication durability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpManagementActivation {
    NotAttempted,
}

/// Owned observation or publication receipt. Saved data is not live runtime state.
pub enum NativeMcpManagementReceipt {
    Configured(Box<[McpConfiguredServer]>),
    Path(PathBuf),
    Saved {
        commit: NativeMcpConfigCommit,
        activation: McpManagementActivation,
    },
}

impl NativeMcpManagementReceipt {
    /// Reports publication uncertainty, never an inferred activation failure.
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(self, Self::Saved { commit, .. }
            if commit.durability() == McpConfigCommitDurability::Ambiguous)
    }
}

/// Fixed diagnostics omit paths, server names, arguments and credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMcpManagementError {
    InvalidCommand,
    Cancelled,
    Store(NativeMcpConfigStoreError),
    RuntimeUnavailable,
}

impl fmt::Display for NativeMcpManagementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCommand => "invalid MCP management command",
            Self::Cancelled => "MCP management cancelled",
            Self::Store(_) => "MCP profile management failed",
            Self::RuntimeUnavailable => "MCP runtime operation is unavailable",
        })
    }
}

impl std::error::Error for NativeMcpManagementError {}

impl From<NativeMcpConfigStoreError> for NativeMcpManagementError {
    fn from(error: NativeMcpConfigStoreError) -> Self {
        Self::Store(error)
    }
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($ty), " { <redacted> }"))
            }
        }
    )+};
}
redacted_debug!(
    NativeMcpManagementService,
    NativeMcpManagementReceipt,
    McpConfiguredServer
);

type Result<T> = std::result::Result<T, NativeMcpManagementError>;

impl NativeMcpManagementService {
    /// Retains explicitly selected store authority without loading or creating it.
    #[must_use]
    pub fn new(store: Arc<NativeMcpConfigStore>) -> Self {
        Self { store }
    }

    /// Shares the selected observation identity; never reopens a second store.
    pub(crate) fn config_store(&self) -> Arc<NativeMcpConfigStore> {
        self.store.clone()
    }

    /// Validates directly constructed profile commands without copying or effects.
    /// Runtime-only variants are unavailable, regardless of their input fields.
    ///
    /// # Errors
    /// Rejects forged tokens and command budgets; rejects runtime-only variants.
    pub fn validate_command(command: &McpCommand) -> Result<()> {
        match command {
            McpCommand::Summary | McpCommand::List | McpCommand::Path => Ok(()),
            McpCommand::Remove { server } => validate_server(server),
            McpCommand::Add {
                server,
                command,
                arguments,
            } => {
                validate_server(server)?;
                validate_token(command)?;
                if arguments.len() > 256 {
                    return Err(NativeMcpManagementError::InvalidCommand);
                }
                // Minimal parser spelling: "add <server> <command> [args...]".
                let mut bytes = 5 + server.len() + command.len();
                for argument in arguments {
                    validate_token(argument)?;
                    bytes += 1 + argument.len();
                    if bytes > MAX_MCP_COMMAND_BYTES {
                        return Err(NativeMcpManagementError::InvalidCommand);
                    }
                }
                Ok(())
            }
            _ => Err(NativeMcpManagementError::RuntimeUnavailable),
        }
    }

    /// Executes one bounded synchronous profile operation on the caller's worker.
    /// No worker, process, environment lookup or network operation is started.
    /// Cancellation is checked before observation and again before publication.
    /// Once publication starts, its receipt survives later cancellation.
    ///
    /// # Errors
    /// Invalid/unavailable intents and cancellation precede effects. Store errors
    /// precede rename; post-rename uncertainty remains an owned save receipt.
    pub fn execute(
        &self,
        command: McpCommand,
        cancellation: &CancellationToken,
    ) -> Result<NativeMcpManagementReceipt> {
        Self::validate_command(&command)?;
        check_cancelled(cancellation)?;
        match command {
            McpCommand::Path => Ok(NativeMcpManagementReceipt::Path(self.store.path().into())),
            McpCommand::Summary | McpCommand::List => {
                let snapshot = self.store.load()?;
                check_cancelled(cancellation)?;
                Ok(NativeMcpManagementReceipt::Configured(
                    snapshot
                        .config()
                        .servers()
                        .iter()
                        .map(project_server)
                        .collect(),
                ))
            }
            McpCommand::Add {
                server,
                command,
                arguments,
            } => {
                let args: Vec<_> = arguments.iter().map(String::as_str).collect();
                let config = McpServerConfig::stdio(&server, &command, &args)
                    .map_err(NativeMcpConfigStoreError::from)?;
                self.save(&McpConfigMutation::Replace(config), cancellation)
            }
            McpCommand::Remove { server } => {
                self.save(&McpConfigMutation::Remove(server.into()), cancellation)
            }
            _ => Err(NativeMcpManagementError::RuntimeUnavailable),
        }
    }

    fn save(
        &self,
        mutation: &McpConfigMutation,
        cancellation: &CancellationToken,
    ) -> Result<NativeMcpManagementReceipt> {
        check_cancelled(cancellation)?;
        let snapshot = self.store.load()?;
        check_cancelled(cancellation)?;
        let commit = futures_executor::block_on(self.store.apply(&snapshot, mutation))?;
        Ok(NativeMcpManagementReceipt::Saved {
            commit,
            activation: McpManagementActivation::NotAttempted,
        })
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(NativeMcpManagementError::Cancelled)
    } else {
        Ok(())
    }
}

fn validate_server(server: &str) -> Result<()> {
    if server.is_empty()
        || server.len() > MAX_SERVER_NAME_BYTES
        || !server
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(NativeMcpManagementError::InvalidCommand);
    }
    Ok(())
}

fn validate_token(token: &str) -> Result<()> {
    if token.is_empty() || token.len() > 4096 || token.chars().any(|c| c.is_control() || c == ' ') {
        return Err(NativeMcpManagementError::InvalidCommand);
    }
    Ok(())
}

fn project_server(server: &McpServerConfig) -> McpConfiguredServer {
    McpConfiguredServer {
        name: server.name().into(),
        transport: match server.transport() {
            McpTransportConfig::Stdio(_) => McpConfiguredTransport::Stdio,
            McpTransportConfig::Http(_) => McpConfiguredTransport::Http,
            McpTransportConfig::Sse(_) => McpConfiguredTransport::Sse,
        },
        enabled: server.enabled(),
        required: server.required(),
    }
}
