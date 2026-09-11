//! Native MCP configuration, protocol and command boundaries.
//!
//! Parsing configuration or protocol data does not grant process, network,
//! persistence or execution authority. Runtime effects require separate,
//! explicitly injected native ownership.

pub mod commands;
pub mod config;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod endpoint;
pub mod protocol;

pub mod sse;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod store;
pub mod submission;
