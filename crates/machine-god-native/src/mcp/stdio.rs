//! Owned, bounded MCP stdin/stdout connections on Linux and macOS.
//!
//! Construction is inert. One collected worker owns each child and its pipes;
//! connection closure never closes the caller's unrelated worker scope. Wire
//! correlation, negotiation and catalog publication remain runtime duties.

use std::collections::VecDeque;
use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};
use std::time::Instant;

use futures_util::task::AtomicWaker;
use machine_god_core::{BoxFuture, CancellationToken};

use super::protocol::{RpcEnvelope, WireLimits, parse_envelope};
use super::submission::{McpSubmission, McpSubmissionRuntime};
use crate::{NativeOwnedWorkerCompletion, NativeOwnedWorkerScope};

mod control;
mod launch;
mod worker;
pub use control::McpStdioControl;
pub use launch::McpStdioLaunch;

/// Fixed maximum admitted outbound requests, including the active writer.
pub const MAX_MCP_STDIO_WRITES: usize = 8;
/// Fixed maximum queued inbound frames. A stalled consumer fails closed.
pub const MAX_MCP_STDIO_FRAMES: usize = 2;
/// Fixed maximum explicitly registered executable runtime allocations.
pub const MAX_MCP_STDIO_RUNTIMES: usize = super::pagination::McpCatalogKind::Tools.max_items();

/// Redacted failures; no command, environment or server bytes are retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpStdioError {
    Invalid,
    Capacity,
    Cancelled,
    Deadline,
    Closed,
    Process,
    Protocol,
    ForeignRuntime,
    Submission(super::submission::McpSubmissionError),
}
impl fmt::Display for McpStdioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP stdio operation failed")
    }
}
impl std::error::Error for McpStdioError {}
type Result<T> = std::result::Result<T, McpStdioError>;

/// Submission evidence, not a server response or execution-success claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpStdioWriteReceipt {
    pub outcome: Result<()>,
    /// True before entering the final writer, even when no bytes are acknowledged.
    pub attempted: bool,
    pub acknowledged_bytes: usize,
}

/// What the worker actually observed, not a negotiation or retry decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpStdioReadEnd {
    /// EOF was observed with an empty incremental decoder.
    CleanEof,
    /// EOF was observed with an unterminated frame.
    IncompleteEof,
    /// A bounded discovery-timeout snapshot observed `WouldBlock` with no frame
    /// or partial input. This is a cutoff observation, not an EOF guarantee.
    DiscoveryTimeoutQuiescent,
    /// No clean EOF proof: unread bytes may still contain a response.
    Unclassified,
}

/// Authoritative only after owned connection cleanup completes. Queued complete
/// frames must still be consumed and validated before interpreting close evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpStdioCloseObservation {
    pub reason: McpStdioError,
    pub read_end: McpStdioReadEnd,
    pub buffered_partial_frame: bool,
    pub unconsumed_complete_frames: usize,
}

/// One admitted frame with its original JSON bytes. Parsing is an observation,
/// not permission; callers independently bound retained frames and catalogs.
pub struct McpStdioFrame {
    bytes: Box<[u8]>,
    envelope: RpcEnvelope,
}
impl McpStdioFrame {
    /// Original JSON bytes, without the NDJSON line delimiter.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Bounded parsed envelope for routing, not lossless schema serialization.
    #[must_use]
    pub fn envelope(&self) -> &RpcEnvelope {
        &self.envelope
    }

    /// Discards the original bytes when only parsed routing data is needed.
    #[must_use]
    pub fn into_envelope(self) -> RpcEnvelope {
        self.envelope
    }
}
impl fmt::Debug for McpStdioFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpStdioFrame { <redacted> }")
    }
}

/// Non-clone connection owner. Drop cancels admission and the owned worker;
/// cleanup may remain enrolled after Drop until the child is positively reaped.
pub struct McpStdioConnection {
    shared: Arc<Shared>,
    completion: NativeOwnedWorkerCompletion,
}
/// Observation only; does not retain a connection or its worker ownership.
pub(crate) struct McpStdioConnectionReadiness(std::sync::Weak<Shared>);
impl McpStdioConnectionReadiness {
    pub(crate) fn is_ready(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|shared| shared.check().is_ok())
    }
}
impl fmt::Debug for McpStdioConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpStdioConnection { <redacted> }")
    }
}
impl Drop for McpStdioConnection {
    fn drop(&mut self) {
        self.close();
    }
}
impl McpStdioConnection {
    pub(crate) fn readiness(&self) -> McpStdioConnectionReadiness {
        McpStdioConnectionReadiness(Arc::downgrade(&self.shared))
    }
    #[cfg(test)]
    pub(crate) fn inert_for_test() -> Self {
        let scope = NativeOwnedWorkerScope::new();
        let completion = scope.completion();
        scope.close();
        Self {
            shared: Arc::new(Shared::new(WireLimits::default(), CancellationToken::new())),
            completion,
        }
    }

    /// Stops this connection only. The completion observer includes deferred reap.
    pub fn close(&self) {
        self.shared.stop.cancel();
    }

    /// Freezes new writes and asks the owned worker to classify discovery input
    /// before cleanup. Only settled `close_observation` can establish quiescence;
    /// calling this method alone grants no fallback or retry permission.
    pub fn close_after_discovery_timeout(&self) {
        let _state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.shared.discovery_timeout.store(true, Ordering::Release);
    }

    /// Observation does not extend process authority or keep the connection open.
    #[must_use]
    pub fn completion(&self) -> NativeOwnedWorkerCompletion {
        self.completion.clone()
    }
    /// Exact immutable launch admission bounds, not execution authority.
    #[must_use]
    pub fn wire_limits(&self) -> WireLimits {
        self.shared.limits
    }

    /// Returns settled evidence only after worker join and retained child cleanup.
    /// Cancellation or a dropped receive never manufactures clean EOF evidence.
    #[must_use]
    pub fn close_observation(&self) -> Option<McpStdioCloseObservation> {
        if !self.completion.is_complete() {
            return None;
        }
        let state = self.shared.state.lock().ok()?;
        Some(McpStdioCloseObservation {
            reason: state.closed?,
            read_end: state.read_end.unwrap_or(McpStdioReadEnd::Unclassified),
            buffered_partial_frame: state.buffered_partial_frame,
            unconsumed_complete_frames: state.frames.len(),
        })
    }

    /// Native composition registers exact executable allocations after catalog
    /// admission. Names and equal numeric generations cannot authorize a route.
    /// Replacing the set also invalidates queued foreign submissions.
    ///
    /// # Errors
    /// Rejects excessive allocations or a closed connection.
    pub fn admit_runtimes(&self, runtimes: Vec<Arc<McpSubmissionRuntime>>) -> Result<()> {
        if runtimes.len() > MAX_MCP_STDIO_RUNTIMES {
            return Err(McpStdioError::Capacity);
        }
        self.shared.check()?;
        let previous = {
            let mut admitted = self
                .shared
                .runtimes
                .lock()
                .map_err(|_| McpStdioError::Closed)?;
            std::mem::replace(&mut *admitted, runtimes.into_boxed_slice())
        };
        drop(previous);
        Ok(())
    }

    /// Enqueues on first poll only; a full queue rejects without starting a write.
    /// The one-shot proof stays owned through queue waits and final pipe writes.
    /// Abandonment cancels this request, never replays it, and closes the transport
    /// if submission might have begun. The deadline covers queue and writer waits.
    ///
    /// # Errors
    /// Returns bounded admission errors. After admission, the receipt separately
    /// reports submission failure and ambiguous/acknowledged byte evidence.
    #[must_use]
    pub fn submit(
        &self,
        submission: McpSubmission,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<McpStdioWriteReceipt>> {
        self.enqueue(Payload::Tool(submission), deadline)
    }

    /// Submits strictly typed startup/discovery/control data under the explicitly
    /// owned connection. Never admits tool or application feature requests.
    ///
    /// # Errors
    /// Uses the same bounded queue, deadline and receipt rules as `submit`.
    #[must_use]
    pub fn control(
        &self,
        control: McpStdioControl,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<McpStdioWriteReceipt>> {
        self.enqueue(Payload::Control(control), deadline)
    }

    fn enqueue(
        &self,
        payload: Payload,
        deadline: Instant,
    ) -> BoxFuture<'static, Result<McpStdioWriteReceipt>> {
        let shared = self.shared.clone();
        Box::pin(async move {
            shared.check()?;
            if Instant::now() >= deadline {
                return Err(McpStdioError::Deadline);
            }
            if let Payload::Tool(submission) = &payload {
                shared.affinity(submission)?;
                if poll_fn(|cx| Poll::Ready(submission.cancelled().as_mut().poll(cx).is_ready()))
                    .await
                {
                    return Err(McpStdioError::Cancelled);
                }
            }
            let response = Arc::new(Response::new());
            let cancel = CancellationToken::new();
            let _abandon = CancelOnDrop(Some(cancel.clone()));
            {
                let mut state = shared.state.lock().map_err(|_| McpStdioError::Closed)?;
                if let Some(error) = state.closed {
                    return Err(error);
                }
                if shared.stop.is_cancelled() {
                    return Err(McpStdioError::Cancelled);
                }
                if shared.discovery_timeout.load(Ordering::Acquire) {
                    return Err(McpStdioError::Deadline);
                }
                if state.admitted >= MAX_MCP_STDIO_WRITES {
                    return Err(McpStdioError::Capacity);
                }
                state.admitted += 1;
                state.queue.push_back(Queued {
                    payload,
                    deadline,
                    cancel,
                    response: response.clone(),
                });
            }
            response.wait().await
        })
    }

    /// Receives one complete, admitted JSON-RPC envelope. Only one live receiver
    /// is admitted; wire IDs and full method result shapes must be checked by the
    /// runtime. No null-ID correlation exception is applied by this transport.
    ///
    /// # Errors
    /// Rejects concurrent receivers, malformed frames, EOF and closed connections.
    #[must_use]
    pub fn receive(&self) -> BoxFuture<'static, Result<RpcEnvelope>> {
        let frame = self.receive_frame();
        Box::pin(async move { frame.await.map(McpStdioFrame::into_envelope) })
    }

    /// Receives the same validated envelope together with original bounded JSON
    /// bytes. Use this for catalog/schema admission without number normalization.
    /// Shares the single receiving lane and closure rules with `receive`.
    ///
    /// # Errors
    /// Rejects a concurrent receiver, malformed framing/JSON, or a closed queue.
    #[must_use]
    pub fn receive_frame(&self) -> BoxFuture<'static, Result<McpStdioFrame>> {
        let shared = self.shared.clone();
        Box::pin(async move {
            struct Receiving(Arc<Shared>);
            impl Drop for Receiving {
                fn drop(&mut self) {
                    // Remove stale caller custody before another receiver can
                    // register; arbitrary waker destruction runs outside locks.
                    drop(self.0.reader.take());
                    self.0.receiving.store(false, Ordering::Release);
                }
            }
            if shared.receiving.swap(true, Ordering::AcqRel) {
                return Err(McpStdioError::Capacity);
            }
            let _receiving = Receiving(shared.clone());
            let frame = poll_fn(|cx| {
                shared.reader.register(cx.waker());
                let Ok(mut state) = shared.state.lock() else {
                    return Poll::Ready(Err(McpStdioError::Closed));
                };
                if let Some(frame) = state.frames.pop_front() {
                    return Poll::Ready(Ok(frame));
                }
                state
                    .closed
                    .map_or(Poll::Pending, |error| Poll::Ready(Err(error)))
            })
            .await?;
            let envelope = parse_envelope(&frame, shared.limits).map_err(|_| {
                {
                    let mut state = shared
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.closed = Some(McpStdioError::Protocol);
                }
                shared.stop.cancel();
                shared.finish(McpStdioError::Protocol);
                McpStdioError::Protocol
            })?;
            Ok(McpStdioFrame {
                bytes: frame.into_boxed_slice(),
                envelope,
            })
        })
    }
}

struct Shared {
    state: Mutex<State>,
    runtimes: Mutex<Box<[Arc<McpSubmissionRuntime>]>>,
    reader: AtomicWaker,
    receiving: AtomicBool,
    discovery_timeout: AtomicBool,
    stop: CancellationToken,
    limits: WireLimits,
}
impl Shared {
    fn new(limits: WireLimits, stop: CancellationToken) -> Self {
        Self {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                frames: VecDeque::new(),
                admitted: 0,
                closed: None,
                read_end: None,
                buffered_partial_frame: false,
            }),
            runtimes: Mutex::new(Box::new([])),
            reader: AtomicWaker::new(),
            receiving: AtomicBool::new(false),
            discovery_timeout: AtomicBool::new(false),
            stop,
            limits,
        }
    }
    fn check(&self) -> Result<()> {
        if self.stop.is_cancelled() {
            return Err(McpStdioError::Cancelled);
        }
        if self.discovery_timeout.load(Ordering::Acquire) {
            return Err(McpStdioError::Deadline);
        }
        self.state
            .lock()
            .map_err(|_| McpStdioError::Closed)?
            .closed
            .map_or(Ok(()), Err)
    }
    fn affinity(&self, submission: &McpSubmission) -> Result<()> {
        if self
            .runtimes
            .lock()
            .map_err(|_| McpStdioError::Closed)?
            .iter()
            .any(|runtime| submission.belongs_to_runtime(runtime))
        {
            Ok(())
        } else {
            Err(McpStdioError::ForeignRuntime)
        }
    }
    fn finish(&self, error: McpStdioError) {
        let queued = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed.is_none() {
                state.closed = Some(error);
            }
            state.admitted = 0;
            std::mem::take(&mut state.queue)
        };
        for queued in queued {
            queued.response.complete(Ok(failed(error)));
        }
        self.reader.wake();
    }
}
struct State {
    queue: VecDeque<Queued>,
    frames: VecDeque<Vec<u8>>,
    admitted: usize,
    closed: Option<McpStdioError>,
    read_end: Option<McpStdioReadEnd>,
    buffered_partial_frame: bool,
}
enum Payload {
    Tool(McpSubmission),
    Control(McpStdioControl),
}
struct Queued {
    payload: Payload,
    deadline: Instant,
    cancel: CancellationToken,
    response: Arc<Response<McpStdioWriteReceipt>>,
}
struct Response<T> {
    value: Mutex<Option<Result<T>>>,
    waker: AtomicWaker,
    completed: AtomicBool,
}
impl<T> Response<T> {
    fn new() -> Self {
        Self {
            value: Mutex::new(None),
            waker: AtomicWaker::new(),
            completed: AtomicBool::new(false),
        }
    }
    fn complete(&self, value: Result<T>) {
        if self.completed.swap(true, Ordering::AcqRel) {
            return;
        }
        let previous = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(value);
        drop(previous);
        self.waker.wake();
    }
    fn complete_if_empty(&self, value: Result<T>) {
        self.complete(value);
    }
    async fn wait(&self) -> Result<T> {
        poll_fn(|cx| {
            self.waker.register(cx.waker());
            self.value
                .lock()
                .map_or(Poll::Ready(Err(McpStdioError::Closed)), |mut value| {
                    value.take().map_or(Poll::Pending, Poll::Ready)
                })
        })
        .await
    }
}
struct CancelOnDrop(Option<CancellationToken>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel();
        }
    }
}
fn failed(error: McpStdioError) -> McpStdioWriteReceipt {
    McpStdioWriteReceipt {
        outcome: Err(error),
        attempted: false,
        acknowledged_bytes: 0,
    }
}

#[cfg(test)]
mod tests;
