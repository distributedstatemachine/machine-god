//! Modern ACP boundaries. Wire data never supplies native execution authority.

pub mod interaction;
pub mod projection;
pub mod prompt;
pub mod protocol;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod client_requests;

#[cfg(all(
    feature = "ai-gateway-http",
    any(target_os = "linux", target_os = "macos")
))]
pub mod session;

#[cfg(all(
    feature = "ai-gateway-http",
    any(target_os = "linux", target_os = "macos")
))]
pub mod commands;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod resources;

#[cfg(all(
    feature = "ai-gateway-http",
    any(target_os = "linux", target_os = "macos")
))]
pub mod driver;
#[cfg(all(
    feature = "ai-gateway-http",
    any(target_os = "linux", target_os = "macos")
))]
pub mod selection;
