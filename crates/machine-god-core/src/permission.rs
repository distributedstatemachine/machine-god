use crate::{
    BoxFuture, PermissionError, PermissionRequestId, SessionId, SessionIncarnationId, ToolCallId,
    ToolName, TurnId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Filesystem operation being considered by a permission handler.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FilesystemAccess {
    Read,
    Metadata,
    Write,
    Edit,
    Create,
    Delete,
    Enumerate,
    EnumerateRecursive,
    SearchContent,
}

/// Normalized network destination supplied by a native host.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkTarget {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
}

/// Stable identity of the exact environment installed for a process.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcessEnvironment {
    /// Host-defined environment profile name.
    pub profile: String,
    /// Lowercase SHA-256 digest of the profile's exact environment entries.
    pub sha256: String,
}

/// Explicit standard-input mode included in a process's permission identity.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessInput {
    /// No host-supplied process input.
    #[default]
    Null,
    /// An explicitly owned pipe for subsequent separately authorized input.
    Pipe,
}

/// An explicit capability that a host may authorize or deny.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Capability {
    Tool {
        name: ToolName,
        call_id: ToolCallId,
        arguments: Value,
    },
    Filesystem {
        access: FilesystemAccess,
        path: String,
    },
    FilesystemRename {
        old_path: String,
        new_path: String,
    },
    FilesystemCopy {
        source: String,
        destination: String,
    },
    /// Open one existing regular file in a host-selected desktop application.
    OpenFile {
        path: String,
    },
    Process {
        program: String,
        arguments: Vec<String>,
        working_directory: String,
        environment: ProcessEnvironment,
        /// Older serialized capabilities imply null standard input.
        #[serde(default)]
        stdin: ProcessInput,
    },
    Network {
        target: NetworkTarget,
    },
    /// Disclose the exact ordered set of approved workspace images to one
    /// normalized provider destination for visual inspection.
    Vision {
        paths: Vec<String>,
        target: NetworkTarget,
    },
    Custom {
        name: String,
        details: Value,
    },
}

/// Host-facing risk hint. The handler remains the authority.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRisk {
    Low,
    Medium,
    High,
    Critical,
}

/// Complete, auditable input to permission policy.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PermissionRequest {
    pub id: PermissionRequestId,
    pub session_id: SessionId,
    pub session_incarnation_id: SessionIncarnationId,
    pub turn_id: TurnId,
    pub capability: Capability,
    pub risk: PermissionRisk,
    pub reason: String,
}

/// Lifetime of a positive permission decision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGrantScope {
    Once,
    Turn,
    Session,
}

/// An explicit permission decision. Failure to decide is not approval.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow { scope: PermissionGrantScope },
    Deny { reason: String },
}

/// Borrowed identity and actual prepared arguments of one tool invocation.
/// This view adds no user, workspace, or ambient authority. Arguments are not
/// the original provider input or a persisted archive projection.
#[derive(Clone, Copy)]
pub struct PermissionInvocation<'a> {
    pub tool_name: &'a ToolName,
    pub call_id: &'a ToolCallId,
    pub arguments: &'a Value,
}

impl fmt::Debug for PermissionInvocation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionInvocation")
            .finish_non_exhaustive()
    }
}

/// Optional final host-policy check immediately before initial tool execution.
/// Implementations must do only bounded synchronous work. Admission is consumed
/// once; drop without admission must release its owned state without effects.
/// An accepted admission cannot undo effects if policy changes afterward.
pub trait PermissionExecutionAdmission: Send + Sync + 'static {
    /// Consumes this admission, rejecting stale or revoked host authority.
    ///
    /// # Errors
    /// Returns a permission failure when execution must not start.
    fn admit(self: Box<Self>) -> Result<(), PermissionError>;
}

/// A normal policy decision and an optional owned execution-admission check.
/// The guard is never serialized, cloned, or invoked by debugging. A denied
/// decision discards its guard without admitting execution.
pub struct PermissionAuthorization {
    pub decision: PermissionDecision,
    pub admission: Option<Box<dyn PermissionExecutionAdmission>>,
}

impl PermissionAuthorization {
    /// Wraps an existing decision without changing its execution guarantees.
    #[must_use]
    pub const fn new(decision: PermissionDecision) -> Self {
        Self {
            decision,
            admission: None,
        }
    }

    /// Retains an explicitly supplied, one-shot execution-admission check.
    #[must_use]
    pub fn with_admission(mut self, admission: impl PermissionExecutionAdmission) -> Self {
        self.admission = Some(Box::new(admission));
        self
    }
}

impl fmt::Debug for PermissionAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionAuthorization")
            .finish_non_exhaustive()
    }
}

/// Object-safe host policy boundary for every privileged capability.
pub trait PermissionHandler: Send + Sync + 'static {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>>;

    /// Authorizes the actual prepared invocation without cloning its arguments.
    /// The default is inert until polled and delegates to the existing handler;
    /// it provides no new revocation or execution-admission guarantee.
    fn authorize_invocation<'a>(
        &'a self,
        request: PermissionRequest,
        _invocation: PermissionInvocation<'a>,
    ) -> BoxFuture<'a, Result<PermissionAuthorization, PermissionError>> {
        Box::pin(async move {
            self.authorize(request)
                .await
                .map(PermissionAuthorization::new)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, ProcessInput};
    use serde_json::json;

    #[test]
    fn process_input_has_a_closed_snake_case_representation() {
        assert_eq!(ProcessInput::default(), ProcessInput::Null);
        for (input, wire) in [(ProcessInput::Null, "null"), (ProcessInput::Pipe, "pipe")] {
            assert_eq!(serde_json::to_value(input).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<ProcessInput>(json!(wire)).unwrap(),
                input
            );
        }
        for invalid in [json!(null), json!("Pipe"), json!("inherit"), json!(true)] {
            assert!(serde_json::from_value::<ProcessInput>(invalid).is_err());
        }
    }

    #[test]
    fn legacy_process_capability_defaults_to_null_and_pipe_is_a_distinct_identity() {
        let legacy = json!({
            "type": "process",
            "program": "/bin/sh",
            "arguments": ["-c", "cat"],
            "working_directory": ".",
            "environment": {"profile": "fixed", "sha256": "0".repeat(64)}
        });
        let legacy_capability = serde_json::from_value::<Capability>(legacy.clone()).unwrap();
        assert!(matches!(
            &legacy_capability,
            Capability::Process {
                stdin: ProcessInput::Null,
                ..
            }
        ));

        let mut explicit_null = legacy.clone();
        explicit_null["stdin"] = json!("null");
        assert_eq!(
            legacy_capability,
            serde_json::from_value::<Capability>(explicit_null.clone()).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&legacy_capability).unwrap(),
            explicit_null
        );

        let mut explicit_pipe = legacy;
        explicit_pipe["stdin"] = json!("pipe");
        let pipe = serde_json::from_value::<Capability>(explicit_pipe.clone()).unwrap();
        assert_ne!(legacy_capability, pipe);
        assert_eq!(serde_json::to_value(pipe).unwrap(), explicit_pipe);
    }
}
