//! Exact HTTP/1.1 plaintext submission, independent of any networking executor.

use std::collections::BTreeSet;
use std::fmt;
use std::io::IoSlice;
use std::sync::Arc;
use std::task::{Context, Poll};

use machine_god_core::{BoxFuture, CancellationToken, PermissionInvocation, PermissionRequest};

use super::{
    Framing, McpSubmission, McpSubmissionError, McpSubmissionRegistry, McpSubmissionRuntime,
    McpSubmissionWriter, PreparedMcpSubmission, Result, WriteGuard,
};
use crate::mcp::endpoint::McpEndpoint;

const MAX_HEAD_BYTES: usize = 64 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_NAME_BYTES: usize = 128;
const MAX_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_VECTORS: usize = 64;
type OwnedHeader = (Box<str>, Box<[u8]>);

/// Immutable origin-form HTTP/1.1 POST head inputs. This is bounded request data,
/// not connection, DNS, TLS, authentication or submission authority. Trusted
/// native preparation supplies the endpoint and headers of the pinned runtime.
/// Generated framing fields cannot be replaced by explicit headers.
pub struct McpSubmissionHttpHead {
    endpoint: McpEndpoint,
    headers: Box<[OwnedHeader]>,
}
impl McpSubmissionHttpHead {
    /// Copies bounded explicit headers and an already syntax-admitted endpoint.
    /// Header names are canonical lowercase; values retain exact bytes,
    /// including HTAB and obs-text from explicitly resolved header inputs.
    ///
    /// # Errors
    /// Rejects case-insensitive duplicates, generated/hop-by-hop header names,
    /// prohibited controls, and the finite count/name/value/aggregate bounds.
    pub fn new(endpoint: &McpEndpoint, headers: &[(&str, &[u8])]) -> Result<Self> {
        if headers.len() > MAX_HEADERS {
            return Err(McpSubmissionError::Limit);
        }
        let mut names = BTreeSet::new();
        let mut owned = Vec::with_capacity(headers.len());
        let mut total = endpoint.as_str().len();
        for &(name, value) in headers {
            if name.len() > MAX_HEADER_NAME_BYTES || value.len() > MAX_HEADER_VALUE_BYTES {
                return Err(McpSubmissionError::Limit);
            }
            if name.is_empty()
                || !name.bytes().all(header_name_byte)
                || !value
                    .iter()
                    .all(|byte| *byte == b'\t' || (*byte >= b' ' && *byte != 0x7f))
            {
                return Err(McpSubmissionError::Invalid);
            }
            let name = name.to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "content-type"
                    | "accept"
                    | "connection"
                    | "transfer-encoding"
                    | "content-encoding"
                    | "trailer"
                    | "upgrade"
                    | "expect"
                    | "te"
                    | "proxy-connection"
                    | "proxy-authorization"
            ) || !names.insert(name.clone())
            {
                return Err(McpSubmissionError::Invalid);
            }
            total = total
                .checked_add(name.len() + value.len() + 4)
                .ok_or(McpSubmissionError::Limit)?;
            if total > MAX_HEAD_BYTES {
                return Err(McpSubmissionError::Limit);
            }
            owned.push((name.into_boxed_str(), value.into()));
        }
        Ok(Self {
            endpoint: endpoint.clone(),
            headers: owned.into_boxed_slice(),
        })
    }

    fn encode(&self, payload: &[u8]) -> Result<Box<[u8]>> {
        // Every append is charged before allocation. The endpoint and custom
        // fields were validated before ownership; no reparsing or ambient input.
        let mut bytes = Vec::new();
        append(&mut bytes, b"POST ")?;
        append(&mut bytes, self.endpoint.request_target().as_bytes())?;
        append(&mut bytes, b" HTTP/1.1\r\nhost: ")?;
        append(&mut bytes, self.endpoint.authority().as_bytes())?;
        append(&mut bytes, b"\r\ncontent-type: application/json\r\naccept: application/json, text/event-stream\r\ncontent-length: ")?;
        append(&mut bytes, payload.len().to_string().as_bytes())?;
        append(&mut bytes, b"\r\nconnection: close\r\n")?;
        for (name, value) in &self.headers {
            append(&mut bytes, name.as_bytes())?;
            append(&mut bytes, b": ")?;
            append(&mut bytes, value)?;
            append(&mut bytes, b"\r\n")?;
        }
        append(&mut bytes, b"\r\n")?;
        // Payload was bounded and semantically validated by copy_request.
        let mut framed = Vec::with_capacity(bytes.len() + payload.len());
        framed.extend_from_slice(&bytes);
        framed.extend_from_slice(payload);
        Ok(framed.into_boxed_slice())
    }
}
fn header_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}
fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    if value.len() > MAX_HEAD_BYTES - bytes.len() {
        return Err(McpSubmissionError::Limit);
    }
    bytes.extend_from_slice(value);
    Ok(())
}
impl fmt::Debug for McpSubmissionHttpHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpSubmissionHttpHead { <redacted> }")
    }
}

impl McpSubmissionRegistry {
    /// Prepares one exact fixed-length HTTP/1.1 POST before binding permission.
    /// Like `prepare`, construction only performs bounded copying/validation;
    /// the reservation is created on first poll. The retained HTTP bytes contain
    /// the admitted payload without NDJSON's LF. All endpoint/header/body bytes
    /// are immutable before core consumes the concrete native proof admission.
    ///
    /// # Errors
    /// Rejects the same identity/semantic/budget failures as `prepare`, plus
    /// excessive HTTP head framing. No arbitrary methods or chunking are admitted.
    pub fn prepare_http(
        self: &Arc<Self>,
        request: &PermissionRequest,
        invocation: PermissionInvocation<'_>,
        runtime: Arc<McpSubmissionRuntime>,
        head: &McpSubmissionHttpHead,
        payload: &[u8],
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<PreparedMcpSubmission>> {
        let copied = self
            .copy_request(request, invocation, &runtime, payload, &cancellation)
            .and_then(|mut copied| {
                // copy_request produces a validated payload and exactly one final LF.
                copied.wire = head.encode(&copied.wire[..copied.wire.len() - 1])?;
                copied.framing = Framing::Http;
                Ok(copied)
            });
        self.prepare_copied(runtime, copied, cancellation)
    }
}

impl McpSubmission {
    /// Consumes one HTTP-prepared submission into the dedicated connection's
    /// final plaintext writer wrapper ABOVE TLS. Construction does not poll or
    /// write. `writer` owns the exclusive connection/permit and must not defer
    /// plaintext delegation to a later unguarded task. No inner writer escapes.
    ///
    /// The connection must be HTTP/1.1, nonpooled, nonpipelined, and dedicated to
    /// this single bound request; network/TLS/destination admission is separate.
    /// This adapter is not a request-body producer or a pre-execute check.
    ///
    /// # Errors
    /// Consumes and rejects a non-HTTP prepared submission without writer work.
    pub fn into_http_driver<W: McpSubmissionWriter>(
        self,
        writer: W,
    ) -> Result<McpSubmissionHttpDriver<W>> {
        if self.ready.data.framing != Framing::Http {
            return Err(McpSubmissionError::Invalid);
        }
        Ok(McpSubmissionHttpDriver {
            guard: WriteGuard::new(self),
            writer,
            complete: false,
        })
    }
}

/// Non-clone single-request HTTP connection writer. Proposed plaintext must be
/// an exact prefix of the remaining pre-admitted bytes; neither a changed head,
/// body, repeated prefix, nor a trailing/pipelined request reaches the sink.
/// Every write/vector/flush poll keeps and revalidates the original concrete
/// proof, turn, runtime and cancellation. Errors/panics permanently close it.
///
/// Implement a networking library's plaintext IO trait by forwarding its final
/// scalar/vectored writes and flushes here. Place this wrapper above its TLS
/// stream, never on the encrypted side or merely around HTTP body polling.
pub struct McpSubmissionHttpDriver<W> {
    guard: WriteGuard,
    writer: W,
    complete: bool,
}
impl<W> fmt::Debug for McpSubmissionHttpDriver<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpSubmissionHttpDriver { <redacted> }")
    }
}
impl<W> McpSubmissionHttpDriver<W> {
    /// Exact immutable request data for the trusted native encoder only. This
    /// borrowed data is not a permission proof; it cannot authorize any write.
    #[must_use]
    pub(crate) fn request_bytes(&self) -> &[u8] {
        &self.guard.submission.ready.data.wire
    }
    /// Whether a sink write or flush was entered, including Pending/error/panic.
    #[must_use]
    pub const fn was_attempted(&self) -> bool {
        self.guard.submission.was_attempted()
    }
    /// Successfully acknowledged bytes, not a bound on ambiguous error effects.
    #[must_use]
    pub const fn acknowledged_bytes(&self) -> usize {
        self.guard.offset
    }
    /// All exact request bytes were accepted and the final flush succeeded.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    fn match_prefix(&mut self, chunks: &[IoSlice<'_>]) -> Result<usize> {
        let mut offset = self.guard.offset;
        if chunks.len() > MAX_VECTORS {
            self.guard.stop();
            return Err(McpSubmissionError::Limit);
        }
        for chunk in chunks {
            let expected = &self.request_bytes()[offset..];
            if !expected.starts_with(chunk) {
                self.guard.stop();
                return Err(McpSubmissionError::Denied);
            }
            offset += chunk.len();
        }
        Ok(offset - self.guard.offset)
    }
}
impl<W: McpSubmissionWriter> McpSubmissionHttpDriver<W> {
    /// Delegates one matching nonempty plaintext prefix synchronously.
    ///
    /// # Errors
    /// Returns a terminal redacted mismatch, authority, cancellation or writer
    /// failure. Even a writer error with zero acknowledged bytes is ambiguous.
    pub fn poll_write(&mut self, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<Result<usize>> {
        self.guard.poll_ready(cx)?;
        let offered = self.match_prefix(&[IoSlice::new(bytes)])?;
        if offered == 0 {
            return Poll::Ready(Ok(0));
        }
        self.guard.begin_delegate()?;
        let outcome = self.writer.poll_write(cx, bytes);
        self.guard.write_result(&outcome, offered)
    }

    /// Validates the entire offered vector before any synchronous sink call.
    /// Partial counts can cross slice boundaries; only the acknowledged prefix
    /// advances. Pending retries are revalidated and never restart that prefix.
    ///
    /// # Errors
    /// Rejects more than 64 vectors, changed bytes, stale authority or sink
    /// failure. No vector concatenation or unbounded allocation is performed.
    pub fn poll_write_vectored(
        &mut self,
        cx: &mut Context<'_>,
        bytes: &[IoSlice<'_>],
    ) -> Poll<Result<usize>> {
        self.guard.poll_ready(cx)?;
        let offered = self.match_prefix(bytes)?;
        if offered == 0 {
            return Poll::Ready(Ok(0));
        }
        self.guard.begin_delegate()?;
        let outcome = self.writer.poll_write_vectored(cx, bytes);
        self.guard.write_result(&outcome, offered)
    }

    /// Revalidates immediately before each flush, including Pending retries.
    /// Intermediate flushes do not reset the byte cursor. After the full request
    /// and final successful flush, all further writes/flushes are rejected.
    ///
    /// # Errors
    /// Revocation/cancellation, repeated completion and sink failures are terminal.
    pub fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.guard.poll_ready(cx)?;
        self.guard.begin_delegate()?;
        let outcome = self.writer.poll_flush(cx);
        let result = self.guard.flush_result(&outcome);
        if matches!(result, Poll::Ready(Ok(()))) && self.guard.offset == self.request_bytes().len()
        {
            self.complete = true;
        }
        result
    }
}
