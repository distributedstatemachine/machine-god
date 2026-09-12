use super::{
    route::{ServerRoute, ToolRoute},
    tool::unavailable,
};
use crate::mcp::{
    catalog::McpToolDescriptor,
    context::NativeMcpTurnContext,
    continuation::{AdmittedResponse, ContinuationConsent, ContinuationInput},
    protocol::{NegotiatedProtocol, RpcId},
    submission::{McpContinuationCustody, McpSubmission, McpSubmissionRuntime, McpToolCallOptions},
    tool_result::{
        McpToolResponseContext, McpToolResponseDisposition, NativeMcpToolResultAdmission,
    },
};
use futures_util::future::{Either, select};
use machine_god_core::{BoxFuture, CancellationToken, ToolContext, ToolError, ToolName};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

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
    custody: McpContinuationCustody,
    round: Arc<AtomicBool>,
    response_pending: bool,
    continuations: u8,
    interaction_deadline: Option<Instant>,
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
        let custody = submission
            .continuation_custody()
            .map_err(|_| unavailable())?;
        let round = submission.write_completion();
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
            custody,
            round,
            response_pending: false,
            continuations: 0,
            interaction_deadline: None,
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
        self.custody.revalidate().map_err(|_| unavailable())?;
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
        let binding = self.tool.binding.cancelled_owned();
        let custody = self.custody.cancelled_owned();
        let invalid = self.tool.binding.live().is_err();
        Box::pin(async move {
            if invalid {
                return;
            }
            select(
                Box::pin(async {
                    select(caller, turn).await;
                }),
                Box::pin(async {
                    select(
                        route,
                        Box::pin(async {
                            select(Box::pin(binding), custody).await;
                        }),
                    )
                    .await;
                }),
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
            if !self.round.load(Ordering::Acquire) {
                return Err(unavailable());
            }
            self.response_pending = true;
            Ok(NativeMcpRuntimeToolResponse {
                bytes: response,
                request_id: self.request_id.clone(),
                protocol: self.server.protocol,
                round: self.round.clone(),
            })
        })
    }

    pub(crate) fn admit_response(
        &mut self,
        response: NativeMcpRuntimeToolResponse,
        admission: &NativeMcpToolResultAdmission,
    ) -> Result<AdmittedResponse, ToolError> {
        self.revalidate()?;
        if !self.response_pending
            || !Arc::ptr_eq(&self.round, &response.round)
            || !self.round.load(Ordering::Acquire)
        {
            return Err(unavailable());
        }
        self.response_pending = false;
        let context = McpToolResponseContext::new(
            self.context.clone(),
            self.tool.name.clone(),
            self.server.name.clone(),
            self.tool.descriptor.clone(),
            self.tool.binding.clone(),
            self.server.protocol,
            self.request_id.clone(),
        )
        .map_err(|_| unavailable())?;
        let bytes = response.into_bytes();
        let admitted = admission
            .admit(context, &bytes)
            .map_err(|_| unavailable())?;
        self.revalidate()?;
        Ok(match admitted {
            McpToolResponseDisposition::Complete(output) => AdmittedResponse::Complete(output),
            McpToolResponseDisposition::ProtocolFailure(failure) => {
                AdmittedResponse::ProtocolFailure(failure)
            }
            McpToolResponseDisposition::InputRequired(required) => {
                if self.interaction_deadline.is_none() {
                    self.interaction_deadline = Some(
                        self.server
                            .clock
                            .now()
                            .checked_add(Duration::from_secs(30 * 60))
                            .ok_or_else(unavailable)?,
                    );
                }
                AdmittedResponse::InputRequired(ContinuationInput {
                    required,
                    round: self.round.clone(),
                })
            }
        })
    }

    pub(crate) fn continuation_limit_reached(&self) -> bool {
        self.continuations >= 8
    }

    pub(crate) fn check_interaction_deadline(&self) -> Result<(), ToolError> {
        if self
            .interaction_deadline
            .is_none_or(|deadline| self.server.clock.now() >= deadline)
        {
            Err(unavailable())
        } else {
            Ok(())
        }
    }

    pub(crate) fn interaction_cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let Some(deadline) = self.interaction_deadline else {
                return;
            };
            select(self.cancelled(), self.server.clock.sleep_until(deadline)).await;
        })
    }

    pub(crate) fn continue_exchange(
        &mut self,
        consent: ContinuationConsent,
    ) -> BoxFuture<'_, Result<NativeMcpRuntimeToolResponse, ToolError>> {
        Box::pin(async move {
            self.revalidate()?;
            self.check_interaction_deadline()?;
            if self.continuation_limit_reached()
                || self.pending.is_some()
                || self.response_pending
                || !Arc::ptr_eq(&self.round, &consent.input.round)
                || !self.round.load(Ordering::Acquire)
            {
                return Err(unavailable());
            }
            // Consume this round before the first suspension, including queue
            // failure/drop. A consent allocation can never authorize replay.
            self.round = Arc::new(AtomicBool::new(false));
            self.continuations += 1;
            let response = {
                let (mut peer, _) = match select(
                    Box::pin(self.server.acquire(&self.turn, &self.cancellation)),
                    self.interaction_cancelled(),
                )
                .await
                {
                    Either::Left((peer, observer)) => (peer.map_err(|_| unavailable())?, observer),
                    Either::Right(_) => return Err(unavailable()),
                };
                self.revalidate()?;
                self.check_interaction_deadline()?;
                let reservation = peer.peer.reserve().map_err(|_| unavailable())?;
                let submission = self
                    .custody
                    .prepare(
                        reservation,
                        &consent.responses,
                        consent.input.required.required().request_state_json(),
                    )
                    .map_err(|_| unavailable())?;
                let written = submission.write_completion();
                let options = submission.tool_options().ok_or_else(unavailable)?;
                let id = submission.rpc_id().clone();
                let original_cancelled = submission.cancelled_owned();
                let deadline = peer.deadline;
                let bytes = match select(
                    Box::pin(peer.peer.call(submission, deadline)),
                    Box::pin(async {
                        select(self.interaction_cancelled(), original_cancelled).await;
                    }),
                )
                .await
                {
                    Either::Left((bytes, _)) => bytes.map_err(|_| unavailable())?,
                    Either::Right(_) => return Err(unavailable()),
                };
                self.revalidate()?;
                self.check_interaction_deadline()?;
                if !written.load(Ordering::Acquire) {
                    return Err(unavailable());
                }
                (bytes, written, options, id)
            };
            self.round = response.1;
            self.options = response.2;
            self.request_id = response.3;
            self.response_pending = true;
            Ok(NativeMcpRuntimeToolResponse {
                bytes: response.0,
                request_id: self.request_id.clone(),
                protocol: self.server.protocol,
                round: self.round.clone(),
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
    round: Arc<AtomicBool>,
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
