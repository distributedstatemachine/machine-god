use crate::{
    BoxFuture, CancellationToken, Capability, SessionId, SessionIncarnationId, ToolCallId,
    ToolError, ToolName, TurnId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

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
        }
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
                | Capability::Network { .. } => {}
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
}
