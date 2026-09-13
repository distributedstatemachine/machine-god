//! Session-owned ACP MCP selections. No profile or credential store authority.

mod owner;
#[cfg(test)]
mod tests;
pub(crate) use owner::NativeMcpEphemeralCatalog;
pub use owner::{NativeMcpEphemeralOwner, NativeMcpEphemeralReceipt};

#[cfg(feature = "mcp-http")]
use super::headers::McpResolvedHeaders;
use super::{
    config::{McpConfig, McpConfigError},
    lifetime::McpPeerLifetime,
    runtime::{NativeMcpRuntime, NativeMcpRuntimeClock, NativeMcpRuntimeError},
    startup::NativeMcpStartupError,
    stdio_startup::NativeMcpStdioStartup,
};
use crate::NativeOwnedWorkerScope;
use machine_god_core::{CancellationToken, ToolName};
use std::{ffi::OsString, fmt, sync::Arc, time::Instant};

/// Pure, bounded, authoritative ACP `mcpServers` selection. Secret-bearing data
/// is deliberately neither publicly exposed nor serializable as profile state.
#[derive(Clone)]
pub struct NativeMcpEphemeralConfiguration {
    configuration: Arc<McpConfig>,
    #[cfg(feature = "mcp-http")]
    headers: Vec<(Box<str>, McpResolvedHeaders)>,
    identities: Vec<Arc<[u8]>>,
}
impl NativeMcpEphemeralConfiguration {
    /// Omission and `[]` select no servers. Null, profile syntax and deprecated
    /// transports are rejected before acquiring any native authority.
    /// # Errors
    /// Rejects malformed/duplicate input, invalid fields and finite tree bounds.
    pub fn decode(raw_servers: Option<&[u8]>) -> Result<Self, McpConfigError> {
        let (configuration, headers, identities) = super::config::decode_acp(raw_servers)?;
        #[cfg(not(feature = "mcp-http"))]
        drop(headers);
        Ok(Self {
            configuration: Arc::new(configuration),
            #[cfg(feature = "mcp-http")]
            headers,
            identities,
        })
    }
    #[must_use]
    pub fn server_count(&self) -> usize {
        self.configuration.servers().len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.server_count() == 0
    }
}

/// Explicit captured authorities for exactly one session incarnation. Supply a
/// fresh dedicated runtime; a profile controller must never share that runtime.
/// There is intentionally no configuration store or authentication-service field.
pub struct NativeMcpEphemeralOptions {
    pub runtime: Arc<NativeMcpRuntime>,
    pub workers: NativeOwnedWorkerScope,
    pub reserved_tool_names: Box<[ToolName]>,
    pub captured_environment: Vec<(OsString, OsString)>,
    pub stdio: Option<Arc<NativeMcpStdioStartup>>,
    pub clock: Arc<dyn NativeMcpRuntimeClock>,
    pub catalog_epoch: Instant,
    pub owner_cancellation: CancellationToken,
    #[cfg(feature = "mcp-http")]
    pub network: Option<Arc<super::network::NativeMcpNetwork>>,
    pub peer_lifetime: McpPeerLifetime,
    /// Positive, at most 256 MiB per startup generation; runtime adds its own
    /// shared active-plus-retired publication bound.
    pub max_retained_bytes: usize,
    /// Active, pending, retired and retained receipt generations; 1..=8.
    pub max_retained_generations: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMcpEphemeralError {
    Invalid,
    Limit,
    Busy,
    Closed,
    Cancelled,
    Deadline,
    Unavailable,
    Startup(NativeMcpStartupError),
    Runtime(NativeMcpRuntimeError),
}
impl fmt::Display for NativeMcpEphemeralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ACP MCP selection unavailable")
    }
}
impl std::error::Error for NativeMcpEphemeralError {}
impl From<NativeMcpStartupError> for NativeMcpEphemeralError {
    fn from(value: NativeMcpStartupError) -> Self {
        Self::Startup(value)
    }
}
impl From<NativeMcpRuntimeError> for NativeMcpEphemeralError {
    fn from(value: NativeMcpRuntimeError) -> Self {
        Self::Runtime(value)
    }
}

macro_rules! redacted { ($($ty:ty),+ $(,)?) => {$(impl fmt::Debug for $ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($ty), " { <redacted> }")) }
})+}; }
redacted!(NativeMcpEphemeralConfiguration, NativeMcpEphemeralOptions);
