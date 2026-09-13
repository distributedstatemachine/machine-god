//! Modern ACP boundaries. Wire data never supplies native execution authority.

pub mod interaction;
pub mod protocol;

#[cfg(all(
    feature = "ai-gateway-http",
    any(target_os = "linux", target_os = "macos")
))]
pub mod session;
