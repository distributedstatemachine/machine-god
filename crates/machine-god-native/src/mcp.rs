//! Native MCP configuration, protocol and command boundaries.
//!
//! Parsing configuration or protocol data does not grant process, network,
//! persistence or execution authority. Runtime effects require separate,
//! explicitly injected native ownership.

#[cfg(all(feature = "mcp-http", any(target_os = "linux", target_os = "macos")))]
pub mod auth;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod browser_launcher;
pub mod catalog;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod catalog_refresh;
#[cfg(all(feature = "mcp-http", any(target_os = "linux", target_os = "macos")))]
pub mod clock;
pub mod commands;
pub mod config;
pub mod context;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod continuation;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod control;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod controller;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod endpoint;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod ephemeral;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod execution;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod feature;
pub mod headers;
#[cfg(all(feature = "mcp-http", any(target_os = "linux", target_os = "macos")))]
pub mod http;
#[cfg(all(feature = "mcp-http", any(target_os = "linux", target_os = "macos")))]
pub mod http_peer;
pub mod interaction;
pub mod lifetime;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod management;
pub mod mrtr;
#[cfg(all(feature = "mcp-http", any(target_os = "linux", target_os = "macos")))]
pub mod network;
pub mod pagination;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod peer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod permission;
pub mod protocol;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod runtime;
pub mod schema;

pub mod sse;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod startup;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod stdio;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod stdio_startup;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod store;
pub mod submission;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod tool_result;
