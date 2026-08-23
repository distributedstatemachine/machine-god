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
    Process {
        program: String,
        arguments: Vec<String>,
    },
    Network {
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
