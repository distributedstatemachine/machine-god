//! Bounded, exact-turn preparation and one-shot MCP tool submission.
//!
//! This module has no transport authority. Trusted native composition prepares
//! a `tools/call` request, binds the concrete native policy proof, and lets core
//! consume its admission before the execution future can claim it. The writer
//! must obtain its queue permit before calling [`McpSubmission::into_writer`].

use std::collections::BTreeMap;
use std::fmt;
use std::future::{Future, poll_fn};
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};

use machine_god_core::{
    BoxFuture, CancellationToken, Cancelled, Capability, PermissionError,
    PermissionExecutionAdmission, PermissionInvocation, PermissionRequest, PermissionRequestId,
    Session, SessionId, SessionIncarnationId, ToolCallId, ToolContext, ToolName, Turn, TurnHandle,
};
use serde::Serialize;
use serde_json::Value;

use super::protocol::{RpcId, RpcKind, WireLimits, parse_envelope};
use crate::NativePermissionExecutionProof;

mod runtime;
pub use runtime::{McpSubmissionRuntime, McpSubmissionRuntimeBinding, McpSubmissionRuntimeOwner};

/// Maximum reserved, ready and claimed call identities in one registered turn.
pub const MAX_MCP_SUBMISSION_SLOTS: usize = 64;
/// Maximum canonical arguments of a single invocation.
pub const MAX_MCP_SUBMISSION_ARGUMENT_BYTES: usize = 64 * 1024;
/// Maximum exact prepared JSON-RPC request bytes (without transport framing).
pub const MAX_MCP_SUBMISSION_REQUEST_BYTES: usize = 128 * 1024;
/// Maximum aggregate immutable runtime binding bytes.
pub const MAX_MCP_SUBMISSION_BINDING_BYTES: usize = 1024 * 1024;
const MAX_ARGUMENT_NODES: usize = 4096;
const MAX_ARGUMENT_DEPTH: usize = 64;

/// Fixed, redacted submission failures. None implies that a delegated write
/// transferred zero bytes; consult [`McpSubmissionWrite::was_attempted`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpSubmissionError {
    Invalid,
    Unavailable,
    Denied,
    Cancelled,
    Duplicate,
    Limit,
    GenerationExhausted,
    AlreadyAttempted,
    WriterFailed,
}
impl fmt::Display for McpSubmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MCP submission rejected")
    }
}
impl std::error::Error for McpSubmissionError {}
type Result<T> = std::result::Result<T, McpSubmissionError>;

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($ty), " { <redacted> }"))
            }
        }
    )+};
}

struct State {
    next_generation: Option<u64>,
    slots: BTreeMap<ToolCallId, Slot>,
}
enum Slot {
    Reserved {
        generation: u64,
        request: PermissionRequestId,
    },
    Ready(Box<Ready>),
    Claimed(PermissionRequestId),
}
impl Slot {
    fn request_id(&self) -> &PermissionRequestId {
        match self {
            Self::Reserved { request, .. } | Self::Claimed(request) => request,
            Self::Ready(ready) => &ready.data.permission.id,
        }
    }
}
struct Ready {
    data: Data,
    proof: NativePermissionExecutionProof,
}
struct Data {
    permission: PermissionRequest,
    tool: ToolName,
    arguments: Box<[u8]>,
    wire: Box<[u8]>,
    rpc_id: RpcId,
    runtime: Arc<McpSubmissionRuntime>,
    cancellation: CancellationToken,
    reservation: Reservation,
}
struct Reservation {
    registry: Weak<McpSubmissionRegistry>,
    call: ToolCallId,
    generation: u64,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            let removed = {
                let mut state = registry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if matches!(state.slots.get(&self.call), Some(Slot::Reserved { generation, .. }) if *generation == self.generation)
                {
                    state.slots.remove(&self.call)
                } else {
                    None
                }
            };
            drop(removed);
        }
    }
}

/// Exact live-turn registry. IDs are routing, not permission. Trusted native
/// composition retains exactly one registration for a turn and closes it before
/// retiring the native turn. No global or current-session fallback exists.
pub struct McpSubmissionRegistry {
    session: SessionId,
    incarnation: SessionIncarnationId,
    handle: TurnHandle,
    open: AtomicBool,
    cancellation: CancellationToken,
    state: Mutex<State>,
}

/// Non-clone native scope owner. Drop invalidates first, clears every slot and
/// tombstone, then cancels queue waiters and drops proofs outside the mutex.
pub struct McpSubmissionTurnRegistration {
    registry: Arc<McpSubmissionRegistry>,
}
impl Drop for McpSubmissionTurnRegistration {
    fn drop(&mut self) {
        self.registry.close();
    }
}

impl McpSubmissionRegistry {
    /// Registers an explicitly supplied session and live turn; performs no I/O.
    /// The returned guard must be retained by the native turn owner, not a tool.
    ///
    /// # Errors
    /// Rejects a foreign session/incarnation or already cancelled turn.
    pub fn register_turn(
        session: &Session,
        turn: &Turn,
    ) -> Result<(Arc<Self>, McpSubmissionTurnRegistration)> {
        if turn.session_id() != &session.id()
            || turn.session_incarnation_id() != &session.incarnation_id()
            || turn.handle().is_cancelled()
        {
            return Err(McpSubmissionError::Unavailable);
        }
        let registry = Arc::new(Self {
            session: session.id(),
            incarnation: session.incarnation_id(),
            handle: turn.handle(),
            open: AtomicBool::new(true),
            cancellation: CancellationToken::new(),
            state: Mutex::new(State {
                next_generation: Some(1),
                slots: BTreeMap::new(),
            }),
        });
        Ok((registry.clone(), McpSubmissionTurnRegistration { registry }))
    }

    fn live(&self) -> Result<()> {
        if !self.open.load(Ordering::Acquire) || self.handle.is_cancelled() {
            Err(McpSubmissionError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn close(&self) {
        self.open.store(false, Ordering::Release);
        let removed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut state.slots)
        };
        self.cancellation.cancel();
        drop(removed);
    }

    /// Bounded copying/validation happens at construction; reservation happens
    /// only on first poll. Dropping an unpolled or pre-cancelled future reserves
    /// nothing. Only an exact `Capability::Tool` and matching `tools/call` wire
    /// request are accepted. Startup/control messages use separate authority.
    ///
    /// # Errors
    /// Rejects mismatches, closed/retired/cancelled authority, duplicate live
    /// calls or permission IDs, and finite resource/generation exhaustion.
    /// The payload excludes framing and literal CR/LF; the owned direct writer
    /// submits the exact bytes plus exactly one final LF.
    pub fn prepare(
        self: &Arc<Self>,
        request: &PermissionRequest,
        invocation: PermissionInvocation<'_>,
        runtime: Arc<McpSubmissionRuntime>,
        wire: &[u8],
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<PreparedMcpSubmission>> {
        let copied = self.copy_request(request, invocation, &runtime, wire, &cancellation);
        let registry = self.clone();
        Box::pin(async move {
            check(&cancellation)?;
            registry.live()?;
            runtime.live()?;
            let CopiedRequest {
                permission,
                tool,
                call,
                arguments,
                wire,
                rpc_id,
            } = copied?;
            let generation = {
                let mut state = registry
                    .state
                    .lock()
                    .map_err(|_| McpSubmissionError::Unavailable)?;
                registry.live()?;
                if state.slots.contains_key(&call)
                    || state
                        .slots
                        .values()
                        .any(|slot| slot.request_id() == &permission.id)
                {
                    return Err(McpSubmissionError::Duplicate);
                }
                if state.slots.len() >= MAX_MCP_SUBMISSION_SLOTS {
                    return Err(McpSubmissionError::Limit);
                }
                let generation = next_generation(&mut state.next_generation)?;
                state.slots.insert(
                    call.clone(),
                    Slot::Reserved {
                        generation,
                        request: permission.id.clone(),
                    },
                );
                generation
            };
            Ok(PreparedMcpSubmission {
                data: Data {
                    permission,
                    tool,
                    arguments,
                    wire,
                    rpc_id,
                    runtime,
                    cancellation,
                    reservation: Reservation {
                        registry: Arc::downgrade(&registry),
                        call,
                        generation,
                    },
                },
            })
        })
    }

    /// Captures a ready generation at future construction. Missing stays
    /// missing; later admission or replacement cannot repair an old future.
    /// The first poll claims exactly once, leaving a turn-lifetime tombstone.
    ///
    /// # Errors
    /// Rejects missing/stale tickets and every invocation/runtime mismatch.
    pub fn claim(
        self: &Arc<Self>,
        context: ToolContext,
        tool: &ToolName,
        arguments: &Value,
        runtime: Arc<McpSubmissionRuntime>,
        cancellation: CancellationToken,
    ) -> BoxFuture<'static, Result<McpSubmission>> {
        let ticket = self.ticket(&context, tool, arguments, &runtime);
        let registry = self.clone();
        Box::pin(async move {
            check(&cancellation)?;
            registry.live()?;
            runtime.live()?;
            let generation = ticket?;
            let removed = {
                let mut state = registry
                    .state
                    .lock()
                    .map_err(|_| McpSubmissionError::Unavailable)?;
                registry.live()?;
                if !matches!(state.slots.get(&context.call_id), Some(Slot::Ready(ready)) if ready.data.reservation.generation == generation)
                {
                    return Err(McpSubmissionError::Denied);
                }
                let request = state
                    .slots
                    .get(&context.call_id)
                    .ok_or(McpSubmissionError::Denied)?
                    .request_id()
                    .clone();
                state.slots.insert(context.call_id, Slot::Claimed(request))
            };
            let Some(Slot::Ready(ready)) = removed else {
                return Err(McpSubmissionError::Denied);
            };
            let submission = McpSubmission {
                registry,
                ready,
                cancellation,
                attempted: false,
            };
            submission.checkpoint()?;
            Ok(submission)
        })
    }

    fn ticket(
        &self,
        context: &ToolContext,
        tool: &ToolName,
        arguments: &Value,
        runtime: &Arc<McpSubmissionRuntime>,
    ) -> Result<u64> {
        self.live()?;
        if context.session_id != self.session
            || context.session_incarnation_id != self.incarnation
            || &context.turn_id != self.handle.id()
        {
            return Err(McpSubmissionError::Denied);
        }
        let arguments = canonical_arguments(arguments)?;
        let state = self
            .state
            .lock()
            .map_err(|_| McpSubmissionError::Unavailable)?;
        let Some(Slot::Ready(ready)) = state.slots.get(&context.call_id) else {
            return Err(McpSubmissionError::Denied);
        };
        if ready.data.tool != *tool
            || ready.data.arguments != arguments
            || !Arc::ptr_eq(&ready.data.runtime, runtime)
        {
            return Err(McpSubmissionError::Denied);
        }
        Ok(ready.data.reservation.generation)
    }

    fn copy_request(
        &self,
        request: &PermissionRequest,
        invocation: PermissionInvocation<'_>,
        runtime: &McpSubmissionRuntime,
        wire: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<CopiedRequest> {
        check(cancellation)?;
        self.live()?;
        runtime.live()?;
        if request.session_id != self.session
            || request.session_incarnation_id != self.incarnation
            || &request.turn_id != self.handle.id()
            || invocation.tool_name != runtime.binding.tool_name()
        {
            return Err(McpSubmissionError::Denied);
        }
        let Capability::Tool {
            name,
            call_id,
            arguments,
        } = &request.capability
        else {
            return Err(McpSubmissionError::Invalid);
        };
        let canonical = canonical_arguments(invocation.arguments)?;
        if name != invocation.tool_name
            || call_id != invocation.call_id
            || canonical_arguments(arguments)? != canonical
        {
            return Err(McpSubmissionError::Denied);
        }
        // Bound every copied permission field, including diagnostic reason.
        if request.reason.len() > MAX_MCP_SUBMISSION_REQUEST_BYTES {
            return Err(McpSubmissionError::Limit);
        }
        bounded_json(request, MAX_MCP_SUBMISSION_REQUEST_BYTES)?;
        if wire.len() > MAX_MCP_SUBMISSION_REQUEST_BYTES {
            return Err(McpSubmissionError::Limit);
        }
        // A prepared payload excludes NDJSON framing; this boundary alone adds
        // exactly one final LF and never permits internal literal CR/LF bytes.
        if wire.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            return Err(McpSubmissionError::Invalid);
        }
        let envelope = parse_envelope(
            wire,
            WireLimits {
                max_frame_bytes: MAX_MCP_SUBMISSION_REQUEST_BYTES,
                max_depth: 64,
                max_nodes: 8192,
            },
        )
        .map_err(|_| McpSubmissionError::Invalid)?;
        let params = envelope
            .params()
            .and_then(Value::as_object)
            .ok_or(McpSubmissionError::Invalid)?;
        if envelope.kind() != RpcKind::Request
            || envelope.method() != Some("tools/call")
            || envelope
                .value()
                .as_object()
                .is_none_or(|value| value.len() != 4)
            || params.len() != 2
            || params.get("name").and_then(Value::as_str) != Some(runtime.binding.remote_tool())
            || canonical_arguments(params.get("arguments").ok_or(McpSubmissionError::Invalid)?)?
                != canonical
        {
            return Err(McpSubmissionError::Denied);
        }
        let rpc_id = envelope.id().ok_or(McpSubmissionError::Invalid)?.clone();
        let mut framed = Vec::with_capacity(wire.len() + 1);
        framed.extend_from_slice(wire);
        framed.push(b'\n');
        Ok(CopiedRequest {
            permission: request.clone(),
            tool: invocation.tool_name.clone(),
            call: invocation.call_id.clone(),
            arguments: canonical,
            wire: framed.into_boxed_slice(),
            rpc_id,
        })
    }
}

struct CopiedRequest {
    permission: PermissionRequest,
    tool: ToolName,
    call: ToolCallId,
    arguments: Box<[u8]>,
    wire: Box<[u8]>,
    rpc_id: RpcId,
}

/// Reserved exact request, not yet executable. Drop releases only its own
/// reservation generation, never a replacement or claimed tombstone.
pub struct PreparedMcpSubmission {
    data: Data,
}
impl PreparedMcpSubmission {
    /// Binds a concrete native policy proof without publishing a ready route.
    /// Core must consume the resulting admission; there is no no-proof variant.
    #[must_use]
    pub fn bind_execution(self, proof: NativePermissionExecutionProof) -> McpSubmissionAdmission {
        McpSubmissionAdmission {
            ready: Box::new(Ready {
                data: self.data,
                proof,
            }),
        }
    }
}

/// One-shot core admission retaining the concrete proof through submission.
pub struct McpSubmissionAdmission {
    ready: Box<Ready>,
}
impl PermissionExecutionAdmission for McpSubmissionAdmission {
    fn admit(self: Box<Self>) -> std::result::Result<(), PermissionError> {
        self.publish().map_err(|_| {
            PermissionError::new(
                "mcp_submission_unavailable",
                "MCP submission admission rejected",
            )
        })
    }
}
impl McpSubmissionAdmission {
    fn publish(self) -> Result<()> {
        let registry = self
            .ready
            .data
            .reservation
            .registry
            .upgrade()
            .ok_or(McpSubmissionError::Unavailable)?;
        registry.live()?;
        check(&self.ready.data.cancellation)?;
        self.ready.data.runtime.live()?;
        self.ready
            .proof
            .revalidate()
            .map_err(|_| McpSubmissionError::Denied)?;
        let previous = {
            let mut state = registry
                .state
                .lock()
                .map_err(|_| McpSubmissionError::Unavailable)?;
            registry.live()?;
            let reservation = &self.ready.data.reservation;
            if !matches!(state.slots.get(&reservation.call), Some(Slot::Reserved { generation, .. }) if *generation == reservation.generation)
            {
                return Err(McpSubmissionError::Denied);
            }
            state
                .slots
                .insert(reservation.call.clone(), Slot::Ready(self.ready))
        };
        drop(previous);
        Ok(())
    }
}

/// Non-clone owned submission. Retains exact wire bytes, native proof and runtime
/// identity while queued. Dropping it never makes its call reusable.
pub struct McpSubmission {
    registry: Arc<McpSubmissionRegistry>,
    ready: Box<Ready>,
    cancellation: CancellationToken,
    attempted: bool,
}
impl McpSubmission {
    fn checkpoint(&self) -> Result<()> {
        check(&self.cancellation)?;
        check(&self.ready.data.cancellation)?;
        self.registry.live()?;
        self.ready.data.runtime.live()?;
        self.ready
            .proof
            .revalidate()
            .map_err(|_| McpSubmissionError::Denied)?;
        check(&self.cancellation)?;
        check(&self.ready.data.cancellation)?;
        self.registry.live()?;
        self.ready.data.runtime.live()
    }

    /// Wait for execution/preparation cancellation, scope closure or runtime
    /// retirement. Race this against queue admission, then use `into_writer`.
    /// The original core turn's cancellation is observed independently of both
    /// caller-supplied tokens, including while a queue provides no wakeups.
    #[must_use]
    pub fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut execution = Box::pin(self.cancellation.cancelled());
            let mut preparation = Box::pin(self.ready.data.cancellation.cancelled());
            let mut turn = Box::pin(self.registry.cancellation.cancelled());
            let mut core_turn = Box::pin(self.registry.handle.cancelled());
            let mut runtime = Box::pin(self.ready.data.runtime.cancellation.cancelled());
            poll_fn(|cx| {
                if execution.as_mut().poll(cx).is_ready()
                    || preparation.as_mut().poll(cx).is_ready()
                    || turn.as_mut().poll(cx).is_ready()
                    || core_turn.as_mut().poll(cx).is_ready()
                    || runtime.as_mut().poll(cx).is_ready()
                {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await;
        })
    }

    /// Consumes this one-shot submission into an inert writer-bound future.
    /// Obtain a queue permit first and retain it in `writer`. The guard owns
    /// both proof and request through all partial-write/flush polls; no callback
    /// future can outlive its proof. Dropping the guard never enables replay.
    #[must_use]
    pub fn into_writer<W: McpSubmissionWriter>(self, writer: W) -> McpSubmissionWrite<W> {
        let cancellations = Some([
            self.cancellation.cancelled(),
            self.ready.data.cancellation.cancelled(),
            self.registry.cancellation.cancelled(),
            self.registry.handle.cancelled(),
            self.ready.data.runtime.cancellation.cancelled(),
        ]);
        McpSubmissionWrite {
            submission: self,
            writer,
            offset: 0,
            terminal: false,
            cancellations,
        }
    }

    /// Whether writer delegation was attempted; does not report bytes written.
    #[must_use]
    pub const fn was_attempted(&self) -> bool {
        self.attempted
    }
    /// Exact request ID, for audit routing only; not transferable authority.
    #[must_use]
    pub fn permission_request_id(&self) -> &PermissionRequestId {
        &self.ready.data.permission.id
    }
    /// Exact retained wire ID for response correlation, never a grant.
    #[must_use]
    pub fn rpc_id(&self) -> &RpcId {
        &self.ready.data.rpc_id
    }
}

/// Trusted synchronous writer boundary. Implementations must perform their
/// actual delegation within these poll calls, not spawn/defer unguarded writes.
/// As with `AsyncWrite`, `Pending` and errors must not claim a successful byte
/// count; an error is nevertheless an ambiguous attempted delegation. Wrappers
/// own the already-acquired exclusive queue permit. This direct writer guard
/// emits exactly the prepared JSON payload plus one final LF. Its writer MUST
/// be the final synchronous plaintext submission boundary, not a queue, HTTP
/// body producer, or an API that later drives writes without this proof. In
/// particular, feeding Hyper does not satisfy the HTTP/TLS driver boundary.
pub trait McpSubmissionWriter {
    /// Delegates only the supplied remaining request suffix.
    fn poll_write(&mut self, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>>;
    /// Flushes under the same retained authority after every byte is accepted.
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
}

/// Non-clone, writer-bound request future. Each poll delegates at most one
/// nonempty suffix (or one flush), revalidating immediately before delegation.
/// Successful byte counts advance a private offset; Pending retries cannot
/// restart an accepted prefix. Any failure permanently terminates this guard.
pub struct McpSubmissionWrite<W> {
    submission: McpSubmission,
    writer: W,
    offset: usize,
    terminal: bool,
    cancellations: Option<[Cancelled; 5]>,
}
impl<W> McpSubmissionWrite<W> {
    /// True once any writer method was entered, even if it returned an error.
    #[must_use]
    pub const fn was_attempted(&self) -> bool {
        self.submission.was_attempted()
    }
    /// Successfully acknowledged bytes; errors can have additional unknown effects.
    #[must_use]
    pub const fn acknowledged_bytes(&self) -> usize {
        self.offset
    }
}
impl<W> fmt::Debug for McpSubmissionWrite<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpSubmissionWrite { <redacted> }")
    }
}
impl<W: McpSubmissionWriter + Unpin> Future for McpSubmissionWrite<W> {
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.terminal {
            return Poll::Ready(Err(McpSubmissionError::AlreadyAttempted));
        }
        if this.cancellations.as_mut().is_some_and(|waiters| {
            waiters
                .iter_mut()
                .any(|waiter| Pin::new(waiter).poll(cx).is_ready())
        }) {
            this.terminal = true;
            this.cancellations = None;
            return Poll::Ready(Err(McpSubmissionError::Cancelled));
        }
        if let Err(error) = this.submission.checkpoint() {
            this.terminal = true;
            this.cancellations = None;
            return Poll::Ready(Err(error));
        }
        // Mark terminal/attempted before delegation, including a writer panic.
        this.terminal = true;
        this.submission.attempted = true;
        let remaining = &this.submission.ready.data.wire[this.offset..];
        if remaining.is_empty() {
            let outcome = this.writer.poll_flush(cx);
            if let Err(error) = this.submission.checkpoint() {
                this.cancellations = None;
                return Poll::Ready(Err(error));
            }
            match outcome {
                Poll::Pending => {
                    this.terminal = false;
                    Poll::Pending
                }
                Poll::Ready(Ok(())) => {
                    this.cancellations = None;
                    Poll::Ready(Ok(()))
                }
                Poll::Ready(Err(_)) => {
                    this.cancellations = None;
                    Poll::Ready(Err(McpSubmissionError::WriterFailed))
                }
            }
        } else {
            let remaining_len = remaining.len();
            let outcome = this.writer.poll_write(cx, remaining);
            if let Poll::Ready(Ok(count)) = &outcome
                && *count > 0
                && *count <= remaining_len
            {
                this.offset += count;
            }
            if let Err(error) = this.submission.checkpoint() {
                this.cancellations = None;
                return Poll::Ready(Err(error));
            }
            match outcome {
                Poll::Pending => {
                    this.terminal = false;
                    Poll::Pending
                }
                Poll::Ready(Ok(count)) if count > 0 && count <= remaining_len => {
                    this.terminal = false;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Poll::Ready(_) => {
                    this.cancellations = None;
                    Poll::Ready(Err(McpSubmissionError::WriterFailed))
                }
            }
        }
    }
}

redacted_debug!(
    McpSubmissionRegistry,
    McpSubmissionTurnRegistration,
    PreparedMcpSubmission,
    McpSubmissionAdmission,
    McpSubmission
);

fn check(cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(McpSubmissionError::Cancelled)
    } else {
        Ok(())
    }
}
fn next_generation(next: &mut Option<u64>) -> Result<u64> {
    let generation = next.ok_or(McpSubmissionError::GenerationExhausted)?;
    *next = generation.checked_add(1);
    Ok(generation)
}

fn canonical_arguments(value: &Value) -> Result<Box<[u8]>> {
    let mut stack = vec![(value, 1)];
    let mut nodes = 0usize;
    let mut string_bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > MAX_ARGUMENT_NODES || depth > MAX_ARGUMENT_DEPTH {
            return Err(McpSubmissionError::Limit);
        }
        match value {
            Value::Array(values) => {
                if nodes + stack.len() + values.len() > MAX_ARGUMENT_NODES {
                    return Err(McpSubmissionError::Limit);
                }
                stack.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if nodes + stack.len() + values.len().saturating_mul(2) > MAX_ARGUMENT_NODES {
                    return Err(McpSubmissionError::Limit);
                }
                nodes += values.len();
                for key in values.keys() {
                    charge_string_bytes(&mut string_bytes, key.len())?;
                }
                stack.extend(values.values().map(|value| (value, depth + 1)));
            }
            Value::String(text) => charge_string_bytes(&mut string_bytes, text.len())?,
            _ => {}
        }
    }
    bounded_json(value, MAX_MCP_SUBMISSION_ARGUMENT_BYTES)
}
fn charge_string_bytes(used: &mut usize, bytes: usize) -> Result<()> {
    if bytes > MAX_MCP_SUBMISSION_ARGUMENT_BYTES - *used {
        return Err(McpSubmissionError::Limit);
    }
    *used += bytes;
    Ok(())
}
fn bounded_json(value: &impl Serialize, limit: usize) -> Result<Box<[u8]>> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit - self.bytes.len() {
                return Err(io::Error::other("MCP byte limit"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| McpSubmissionError::Limit)?;
    Ok(writer.bytes.into_boxed_slice())
}

#[cfg(test)]
mod tests;
