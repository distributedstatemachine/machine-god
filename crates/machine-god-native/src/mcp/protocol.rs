//! Effect-free, bounded MCP framing and protocol selection.
//!
//! Compatibility decisions follow fx `b1774fbf6c7602b503026f96f6e960e946c692ef`.
//! This module neither opens transports nor retries application requests. The
//! transport retains deadlines, cancellation, connection generations and write
//! ownership. Only startup negotiation can produce a restart instruction.

mod json;
mod negotiation;
mod wire;

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
