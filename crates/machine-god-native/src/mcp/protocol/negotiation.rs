use serde_json::Value;

use super::{RpcEnvelope, RpcId, RpcProtocolError, WireError};

/// Configured transport families are distinct, not an automatic fallback chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportKind {
    /// Owned process with incremental NDJSON input/output.
    Stdio,
    /// Request-scoped modern Streamable HTTP.
    StreamableHttp,
}

/// The selected modern protocol; older versions are not admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolVersion {
    /// Stateless modern discovery and request envelopes, 2026-07-28.
    Modern,
}

impl ProtocolVersion {
    /// Exact wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Modern => "2026-07-28",
        }
    }

    /// Parse the modern version supported by either admitted transport.
    #[must_use]
    pub fn parse_for(_transport: TransportKind, text: &str) -> Option<Self> {
        (text == Self::Modern.as_str()).then_some(Self::Modern)
    }
}

/// A selected protocol still requires capability/schema and ownership admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiatedProtocol {
    /// The configured transport, unchanged throughout negotiation.
    pub transport: TransportKind,
    /// The admitted wire version, not an execution or capability grant.
    pub version: ProtocolVersion,
}

impl NegotiatedProtocol {
    /// Modern HTTP includes the exact selected protocol header.
    #[must_use]
    pub const fn sends_http_protocol_header(self) -> bool {
        matches!(self.transport, TransportKind::StreamableHttp)
    }
}

/// Status admission is transport-owned, before inspecting discovery evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpDiscoveryStatus {
    /// Stdio response or HTTP 200.
    Ordinary,
    /// Initial HTTP 400 admitted specifically to classify version evidence.
    VersionError,
}

/// Redacted terminal negotiation reason; remote messages/data are not retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NegotiationFailure {
    /// Malformed input, exhausted wire budget or failed correlation.
    Wire(WireError),
    /// A success payload did not satisfy the discovery shape.
    InvalidDiscovery,
    /// No supported version or bounded permitted retry remains.
    UnsupportedVersion,
    /// A well-formed terminal error; code only.
    ProtocolError(i64),
    /// Observation cannot occur in this transport/state combination.
    WrongState,
    /// Host control explicitly cancelled startup.
    Cancelled,
    /// Host startup/overall deadline expired.
    Deadline,
    /// Non-negotiable transport failure.
    Transport,
}

/// Explicit startup action, never an application-request retry authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NegotiationAction {
    /// Issue modern `server/discover`, with a newly allocated request ID.
    SendDiscover,
    /// Protocol accepted; the host must still validate capabilities and catalog.
    Ready(NegotiatedProtocol),
    /// No additional negotiation I/O may follow on this machine.
    Failed(NegotiationFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Discover { retried: bool },
    Ready,
    Failed,
}

/// Finite startup selection. The host owns IDs, deadlines and connection epochs.
///
/// Stdio has one discover attempt; HTTP permits one explicitly signaled
/// same-modern-version retry. No initialize or downgrade path exists.
/// This state machine cannot accept or replay ordinary MCP operations.
#[derive(Debug)]
pub struct Negotiation {
    transport: TransportKind,
    state: State,
}

impl Negotiation {
    /// Construct an inert machine and obtain its first startup action.
    #[must_use]
    pub const fn new(transport: TransportKind) -> (Self, NegotiationAction) {
        (
            Self {
                transport,
                state: State::Discover { retried: false },
            },
            NegotiationAction::SendDiscover,
        )
    }

    /// Observe a validated response for the current exact outstanding startup ID.
    ///
    /// Correlation failures are terminal, never downgrade evidence. Only HTTP
    /// discovery permits null-ID error responses (including its one retry). The host must bind this
    /// call to the current connection and outstanding request, not a retired one.
    pub fn response(
        &mut self,
        response: &RpcEnvelope,
        expected: &RpcId,
        status: HttpDiscoveryStatus,
    ) -> NegotiationAction {
        if matches!(self.state, State::Ready | State::Failed) {
            return self.fail(NegotiationFailure::WrongState);
        }
        let null_error = self.transport == TransportKind::StreamableHttp
            && matches!(self.state, State::Discover { .. });
        if let Err(error) = response.correlate(expected, null_error) {
            return self.fail(NegotiationFailure::Wire(error));
        }
        match self.state {
            State::Discover { retried } => self.discovery(response, status, retried),
            _ => self.fail(NegotiationFailure::WrongState),
        }
    }

    /// Terminate on cancellation, deadline, malformed wire or transport failure.
    pub fn abort(&mut self, failure: NegotiationFailure) -> NegotiationAction {
        self.fail(failure)
    }

    fn fail(&mut self, failure: NegotiationFailure) -> NegotiationAction {
        self.state = State::Failed;
        NegotiationAction::Failed(failure)
    }

    fn ready(&mut self, version: ProtocolVersion) -> NegotiationAction {
        self.state = State::Ready;
        NegotiationAction::Ready(NegotiatedProtocol {
            transport: self.transport,
            version,
        })
    }

    fn discovery(
        &mut self,
        response: &RpcEnvelope,
        status: HttpDiscoveryStatus,
        retried: bool,
    ) -> NegotiationAction {
        if self.transport != TransportKind::StreamableHttp
            && status != HttpDiscoveryStatus::Ordinary
        {
            return self.fail(NegotiationFailure::WrongState);
        }
        if let Some(error) = response.protocol_error() {
            if self.transport == TransportKind::StreamableHttp
                && error.code == -32022
                && supports_modern(&error)
            {
                if status == HttpDiscoveryStatus::VersionError && !retried {
                    self.state = State::Discover { retried: true };
                    return NegotiationAction::SendDiscover;
                }
                return self.fail(NegotiationFailure::UnsupportedVersion);
            }
            return self.fail(NegotiationFailure::ProtocolError(error.code));
        }
        let Some(result) = response.result().and_then(Value::as_object) else {
            return self.fail(NegotiationFailure::InvalidDiscovery);
        };
        if result.get("resultType").and_then(Value::as_str) != Some("complete")
            || !result.get("capabilities").is_some_and(Value::is_object)
        {
            return self.fail(NegotiationFailure::InvalidDiscovery);
        }
        let Some(versions) = result.get("supportedVersions").and_then(Value::as_array) else {
            return self.fail(NegotiationFailure::InvalidDiscovery);
        };
        if versions.iter().any(|version| !version.is_string()) {
            return self.fail(NegotiationFailure::InvalidDiscovery);
        }
        let modern = versions
            .iter()
            .any(|version| version.as_str() == Some(ProtocolVersion::Modern.as_str()));
        if modern {
            if status == HttpDiscoveryStatus::VersionError {
                return self.fail(NegotiationFailure::InvalidDiscovery);
            }
            return self.ready(ProtocolVersion::Modern);
        }
        self.fail(NegotiationFailure::UnsupportedVersion)
    }
}

fn supports_modern(error: &RpcProtocolError<'_>) -> bool {
    let Some(data) = error.data.and_then(Value::as_object) else {
        return false;
    };
    if data.get("requested").and_then(Value::as_str) != Some(ProtocolVersion::Modern.as_str()) {
        return false;
    }
    let Some(versions) = data.get("supported").and_then(Value::as_array) else {
        return false;
    };
    versions.iter().all(Value::is_string)
        && versions
            .iter()
            .any(|version| version.as_str() == Some(ProtocolVersion::Modern.as_str()))
}
