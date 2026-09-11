use serde_json::Value;

use super::{RpcEnvelope, RpcId, RpcProtocolError, WireError};

/// Configured transport families are distinct, not an automatic fallback chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportKind {
    /// Owned process with incremental NDJSON input/output.
    Stdio,
    /// Request-scoped modern HTTP or negotiated legacy Streamable HTTP.
    StreamableHttp,
    /// Explicit deprecated HTTP+SSE, never selected by HTTP fallback.
    LegacySse,
}

/// Protocol versions supported by the exact upstream pin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolVersion {
    /// Stateless modern discovery and request envelopes, 2026-07-28.
    Modern,
    /// Legacy stdio or Streamable HTTP, 2025-11-25.
    Legacy20251125,
    /// Legacy stdio or Streamable HTTP, 2025-06-18.
    Legacy20250618,
    /// Legacy Streamable HTTP only, 2025-03-26.
    Legacy20250326,
    /// Oldest legacy stdio or explicitly configured HTTP+SSE, 2024-11-05.
    Legacy20241105,
}

impl ProtocolVersion {
    /// Exact wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Modern => "2026-07-28",
            Self::Legacy20251125 => "2025-11-25",
            Self::Legacy20250618 => "2025-06-18",
            Self::Legacy20250326 => "2025-03-26",
            Self::Legacy20241105 => "2024-11-05",
        }
    }

    /// Parse only versions supported by the selected transport.
    #[must_use]
    pub fn parse_for(transport: TransportKind, text: &str) -> Option<Self> {
        [
            Self::Modern,
            Self::Legacy20251125,
            Self::Legacy20250618,
            Self::Legacy20250326,
            Self::Legacy20241105,
        ]
        .into_iter()
        .find(|version| version.as_str() == text && version.supported_by(transport))
    }

    const fn supported_by(self, transport: TransportKind) -> bool {
        match transport {
            TransportKind::Stdio => !matches!(self, Self::Legacy20250326),
            TransportKind::StreamableHttp => !matches!(self, Self::Legacy20241105),
            TransportKind::LegacySse => matches!(self, Self::Legacy20241105),
        }
    }

    const fn older_stdio(self) -> Option<Self> {
        match self {
            Self::Legacy20251125 => Some(Self::Legacy20250618),
            Self::Legacy20250618 => Some(Self::Legacy20241105),
            _ => None,
        }
    }

    fn is_older_stdio_than(self, offered: Self) -> bool {
        let mut next = offered.older_stdio();
        while let Some(version) = next {
            if version == self {
                return true;
            }
            next = version.older_stdio();
        }
        false
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
    /// Legacy initialization must be followed by `notifications/initialized`.
    #[must_use]
    pub const fn needs_initialized_notification(self) -> bool {
        !matches!(self.version, ProtocolVersion::Modern)
    }
    /// Legacy 2025-03 HTTP omits the protocol header; deprecated SSE is separate.
    #[must_use]
    pub const fn sends_http_protocol_header(self) -> bool {
        matches!(self.transport, TransportKind::StreamableHttp)
            && !matches!(self.version, ProtocolVersion::Legacy20250326)
    }
    /// Only legacy 2025-11 HTTP permits the pinned empty priming/poll-close rule.
    #[must_use]
    pub const fn allows_legacy_http_poll_close(self) -> bool {
        matches!(self.transport, TransportKind::StreamableHttp)
            && matches!(self.version, ProtocolVersion::Legacy20251125)
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
    /// A well-formed error which does not authorize fallback; code only.
    ProtocolError(i64),
    /// Observation cannot occur in this transport/state combination.
    WrongState,
    /// Host control explicitly cancelled startup.
    Cancelled,
    /// Host startup/overall deadline expired without eligible fallback.
    Deadline,
    /// Non-negotiable transport failure.
    Transport,
}

/// Explicit startup action, never an application-request retry authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NegotiationAction {
    /// Issue modern `server/discover`, with a newly allocated request ID.
    SendDiscover,
    /// Issue legacy initialize on the current HTTP/SSE transport.
    Initialize(ProtocolVersion),
    /// Retire/reap the old stdio connection before creating its replacement.
    RestartInitialize(ProtocolVersion),
    /// Protocol accepted, but the host must still validate the full result and
    /// send the legacy initialized notification before publishing its catalog.
    Ready(NegotiatedProtocol),
    /// No additional negotiation I/O may follow on this machine.
    Failed(NegotiationFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Discover { retried: bool },
    Initialize(ProtocolVersion),
    Ready,
    Failed,
}

/// Finite startup selection. The host owns IDs, deadlines and connection epochs.
///
/// The maximum stdio path is one discover plus three strictly descending legacy
/// offers. HTTP permits one explicitly signaled modern retry and one legacy
/// initialize. Deprecated SSE starts directly at 2024-11-05. This state machine
/// cannot accept or replay ordinary MCP operations.
#[derive(Debug)]
pub struct Negotiation {
    transport: TransportKind,
    state: State,
}

impl Negotiation {
    /// Construct an inert machine and obtain its first startup action.
    #[must_use]
    pub const fn new(transport: TransportKind) -> (Self, NegotiationAction) {
        let (state, action) = if matches!(transport, TransportKind::LegacySse) {
            (
                State::Initialize(ProtocolVersion::Legacy20241105),
                NegotiationAction::Initialize(ProtocolVersion::Legacy20241105),
            )
        } else {
            (
                State::Discover { retried: false },
                NegotiationAction::SendDiscover,
            )
        };
        (Self { transport, state }, action)
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
            State::Initialize(offered) if status == HttpDiscoveryStatus::Ordinary => {
                self.initialize(response, offered)
            }
            _ => self.fail(NegotiationFailure::WrongState),
        }
    }

    /// Admit initial HTTP 404/405 discovery mismatch only.
    ///
    /// Other statuses, redirects, content-encoding errors and authentication
    /// failures are not downgrade evidence. Pass their failure to [`Self::abort`].
    pub fn http_discovery_mismatch(&mut self, status: u16) -> NegotiationAction {
        if self.transport == TransportKind::StreamableHttp
            && matches!(self.state, State::Discover { retried: false })
            && matches!(status, 404 | 405)
        {
            self.begin_legacy(ProtocolVersion::Legacy20251125)
        } else {
            self.fail(NegotiationFailure::WrongState)
        }
    }

    /// Explicit pinned stdio discovery timeout/clean-EOF compatibility path.
    ///
    /// The caller must first establish that overall control is still live and
    /// no partial/malformed frame was observed. This is not valid for cancellation,
    /// overall deadline, initialize timeout, partial writes or application calls.
    pub fn stdio_discovery_unavailable(&mut self) -> NegotiationAction {
        if self.transport == TransportKind::Stdio
            && matches!(self.state, State::Discover { retried: false })
        {
            self.begin_legacy(ProtocolVersion::Legacy20241105)
        } else {
            self.fail(NegotiationFailure::WrongState)
        }
    }

    /// Clean EOF while waiting for stdio initialize permits one downward offer.
    /// An initialize timeout does not: report it through [`Self::abort`].
    pub fn stdio_initialize_closed(&mut self) -> NegotiationAction {
        if let (TransportKind::Stdio, State::Initialize(offered)) = (self.transport, self.state) {
            self.retry_legacy(offered, None)
        } else {
            self.fail(NegotiationFailure::WrongState)
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

    fn begin_legacy(&mut self, version: ProtocolVersion) -> NegotiationAction {
        self.state = State::Initialize(version);
        if self.transport == TransportKind::Stdio {
            NegotiationAction::RestartInitialize(version)
        } else {
            NegotiationAction::Initialize(version)
        }
    }

    fn retry_legacy(
        &mut self,
        offered: ProtocolVersion,
        hint: Option<ProtocolVersion>,
    ) -> NegotiationAction {
        let next = match hint {
            Some(version) if version.is_older_stdio_than(offered) => Some(version),
            Some(_) => None,
            None => offered.older_stdio(),
        };
        match next {
            Some(version) => self.begin_legacy(version),
            None => self.fail(NegotiationFailure::UnsupportedVersion),
        }
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
            if self.transport == TransportKind::StreamableHttp {
                if error.code == -32022
                    && supports(&error, ProtocolVersion::Modern, ProtocolVersion::Modern)
                {
                    if status == HttpDiscoveryStatus::VersionError && !retried {
                        self.state = State::Discover { retried: true };
                        return NegotiationAction::SendDiscover;
                    }
                    return self.fail(NegotiationFailure::UnsupportedVersion);
                }
                if retried && status != HttpDiscoveryStatus::Ordinary {
                    return self.fail(NegotiationFailure::WrongState);
                }
                return self.begin_legacy(ProtocolVersion::Legacy20251125);
            }
            if error.code == -32021 {
                return self.fail(NegotiationFailure::ProtocolError(error.code));
            }
            if error.code != -32022 {
                return self.begin_legacy(ProtocolVersion::Legacy20251125);
            }
            return match legacy_hint(&error, ProtocolVersion::Modern) {
                Some(version) => self.begin_legacy(version),
                None => self.fail(NegotiationFailure::ProtocolError(error.code)),
            };
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
        // Pinned HTTP reuses the stdio discovery classifier before selecting
        // its own preferred initialize version, not the stdio selected version.
        let legacy = [
            ProtocolVersion::Legacy20251125,
            ProtocolVersion::Legacy20250618,
            ProtocolVersion::Legacy20241105,
        ]
        .into_iter()
        .find(|candidate| {
            versions
                .iter()
                .any(|value| value.as_str() == Some(candidate.as_str()))
        });
        match legacy {
            Some(version) => {
                self.begin_legacy(if self.transport == TransportKind::StreamableHttp {
                    ProtocolVersion::Legacy20251125
                } else {
                    version
                })
            }
            None => self.fail(NegotiationFailure::UnsupportedVersion),
        }
    }

    fn initialize(
        &mut self,
        response: &RpcEnvelope,
        offered: ProtocolVersion,
    ) -> NegotiationAction {
        if let Some(error) = response.protocol_error() {
            if self.transport != TransportKind::Stdio {
                return self.fail(NegotiationFailure::ProtocolError(error.code));
            }
            let hint = legacy_hint(&error, offered);
            return if error.code == -32022 || (error.code == -32602 && hint.is_some()) {
                self.retry_legacy(offered, hint)
            } else {
                self.fail(NegotiationFailure::ProtocolError(error.code))
            };
        }
        let version = response
            .result()
            .and_then(Value::as_object)
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str)
            .and_then(|text| ProtocolVersion::parse_for(self.transport, text));
        match version {
            Some(version) if version != ProtocolVersion::Modern => self.ready(version),
            _ => self.fail(NegotiationFailure::UnsupportedVersion),
        }
    }
}

fn legacy_hint(
    error: &RpcProtocolError<'_>,
    requested: ProtocolVersion,
) -> Option<ProtocolVersion> {
    [
        ProtocolVersion::Legacy20251125,
        ProtocolVersion::Legacy20250618,
        ProtocolVersion::Legacy20241105,
    ]
    .into_iter()
    .find(|candidate| supports(error, requested, *candidate))
}

fn supports(
    error: &RpcProtocolError<'_>,
    requested: ProtocolVersion,
    candidate: ProtocolVersion,
) -> bool {
    let Some(data) = error.data.and_then(Value::as_object) else {
        return false;
    };
    if data.get("requested").and_then(Value::as_str) != Some(requested.as_str()) {
        return false;
    }
    let Some(versions) = data.get("supported").and_then(Value::as_array) else {
        return false;
    };
    versions.iter().all(Value::is_string)
        && versions
            .iter()
            .any(|version| version.as_str() == Some(candidate.as_str()))
}
