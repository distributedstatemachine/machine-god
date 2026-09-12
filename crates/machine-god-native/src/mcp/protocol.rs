//! Effect-free, bounded MCP framing and protocol selection.
//!
//! Modern behavior is informed by fx `b1774fbf6c7602b503026f96f6e960e946c692ef`.
//! This module neither opens transports nor retries application requests. The
//! transport retains deadlines, cancellation, connection generations and write
//! ownership. Older versions and initialization/restart paths are not supported.

mod client_metadata;
mod json;
mod negotiation;
mod wire;

pub use client_metadata::McpClientMetadata;

/// Shared strict admission for bounded non-envelope protocol payloads.
pub(crate) fn parse_json(bytes: &[u8], limits: WireLimits) -> Result<serde_json::Value, WireError> {
    let limits = limits.validate()?;
    if bytes.len() > limits.max_frame_bytes {
        return Err(WireError::FrameTooLarge);
    }
    json::parse(bytes, limits)
}
pub use negotiation::{
    HttpDiscoveryStatus, NegotiatedProtocol, Negotiation, NegotiationAction, NegotiationFailure,
    ProtocolVersion, TransportKind,
};
pub use wire::{
    FrameProgress, NdjsonDecoder, RpcEnvelope, RpcId, RpcKind, RpcProtocolError, WireError,
    WireLimits, parse_envelope,
};

#[cfg(test)]
mod tests;
