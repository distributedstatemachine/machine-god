//! Native MCP configuration, protocol and command boundaries.
//!
//! Parsing configuration or protocol data does not grant process, network,
//! persistence or execution authority. Runtime effects require separate,
//! explicitly injected native ownership.

pub mod commands;
pub mod config;
pub mod context;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod endpoint;
pub mod headers;
#[cfg(all(feature = "mcp-http", any(target_os = "linux", target_os = "macos")))]
pub mod http;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod management;
pub mod pagination;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod peer;
pub mod protocol;

pub mod sse;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod stdio;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod store;
pub mod submission;
