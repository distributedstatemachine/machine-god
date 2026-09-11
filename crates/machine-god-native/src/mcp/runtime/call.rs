use super::{
    route::{ServerRoute, ToolRoute},
    tool::unavailable,
};
use crate::mcp::{
    catalog::McpToolDescriptor,
    context::NativeMcpTurnContext,
    protocol::{NegotiatedProtocol, RpcId},
    submission::{McpSubmission, McpSubmissionRuntime, McpToolCallOptions},
};
use futures_util::future::{Either, select};
use machine_god_core::{BoxFuture, CancellationToken, ToolContext, ToolError, ToolName};
use serde_json::Value;
use std::sync::Arc;

/// Native-owned original invocation and its one-shot claimed permission proof.
/// No public constructor, raw writer, peer handle or replay method is exposed.
pub struct NativeMcpRuntimeToolCall {
    tool: Arc<ToolRoute>,
    server: Arc<ServerRoute>,
    turn: NativeMcpTurnContext,
    context: ToolContext,
    arguments: Value,
    cancellation: CancellationToken,
    pending: Option<McpSubmission>,
    options: McpToolCallOptions,
    request_id: RpcId,
}
impl NativeMcpRuntimeToolCall {
    pub(super) async fn claim(
        tool: Arc<ToolRoute>,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> Result<Self, ToolError> {
        super::tool::validate_arguments(&arguments, &cancellation)?;
        let turn = tool
            .contexts
            .snapshot_for_tool(&context)
            .map_err(|_| unavailable())?;
        let server = tool.server.upgrade().ok_or_else(unavailable)?;
        tool.binding.live().map_err(|_| unavailable())?;
        if server.cancellation.is_cancelled() || cancellation.is_cancelled() {
            return Err(unavailable());
        }
        let submission = turn
            .registry()
            .map_err(|_| unavailable())?
            .claim(
                context.clone(),
                &tool.name,
                &arguments,
                tool.binding.clone(),
                cancellation.clone(),
            )
            .await
            .map_err(|_| unavailable())?;
        let options = submission.tool_options().ok_or_else(unavailable)?;
        let request_id = submission.rpc_id().clone();
        let call = Self {
            tool,
            server,
            turn,
            context,
            arguments,
            cancellation,
            pending: Some(submission),
            options,
            request_id,
        };
        call.revalidate()?;
        Ok(call)
    }
    #[must_use]
    pub fn context(&self) -> &ToolContext {
        &self.context
    }
    #[must_use]
    pub fn tool_name(&self) -> &ToolName {
        &self.tool.name
    }
    #[must_use]
    pub fn server_name(&self) -> &str {
        &self.server.name
    }
    #[must_use]
    pub fn descriptor(&self) -> &McpToolDescriptor {
        &self.tool.descriptor
    }
    #[must_use]
    pub fn runtime(&self) -> &Arc<McpSubmissionRuntime> {
        &self.tool.binding
    }
    #[must_use]
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }
    #[must_use]
    pub const fn options(&self) -> McpToolCallOptions {
        self.options
    }
    #[must_use]
    pub fn request_id(&self) -> &RpcId {
        &self.request_id
    }
    #[must_use]
    pub fn protocol(&self) -> NegotiatedProtocol {
        self.server.protocol
    }

    /// Checks exact native turn/runtime/caller liveness, not permission for a
    /// new request. A completed first exchange never grants continuation rights.
    /// # Errors
    /// Rejects cancelled or replaced exact ownership.
    pub fn revalidate(&self) -> Result<(), ToolError> {
        self.turn.revalidate().map_err(|_| unavailable())?;
        self.tool.binding.live().map_err(|_| unavailable())?;
        if self.cancellation.is_cancelled() || self.server.cancellation.is_cancelled() {
            return Err(unavailable());
        }
        Ok(())
    }
    /// Observes lifecycle cancellation without retaining an executable peer.
    #[must_use]
    pub fn cancelled(&self) -> BoxFuture<'static, ()> {
        let caller = self.cancellation.cancelled();
        let turn = self.turn.cancelled();
        let route = self.server.cancellation.cancelled();
        let binding = self.tool.binding.clone();
        Box::pin(async move {
            if binding.live().is_err() {
                return;
            }
            select(
                Box::pin(async {
                    select(caller, turn).await;
                }),
                route,
            )
            .await;
        })
    }

    /// Consumes the sole initial exchange on its first poll. Dropping a polled
    /// future cannot replay it; dropping an unpolled future performs no I/O.
    /// Actual peer writing retains the exact native proof through every write.
    /// # Errors
    /// Rejects repeated attempts, stale ownership, queue bounds and peer failure.
    pub fn first_exchange(
        &mut self,
    ) -> BoxFuture<'_, Result<NativeMcpRuntimeToolResponse, ToolError>> {
        Box::pin(async move {
            let submission = self.pending.take().ok_or_else(unavailable)?;
            self.revalidate()?;
            let original_cancelled = submission.cancelled_owned();
            let (mut peer, original_cancelled) = match select(
                Box::pin(self.server.acquire(&self.turn, &self.cancellation)),
                original_cancelled,
            )
            .await
            {
                Either::Left((peer, original_cancelled)) => {
                    (peer.map_err(|_| unavailable())?, original_cancelled)
                }
                Either::Right(_) => return Err(unavailable()),
            };
            self.revalidate()?;
            let deadline = peer.deadline;
            let response = match select(
                Box::pin(peer.peer.call(submission, deadline)),
                Box::pin(async {
                    select(self.cancelled(), original_cancelled).await;
                }),
            )
            .await
            {
                Either::Left((response, _)) => response.map_err(|_| unavailable())?,
                Either::Right(_) => return Err(unavailable()),
            };
            self.revalidate()?;
            Ok(NativeMcpRuntimeToolResponse {
                bytes: response,
                request_id: self.request_id.clone(),
                protocol: self.server.protocol,
            })
        })
    }
}

/// Correlated original response bytes plus immutable peer-selected provenance.
/// This is untrusted result data, not a write or continuation capability.
pub struct NativeMcpRuntimeToolResponse {
    bytes: Box<[u8]>,
    request_id: RpcId,
    protocol: NegotiatedProtocol,
}
impl NativeMcpRuntimeToolResponse {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub fn request_id(&self) -> &RpcId {
        &self.request_id
    }
    #[must_use]
    pub const fn protocol(&self) -> NegotiatedProtocol {
        self.protocol
    }
    #[must_use]
    pub fn into_bytes(self) -> Box<[u8]> {
        self.bytes
    }
}
macro_rules! redacted { ($($ty:ty),+ $(,)?) => { $(impl std::fmt::Debug for $ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(concat!(stringify!($ty), " { <redacted> }")) }
})+ }; }
redacted!(NativeMcpRuntimeToolCall, NativeMcpRuntimeToolResponse);
