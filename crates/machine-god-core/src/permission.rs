use crate::{
    BoxFuture, PermissionError, PermissionRequestId, SessionId, SessionIncarnationId, ToolCallId,
    ToolName, TurnId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// Object-safe host policy boundary for every privileged capability.
pub trait PermissionHandler: Send + Sync + 'static {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>>;
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
