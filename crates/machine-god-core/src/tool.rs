use crate::{
    BoxFuture, CancellationToken, Capability, SessionId, SessionIncarnationId, ToolCallId,
    ToolError, ToolName, TurnId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

/// Model-visible description and JSON Schema input contract for a tool.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolSpec {
    pub name: ToolName,
    pub description: String,
    pub input_schema: Value,
}

/// A complete tool invocation requested by a provider.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: ToolName,
    pub arguments: Value,
}

/// Explicit context passed to a tool invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolContext {
    pub session_id: SessionId,
    pub session_incarnation_id: SessionIncarnationId,
    pub turn_id: TurnId,
    pub call_id: ToolCallId,
}

/// Effect-free result of preparing one model-requested tool call.
///
/// The authorization disposition is an explicit assertion by trusted tool
/// code. The owned arguments are passed unchanged to [`Tool::execute`] after
/// any required authorization succeeds.
pub struct PreparedToolCall {
    authorization: PreparedToolAuthorization,
    arguments: Value,
    execution_cancellation: ToolExecutionCancellation,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ToolExecutionCancellation {
    #[default]
    Cancellable,
    CompletionWinsAfterFirstPoll,
}

/// Authorization disposition selected by trusted [`Tool::prepare`] code.
///
/// Core never infers this disposition from model-controlled arguments. Tools
/// retain the permission-required behavior unless they explicitly construct
/// [`PreparedToolAuthorization::NoAuthorityRequired`] through
/// [`PreparedToolCall::without_authority`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PreparedToolAuthorization {
    /// Present the exact capability to the host permission policy before
    /// execution.
    PermissionRequired(Capability),
    /// Execute without invoking permission policy because this call requires
    /// no policy-governed authority.
    NoAuthorityRequired,
}

impl PreparedToolCall {
    /// Creates a prepared call from its policy capability and execution input.
    ///
    /// A caller implementing [`Tool::prepare`] must construct this value in
    /// bounded, nonblocking, effect-free work. Preparation is synchronous and
    /// receives no cancellation token: core checks cancellation immediately
    /// before and after it returns, but cannot preempt it while it is running.
    /// The capability must cover every authority that the later
    /// [`Tool::execute`] may exercise, and that execution must interpret these
    /// arguments consistently with the operation described by the capability.
    #[must_use]
    pub fn new(capability: Capability, arguments: Value) -> Self {
        Self {
            authorization: PreparedToolAuthorization::PermissionRequired(capability),
            arguments,
            execution_cancellation: ToolExecutionCancellation::Cancellable,
        }
    }

    /// Creates a prepared call that requires no policy-governed permission or
    /// authority.
    ///
    /// This is a trusted-host assertion, not an optimization. A tool must use
    /// this constructor only when executing the prepared arguments requires no
    /// policy-governed authority. Core skips permission request/resolution
    /// events and does not invoke the permission handler for this call. This
    /// disposition does not assert that execution performs no host interaction:
    /// a tool may use a separately injected host-interaction interface outside
    /// permission policy only when its public contract explicitly documents
    /// that interface and boundary.
    #[must_use]
    pub fn without_authority(arguments: Value) -> Self {
        Self {
            authorization: PreparedToolAuthorization::NoAuthorityRequired,
            arguments,
            execution_cancellation: ToolExecutionCancellation::Cancellable,
        }
    }

    /// Declares that execution owns cancellation once its first poll begins.
    ///
    /// This is a trusted tool assertion for an operation whose first poll is
    /// its irreversible submission boundary. Turn cancellation still wins
    /// before that poll. Once polling starts, core waits for the real result,
    /// stores it, and publishes its finished event before observing a pending
    /// turn cancellation. Model-controlled arguments cannot select this mode.
    #[must_use]
    pub fn completion_wins_after_first_poll(mut self) -> Self {
        self.execution_cancellation = ToolExecutionCancellation::CompletionWinsAfterFirstPoll;
        self
    }

    /// Returns the explicit authorization disposition for this call.
    #[must_use]
    pub fn authorization(&self) -> &PreparedToolAuthorization {
        &self.authorization
    }

    /// Returns the exact capability that will be presented to policy, if one
    /// is required.
    ///
    /// Returns `Some` for
    /// [`PreparedToolAuthorization::PermissionRequired`] and `None` for
    /// [`PreparedToolAuthorization::NoAuthorityRequired`].
    #[must_use]
    pub fn capability(&self) -> Option<&Capability> {
        match &self.authorization {
            PreparedToolAuthorization::PermissionRequired(capability) => Some(capability),
            PreparedToolAuthorization::NoAuthorityRequired => None,
        }
    }

    /// Returns the exact arguments that will be passed to execution.
    #[must_use]
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }

    pub(crate) fn into_arguments(mut self) -> Value {
        std::mem::take(&mut self.arguments)
    }

    pub(crate) const fn execution_cancellation(&self) -> ToolExecutionCancellation {
        self.execution_cancellation
    }
}

impl fmt::Debug for PreparedToolCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedToolCall")
            .finish_non_exhaustive()
    }
}

impl Drop for PreparedToolCall {
    fn drop(&mut self) {
        if let PreparedToolAuthorization::PermissionRequired(capability) = &mut self.authorization {
            match capability {
                Capability::Tool { arguments, .. } => {
                    crate::session::drop_json_value_iterative(std::mem::take(arguments));
                }
                Capability::Custom { details, .. } => {
                    crate::session::drop_json_value_iterative(std::mem::take(details));
                }
                Capability::Filesystem { .. }
                | Capability::FilesystemRename { .. }
                | Capability::FilesystemCopy { .. }
                | Capability::OpenFile { .. }
                | Capability::Process { .. }
                | Capability::Network { .. }
                | Capability::Vision { .. } => {}
            }
        }
        crate::session::drop_json_value_iterative(std::mem::take(&mut self.arguments));
    }
}

/// Provider-neutral tool output.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolOutput {
    pub content: Value,
    pub is_error: bool,
}

impl ToolOutput {
    #[must_use]
    pub fn success(content: impl Into<Value>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }
}

/// One executable tool registration whose lifetime is limited to the current
/// turn.
///
/// Core advertises a registration only on model rounds after the tool call
/// that produced it has completed and its visible output has been durably
/// stored. The registration is never written to the transcript or session
/// store and is discarded when the turn ends.
pub struct TurnToolRegistration {
    spec: ToolSpec,
    tool: Arc<dyn Tool>,
}

impl TurnToolRegistration {
    /// Captures one tool implementation and its specification.
    #[must_use]
    pub fn new(tool: impl Tool) -> Self {
        Self::shared(Arc::new(tool))
    }

    /// Captures one explicitly shared tool implementation and its
    /// specification.
    #[must_use]
    pub fn shared(tool: Arc<dyn Tool>) -> Self {
        let spec = tool.spec();
        Self { spec, tool }
    }

    /// Returns the captured model-visible specification.
    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    pub(crate) fn tool(&self) -> Arc<dyn Tool> {
        Arc::clone(&self.tool)
    }
}

impl fmt::Debug for TurnToolRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TurnToolRegistration")
            .finish_non_exhaustive()
    }
}

impl Drop for TurnToolRegistration {
    fn drop(&mut self) {
        crate::session::drop_json_value_iterative(std::mem::take(&mut self.spec.input_schema));
    }
}

/// Complete result of a tool execution inside core orchestration.
///
/// The ordinary output remains the only durable, model-visible result. An
/// optional next-round registration is an ephemeral orchestration side
/// effect. Existing tools receive this behavior through the default
/// [`Tool::execute_for_turn`] implementation and need not opt into it.
pub struct ToolExecution {
    output: ToolOutput,
    next_round_tool: Option<Arc<TurnToolRegistration>>,
}

impl ToolExecution {
    /// Creates an execution with only an ordinary durable output.
    #[must_use]
    pub fn output(output: ToolOutput) -> Self {
        Self {
            output,
            next_round_tool: None,
        }
    }

    /// Creates an execution that also proposes one executable tool for later
    /// model rounds in this turn.
    #[must_use]
    pub fn with_next_round_tool(
        output: ToolOutput,
        next_round_tool: Arc<TurnToolRegistration>,
    ) -> Self {
        Self {
            output,
            next_round_tool: Some(next_round_tool),
        }
    }

    /// Returns the ordinary durable output carried by this execution.
    #[must_use]
    pub fn tool_output(&self) -> &ToolOutput {
        &self.output
    }

    /// Returns the proposed next-round registration, when present.
    #[must_use]
    pub fn next_round_tool(&self) -> Option<&TurnToolRegistration> {
        self.next_round_tool.as_deref()
    }

    pub(crate) fn into_parts(self) -> (ToolOutput, Option<Arc<TurnToolRegistration>>) {
        (self.output, self.next_round_tool)
    }

    pub(crate) fn drain_owned_json(&mut self) {
        crate::session::drop_json_value_iterative(std::mem::take(&mut self.output.content));
    }
}

impl fmt::Debug for ToolExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolExecution")
            .field("has_next_round_tool", &self.next_round_tool.is_some())
            .finish_non_exhaustive()
    }
}

/// Object-safe tool implementation supplied explicitly by a host.
pub trait Tool: Send + Sync + 'static {
    fn spec(&self) -> ToolSpec;

    /// Validates and normalizes a call without exercising external authority.
    ///
    /// The default preserves the original critical-risk tool authorization:
    /// policy sees the registered name, call ID, and model arguments, and
    /// execution receives those same arguments.
    ///
    /// Implementations must complete in bounded time, must not block on
    /// external work, and must not exercise filesystem, process, network, or
    /// any other authority. This method is synchronous and receives no
    /// cancellation token: core checks cancellation immediately before and
    /// after it returns, but cannot preempt an in-flight preparation.
    /// For [`PreparedToolAuthorization::PermissionRequired`],
    /// [`Tool::execute`] must exercise no policy-governed authority beyond the
    /// returned capability and must interpret the returned arguments
    /// consistently with the operation that capability describes. For
    /// [`PreparedToolAuthorization::NoAuthorityRequired`], execution must not
    /// require policy-governed authority. A separately injected host-
    /// interaction interface outside permission policy is permitted only when
    /// the tool's public contract explicitly documents that interface and
    /// boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when the implementation cannot validate or
    /// normalize the call without exercising external authority.
    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        let ToolCall {
            id,
            name,
            arguments,
        } = call;
        let capability = Capability::Tool {
            name,
            call_id: id,
            arguments: arguments.clone(),
        };
        Ok(PreparedToolCall::new(capability, arguments))
    }

    /// Executes one tool call.
    ///
    /// Core passes the arguments validated and normalized by [`Tool::prepare`].
    /// For permission-required calls, core invokes this method only after the
    /// host policy resolves their exact capability, and execution must stay
    /// within that policy-governed authority. No-authority calls skip host
    /// permission policy and must require no policy-governed authority. They
    /// may use a separately injected host-interaction interface outside policy
    /// only when the tool's public contract explicitly documents that
    /// interface and boundary. Direct callers that bypass core orchestration
    /// are responsible for establishing the same preparation and authorization
    /// preconditions.
    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>>;

    /// Executes one tool call with access to turn-local orchestration effects.
    ///
    /// The default preserves the ordinary [`Tool::execute`] behavior. A tool
    /// may override this method to propose one executable registration for
    /// subsequent model rounds in the same turn. Core validates the complete
    /// resulting catalog and activates the registration only after the
    /// visible output is durably stored. Registrations never become visible in
    /// the same provider response, survive the turn, or grant authority.
    ///
    /// Core passes the arguments validated and normalized by [`Tool::prepare`]
    /// and applies the same authorization boundary as [`Tool::execute`]. For
    /// permission-required calls, overrides must remain within the exact
    /// capability approved by host policy. No-authority calls must require no
    /// policy-governed authority and may use a separately injected host-
    /// interaction interface only when the tool's public contract documents
    /// that interface and boundary. Direct callers must establish those same
    /// preparation and authorization preconditions before invoking an
    /// override.
    fn execute_for_turn(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let execution = self.execute(context, arguments, cancellation);
        Box::pin(async move { execution.await.map(ToolExecution::output) })
    }
}
