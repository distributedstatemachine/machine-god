//! Inert stdio launch factories from exact native MCP configuration snapshots.

use super::{
    config::{McpServerConfig, McpTransportConfig},
    peer::McpStdioLaunchFactory,
    protocol::WireLimits,
    stdio::{McpStdioError, McpStdioLaunch},
};
use crate::{
    background_process::ValidatedBackgroundEnvironment, terminal_helper::TerminalPtyHelper,
};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::{
    ffi::{OsStr, OsString},
    fmt,
    fs::File,
    path::PathBuf,
    sync::Arc,
};

#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, McpStdioError>;
// Exact Zig 0.16.0 std.Io.Threaded default when its captured parent PATH is absent.
const PINNED_DEFAULT_PATH: &str = "/usr/local/bin:/bin/:/usr/bin";

/// Native startup's explicitly selected process authority. Construction validates
/// data only: no environment read, cwd/path lookup, worker, process or descriptor
/// duplication. Captured environment must already be complete and intentional.
pub struct NativeMcpStdioStartup {
    helper: Arc<TerminalPtyHelper>,
    environment: ValidatedBackgroundEnvironment,
    search_path: Option<Arc<OsStr>>,
    cwd: Arc<File>,
    limits: WireLimits,
}
impl fmt::Debug for NativeMcpStdioStartup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpStdioStartup { <redacted> }")
    }
}
impl NativeMcpStdioStartup {
    /// Selects immutable launch authority without effects. PATH comes from captured
    /// parent environment, independently of a server's replacement child environment.
    /// Missing PATH uses the pinned fixed default; empty segments never imply cwd.
    /// # Errors
    /// Rejects malformed helper, duplicate/oversized environment, oversized PATH
    /// and invalid wire bounds. Unix PATH bytes are retained without lossy decoding.
    pub fn new(
        helper_program: PathBuf,
        helper_arguments: Vec<OsString>,
        captured_environment: Vec<(OsString, OsString)>,
        cwd: Arc<File>,
        limits: WireLimits,
    ) -> Result<Self> {
        let helper = TerminalPtyHelper::new(helper_program, helper_arguments)
            .map_err(|_| McpStdioError::Invalid)?;
        let environment = ValidatedBackgroundEnvironment::new(captured_environment)
            .map_err(|_| McpStdioError::Invalid)?;
        let search_path = selected_path(&environment)?;
        Ok(Self {
            helper: Arc::new(helper),
            environment,
            search_path,
            cwd,
            limits: limits.validate().map_err(|_| McpStdioError::Invalid)?,
        })
    }

    /// Selects one inert shared macOS inventory-service registration for this
    /// startup authority. Every later server factory/restart retains this allocation.
    /// The selected program/args must identify the trusted inventory entrypoint.
    /// # Errors
    /// Rejects invalid selection or a second registration on the same authority.
    #[cfg(target_os = "macos")]
    pub fn with_process_inventory_service(
        mut self,
        program: PathBuf,
        arguments: Vec<OsString>,
    ) -> Result<Self> {
        if self.helper.inventory_helper().is_some() {
            return Err(McpStdioError::Invalid);
        }
        let inventory = crate::process_inventory_helper::ProcessInventoryHelper::new_service(
            program, arguments,
        )
        .map_err(|_| McpStdioError::Invalid)?;
        self.helper = Arc::new(
            self.helper
                .as_ref()
                .clone()
                .with_inventory_helper(inventory),
        );
        Ok(self)
    }

    /// Freezes one enabled stdio server without opening its executable or cwd.
    /// Nonempty configured environment replaces the complete captured child
    /// environment; empty configuration retains it. Values are never expanded.
    /// # Errors
    /// Rejects disabled/non-stdio servers, missing macOS inventory selection,
    /// or an environment/argv exceeding the existing native launch bounds.
    pub fn factory(&self, configuration: Arc<McpServerConfig>) -> Result<NativeMcpStdioFactory> {
        if !configuration.enabled() {
            return Err(McpStdioError::Invalid);
        }
        let McpTransportConfig::Stdio(stdio) = configuration.transport() else {
            return Err(McpStdioError::Invalid);
        };
        #[cfg(target_os = "macos")]
        if self.helper.inventory_helper().is_none() {
            return Err(McpStdioError::Invalid);
        }
        let environment = if configuration.environment().is_empty() {
            self.environment.clone()
        } else {
            ValidatedBackgroundEnvironment::new(
                configuration
                    .environment()
                    .iter()
                    .map(|(key, value)| {
                        (OsString::from(key.as_ref()), OsString::from(value.as_ref()))
                    })
                    .collect(),
            )
            .map_err(|_| McpStdioError::Invalid)?
        };
        let template = McpStdioLaunch::from_validated(
            self.helper.clone(),
            Arc::from(stdio.command()),
            stdio
                .args()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .into(),
            environment,
            self.search_path.clone(),
            self.cwd.clone(),
            self.limits,
        )?;
        Ok(NativeMcpStdioFactory {
            configuration,
            template,
        })
    }
}

/// Exact server snapshot plus immutable launch data. Clones/restarts share helper,
/// command, argv, validated environment/frame, lookup PATH and retained cwd.
#[derive(Clone)]
pub struct NativeMcpStdioFactory {
    configuration: Arc<McpServerConfig>,
    template: McpStdioLaunch,
}
impl fmt::Debug for NativeMcpStdioFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpStdioFactory { <redacted> }")
    }
}
impl NativeMcpStdioFactory {
    /// Exact retained configuration, including required/startup/operation/restart
    /// policy. The runtime interprets policy; this factory does not negotiate.
    #[must_use]
    pub fn configuration(&self) -> &Arc<McpServerConfig> {
        &self.configuration
    }
}
impl McpStdioLaunchFactory for NativeMcpStdioFactory {
    fn launch(&mut self) -> Result<McpStdioLaunch> {
        Ok(self.template.clone())
    }
}

fn selected_path(environment: &ValidatedBackgroundEnvironment) -> Result<Option<Arc<OsStr>>> {
    let raw = environment
        .entries()
        .iter()
        .find(|(key, _)| key == "PATH")
        .map_or(OsStr::new(PINNED_DEFAULT_PATH), |(_, value)| {
            value.as_os_str()
        });
    McpStdioLaunch::admit_search_path(raw)?;
    let mut path = Vec::with_capacity(raw.len());
    for segment in raw
        .as_bytes()
        .split(|byte| *byte == b':')
        .filter(|part| !part.is_empty())
    {
        if !path.is_empty() {
            path.push(b':');
        }
        path.extend_from_slice(segment);
    }
    Ok((!path.is_empty()).then(|| Arc::from(OsString::from_vec(path))))
}
