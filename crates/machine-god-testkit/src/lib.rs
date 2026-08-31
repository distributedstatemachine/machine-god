#![doc = include_str!("../../../docs/testkit.md")]
#![forbid(unsafe_code)]

mod event_sink;
mod permission;
mod provider;
mod session_store;
mod subagent;
mod tool;

pub use event_sink::{EventSinkStep, RecordingEventSink};
pub use permission::{PermissionStep, ScriptedPermissionHandler};
pub use provider::{
    ModelProviderStep, ModelStreamEnd, RecordedModelRequest, ScriptedModelProvider,
};
pub use session_store::{
    InMemorySessionStore, RecordedSessionStoreCall, SessionStoreScript, SessionStoreStep,
};
pub use subagent::{RecordedSubagentRequest, ScriptedSubagentAuthority, SubagentStep};
pub use tool::{
    RecordedToolInvocation, RecordedToolPreparation, ScriptedPreparedTool, ScriptedTool,
    ToolPrepareStep, ToolStep,
};

/// Testkit version aligned with the core API version.
pub const TESTKIT_API_VERSION: u32 = machine_god_core::API_VERSION;

/// Default upper bound for recorded calls or events retained by a double.
pub const DEFAULT_RECORD_CAPACITY: usize = 1_024;
