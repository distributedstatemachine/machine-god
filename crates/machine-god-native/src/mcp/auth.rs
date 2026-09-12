//! Explicit, owned OAuth for native MCP profiles.

use super::http::{McpHttpClock, McpHttpDestination, McpHttpTrust};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{fmt, sync::Arc, time::Instant};

mod browser;
mod codec;
mod discovery;
mod network;
mod profile;
mod service;
mod store;
#[cfg(test)]
mod tests;
mod token;

pub use codec::{McpAuthChallenge, McpAuthConfig, McpAuthIdentity};
pub use profile::NativeMcpAuthProfile;
pub use service::{McpAuthLease, NativeMcpAuthCleanup, NativeMcpAuthService};
pub use store::NativeMcpCredentialStore;

const DOCUMENT_LIMIT: usize = 256 * 1024;
const SECRET_LIMIT: usize = 16 * 1024;
type Result<T> = std::result::Result<T, McpAuthError>;

/// Fixed diagnostics; metadata, URLs and credentials never enter formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpAuthError {
    Invalid,
    Limit,
    Unavailable,
    Denied,
    Cancelled,
    Deadline,
    Network,
    IssuerMismatch,
    StateMismatch,
    Rejected,
    Missing,
    Busy,
    Conflict,
    Persistence,
    AmbiguousPublication,
}
impl fmt::Display for McpAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid MCP authorization data",
            Self::Limit => "MCP authorization limit exceeded",
            Self::Unavailable => "MCP authorization is unavailable",
            Self::Denied => "MCP browser authorization was declined",
            Self::Cancelled => "MCP authorization was cancelled",
            Self::Deadline => "MCP authorization deadline exceeded",
            Self::Network => "MCP authorization exchange failed",
            Self::IssuerMismatch => "MCP authorization issuer does not match",
            Self::StateMismatch => "MCP authorization state does not match",
            Self::Rejected => "MCP authorization was rejected",
            Self::Missing => "MCP credentials are missing",
            Self::Busy => "MCP authorization is already active",
            Self::Conflict => "MCP credential generation changed",
            Self::Persistence => "MCP credential persistence failed",
            Self::AmbiguousPublication => "MCP credential publication is uncertain",
        })
    }
}
impl std::error::Error for McpAuthError {}

/// Separate wall-clock authority is needed only for persisted token expiry.
pub trait McpAuthClock: McpHttpClock {
    fn unix_millis(&self) -> i64;
}

/// Explicit cryptographically secure entropy authority. Implementations must
/// fill every requested byte or fail; deterministic sources belong only in tests.
pub trait McpAuthEntropy: Send + Sync {
    /// # Errors
    /// Reports unavailable or failed explicitly selected secure entropy.
    fn fill(&self, bytes: &mut [u8]) -> Result<()>;
}

/// Selected authority for one exact OAuth destination, not resource headers.
pub struct McpAuthDestination {
    pub destination: McpHttpDestination,
    pub trust: Option<McpHttpTrust>,
}

/// DNS/trust policy is injected independently for every exact OAuth URL.
/// Implementations must not infer credential or proxy authority from metadata.
pub trait McpAuthNetwork: Send + Sync {
    fn admit<'a>(
        &'a self,
        url: &'a str,
        cancellation: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<McpAuthDestination>>;
}

/// Presentation and browser execution are separately polled operations.
pub trait McpAuthBrowser: Send + Sync {
    fn approve<'a>(
        &'a self,
        request: &'a McpAuthBrowserRequest,
        cancellation: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<bool>>;
    fn launch<'a>(
        &'a self,
        request: &'a McpAuthBrowserRequest,
        cancellation: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<()>>;
}
/// Secret-bearing URL view for an explicit presenter/launcher. Never log it.
pub struct McpAuthBrowserRequest {
    url: Box<str>,
    issuer: Box<str>,
}
impl McpAuthBrowserRequest {
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
}

/// Exact generation invalidation, delivered outside service locks. The runtime
/// retains the same generation token through its executable allocations.
pub trait McpAuthInvalidation: Send + Sync {
    fn invalidate(&self, event: McpAuthInvalidated);
}
pub struct McpAuthInvalidated {
    pub identity: McpAuthIdentity,
    pub generation: CancellationToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpAuthLocalRemoval {
    Unchanged,
    Removed,
    Ambiguous,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpAuthRemoteRevocation {
    NotAttempted,
    Confirmed,
    Unsupported,
    Ambiguous,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct McpAuthLogoutReceipt {
    pub local: McpAuthLocalRemoval,
    pub remote: McpAuthRemoteRevocation,
}

pub(crate) struct Authority {
    network: Arc<dyn McpAuthNetwork>,
    clock: Arc<dyn McpAuthClock>,
    entropy: Arc<dyn McpAuthEntropy>,
}

macro_rules! redacted {
    ($($ty:ty),+ $(,)?) => {$(impl fmt::Debug for $ty {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(concat!(stringify!($ty), " { <redacted> }"))
        }
    })+};
}
pub(crate) use redacted;
redacted!(
    McpAuthDestination,
    McpAuthBrowserRequest,
    McpAuthInvalidated
);
