//! Complete model-facing terminal adapter. Native authority is injected, never
//! discovered during construction, preparation or creation of an execution future.

use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::session_store::JsonValueOwner;
use machine_god_core::{
    BoxFuture, CancellationToken, Capability, MAX_TERMINAL_ACTION_RESULTS,
    MAX_TERMINAL_ACTION_TEXT_BYTES, MAX_TERMINAL_HYPERLINK_BYTES, MAX_TERMINAL_HYPERLINKS,
    MAX_TERMINAL_SCREEN_CELLS, MAX_TERMINAL_SCREEN_TEXT_BYTES, PreparedToolCall,
    TerminalActionRequest, TerminalActionResult, TerminalMonitorCondition,
    TerminalMonitorOperation, TerminalWriteLeaseIntent, Tool, ToolCall, ToolContext, ToolError,
    ToolErrorKind, ToolExecution, ToolInputLimits, ToolOutput, ToolOutputLimits, ToolSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::terminal_action_parse::{
    MAX_TERMINAL_ACTION_ARGUMENT_BYTES, MAX_TERMINAL_ACTION_ARGUMENT_NODES, decode_terminal_action,
    terminal_action_input_schema, terminal_action_requested_cwd,
};

/// Bound for the normalized internal envelope, including its immutable host and
/// call identity. Provider input uses `MAX_TERMINAL_ACTION_ARGUMENT_BYTES` instead.
pub const MAX_TERMINAL_PREPARED_ARGUMENT_BYTES: usize =
    MAX_TERMINAL_ACTION_ARGUMENT_BYTES + 64 * 1024;

/// Complete-result JSON ceiling, not a screen truncation policy. The encoder
/// contributes at most six bytes per UTF-8 text byte and four per opaque byte.
/// A cell's non-text structure fits 512 bytes; hyperlink entries fit 64 bytes
/// plus URI encoding. 4096 bytes per facts/event/summary and a 4 MiB envelope
/// cover every non-screen action, including a 256-entry catalog and 1 MiB read.
/// Tests verify these structural margins against the actual Rust encoder.
pub const MAX_TERMINAL_ACTION_RESULT_BYTES: usize = 512 * MAX_TERMINAL_SCREEN_CELLS
    + 6 * MAX_TERMINAL_SCREEN_TEXT_BYTES
    + 4 * MAX_TERMINAL_HYPERLINK_BYTES
    + 64 * MAX_TERMINAL_HYPERLINKS
    + 4096 * MAX_TERMINAL_ACTION_RESULTS
    + 4 * 1024 * 1024;

/// Complete action result plus the compact `ToolOutput` envelope.
pub const MAX_TERMINAL_COMPLETE_TOOL_OUTPUT_BYTES: usize = MAX_TERMINAL_ACTION_RESULT_BYTES + 64;

/// Explicit owned-worker publication boundary for complete terminal results.
///
/// The publisher receives ownership without cloning the complete JSON value.
/// It must durably publish a lossless, session/incarnation/call-scoped archive
/// before returning `ToolExecution::with_persisted_output` with that same full
/// result and a small reference. An error must never advertise an uncommitted
/// reference. Construction of the returned future must be inert; effects and
/// retained worker ownership belong to the injected native implementation.
/// A committed terminal receipt must still finish publication after user
/// cancellation, so this post-execution boundary has no cancellation token.
pub trait TerminalActionResultPublisher: Send + Sync + 'static {
    fn publish(
        &self,
        context: ToolContext,
        output: ToolOutput,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>>;
}

/// Pre-execution publication of complete terminal arguments.
///
/// Implementations may archive the owned input but must never execute it. Return
/// `None` for inline input, or a bounded reference only after the complete input
/// is durably retrievable under the original context. Futures are inert before
/// polling, honour cancellation, and keep submitted workers owned on drop.
pub trait TerminalActionInputPublisher: Send + Sync + 'static {
    fn publish_arguments(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<Option<Value>, ToolError>>;
}

/// Immutable, non-secret host selection bound into every prepared capability.
/// Fingerprints identify the complete captured environment and shell resolver
/// inputs (including executable selection), not merely a profile name.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalActionHostIdentity {
    /// Absolute trusted workspace spelling. The executor must verify native containment.
    pub workspace: String,
    /// Absolute default directory captured by the host.
    pub default_cwd: String,
    /// Lowercase SHA-256 of the host's complete environment snapshot.
    pub environment_sha256: String,
    /// Lowercase SHA-256 of the host's immutable shell-selection inputs.
    pub shell_selection_sha256: String,
}

impl std::fmt::Debug for TerminalActionHostIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalActionHostIdentity")
            .finish_non_exhaustive()
    }
}

impl TerminalActionHostIdentity {
    fn validate(&self) -> Result<(), ToolError> {
        for path in [&self.workspace, &self.default_cwd] {
            if !path.starts_with('/')
                || path.len() > MAX_TERMINAL_ACTION_TEXT_BYTES
                || path.contains('\0')
            {
                return Err(invalid());
            }
        }
        for digest in [&self.environment_sha256, &self.shell_selection_sha256] {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// Authorized asynchronous native action dispatcher.
///
/// Implementations derive owner/incarnation solely from `context`, and choose
/// actor/writer identities themselves. They resolve the original requested cwd
/// against their captured workspace on the worker, preserve shell/environment
/// selection, and authorize the exact repeated monitor effects represented by
/// the prepared capability. A persisted PID or descriptive control is never authority.
/// Cancellation before submission prevents effects; after commitment return the
/// actual receipt (including accepted writes), not a blind cancellation error.
pub trait TerminalActionExecutor: Send + Sync + 'static {
    /// Executes one already prepared and authorized normalized request.
    fn execute(
        &self,
        context: ToolContext,
        invocation: TerminalActionInvocation,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>>;
}

/// Effect-free normalized action awaiting worker-owned cwd resolution.
/// The private command draft uses an inert placeholder, never an executable
/// working directory. Only `resolve_cwd` releases a complete command request.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalActionInvocation {
    draft: TerminalActionRequest,
    requested_cwd: Option<String>,
}

impl std::fmt::Debug for TerminalActionInvocation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalActionInvocation")
            .finish_non_exhaustive()
    }
}

impl TerminalActionInvocation {
    /// Whether catalog selection needs worker-owned workspace-path resolution.
    #[must_use]
    pub fn has_workspace_filter(&self) -> bool {
        matches!(&self.draft, TerminalActionRequest::List { filters } if filters.workspace_root.is_some())
    }

    /// Resolves a catalog predicate separately from command-directory authority.
    /// The native host invokes this only on its owned effect worker.
    ///
    /// # Errors
    /// Propagates resolution failures or rejects malformed invocation/filter data.
    pub fn resolve_workspace_filter(
        mut self,
        resolve: impl FnOnce(&str) -> Result<String, ToolError>,
    ) -> Result<Self, ToolError> {
        self.validate()?;
        if let TerminalActionRequest::List { filters } = &mut self.draft
            && let Some(raw) = &filters.workspace_root
        {
            let canonical = resolve(raw)?;
            if !canonical.starts_with('/') {
                return Err(invalid());
            }
            filters.workspace_root = Some(canonical);
        }
        self.validate()?;
        Ok(self)
    }

    /// Returns the action without releasing unresolved command data.
    #[must_use]
    pub const fn action(&self) -> machine_god_core::TerminalAction {
        self.draft.action()
    }

    /// Returns the exact target session, if this action has one.
    #[must_use]
    pub const fn session_id(&self) -> Option<&machine_god_core::TerminalSessionId> {
        self.draft.session_id()
    }

    fn validate(&self) -> Result<(), ToolError> {
        self.draft.validate().map_err(|_| invalid())?;
        let command_cwd = match &self.draft {
            TerminalActionRequest::Exec { request } => Some(request.cwd.as_str()),
            TerminalActionRequest::Start { request } => Some(request.cwd.as_str()),
            _ => None,
        };
        match (command_cwd, &self.requested_cwd) {
            (Some("/"), Some(raw))
                if !raw.is_empty()
                    && raw.len() <= MAX_TERMINAL_ACTION_TEXT_BYTES
                    && !raw.contains('\0') =>
            {
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err(invalid()),
        }
    }

    /// Resolves the original cwd exactly once, only for exec/start, on the native
    /// worker. The resolver uses the immutable host default/workspace,
    /// establishes containment and preserves native symlink/parent semantics.
    ///
    /// # Errors
    /// Propagates resolution failure or rejects a malformed canonical result.
    pub fn resolve_cwd(
        mut self,
        resolve: impl FnOnce(&str) -> Result<String, ToolError>,
    ) -> Result<TerminalActionRequest, ToolError> {
        self.validate()?;
        if let Some(raw) = &self.requested_cwd {
            let canonical = resolve(raw)?;
            match &mut self.draft {
                TerminalActionRequest::Exec { request } => request.cwd = canonical,
                TerminalActionRequest::Start { request } => request.cwd = canonical,
                _ => return Err(invalid()),
            }
        }
        self.draft.validate().map_err(|_| invalid())?;
        Ok(self.draft)
    }
}

/// All twelve terminal actions sharing one immutable native host boundary.
pub struct TerminalActionTool {
    executor: Arc<dyn TerminalActionExecutor>,
    identity: TerminalActionHostIdentity,
    publisher: Option<Arc<dyn TerminalActionResultPublisher>>,
    input_publisher: Option<Arc<dyn TerminalActionInputPublisher>>,
}

impl std::fmt::Debug for TerminalActionTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalActionTool")
            .field("has_result_publisher", &self.publisher.is_some())
            .field("has_input_publisher", &self.input_publisher.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    version: u8,
    call_id: machine_god_core::ToolCallId,
    host: TerminalActionHostIdentity,
    invocation: TerminalActionInvocation,
}

impl TerminalActionTool {
    /// Constructs an inert adapter; no filesystem or environment discovery occurs.
    ///
    /// # Errors
    /// Rejects malformed host identity; native identity verification belongs to execution.
    pub fn new(
        executor: Arc<dyn TerminalActionExecutor>,
        identity: TerminalActionHostIdentity,
    ) -> Result<Self, ToolError> {
        identity.validate()?;
        Ok(Self {
            executor,
            identity,
            publisher: None,
            input_publisher: None,
        })
    }

    /// Injects an inert durable-result publisher for engine orchestration.
    /// Direct `execute` still returns the complete output without publication.
    #[must_use]
    pub fn with_result_publisher(
        mut self,
        publisher: Arc<dyn TerminalActionResultPublisher>,
    ) -> Self {
        self.publisher = Some(publisher);
        self
    }

    /// Injects pre-execution durable input publication. This does not authorize
    /// an action or expand the host's independent per-turn input budget.
    #[must_use]
    pub fn with_input_publisher(
        mut self,
        publisher: Arc<dyn TerminalActionInputPublisher>,
    ) -> Self {
        self.input_publisher = Some(publisher);
        self
    }

    fn normalize(arguments: &Value) -> Result<TerminalActionInvocation, ToolError> {
        let raw = terminal_action_requested_cwd(arguments).map_err(|_| invalid())?;
        let draft = decode_terminal_action(arguments, "/").map_err(|_| invalid())?;
        let requested_cwd = matches!(
            draft,
            TerminalActionRequest::Exec { .. } | TerminalActionRequest::Start { .. }
        )
        .then(|| raw.map_or_else(|| ".".to_owned(), std::borrow::Cow::into_owned));
        let invocation = TerminalActionInvocation {
            draft,
            requested_cwd,
        };
        invocation.validate()?;
        Ok(invocation)
    }
}

impl Tool for TerminalActionTool {
    fn complete_input_limits(&self) -> Option<ToolInputLimits> {
        self.input_publisher.as_ref().map(|_| ToolInputLimits {
            max_argument_bytes: NonZeroUsize::new(MAX_TERMINAL_ACTION_ARGUMENT_BYTES)
                .expect("fixed nonzero ceiling"),
            max_argument_nodes: NonZeroUsize::new(MAX_TERMINAL_ACTION_ARGUMENT_NODES)
                .expect("fixed nonzero ceiling"),
            max_prepared_argument_bytes: NonZeroUsize::new(MAX_TERMINAL_PREPARED_ARGUMENT_BYTES)
                .expect("fixed nonzero ceiling"),
            max_prepared_argument_nodes: NonZeroUsize::new(
                MAX_TERMINAL_ACTION_ARGUMENT_NODES + 128,
            )
            .expect("fixed nonzero ceiling"),
        })
    }

    fn persist_arguments<'a>(
        &'a self,
        context: ToolContext,
        arguments: &'a Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Value>, ToolError>> {
        Box::pin(async move {
            let Some(publisher) = &self.input_publisher else {
                return Ok(None);
            };
            // Validate before the one bounded clone required to hand ownership
            // to a 'static worker; direct calls do not inherit core admission.
            crate::tool_output_serializer::measure_json_value_compact(
                arguments,
                crate::tool_output_serializer::CompactToolOutputLimits {
                    output_bytes: MAX_TERMINAL_ACTION_ARGUMENT_BYTES,
                    json_depth: machine_god_core::MAX_SAFE_JSON_DEPTH,
                    json_nodes: MAX_TERMINAL_ACTION_ARGUMENT_NODES,
                },
                &cancellation,
            )
            .map_err(|_| {
                if cancellation.is_cancelled() {
                    ToolError::new(
                        ToolErrorKind::Cancelled,
                        "terminal_cancelled",
                        "terminal input publication cancelled",
                        false,
                    )
                } else {
                    invalid()
                }
            })?;
            publisher
                .publish_arguments(context, arguments.clone(), cancellation)
                .await
        })
    }

    fn complete_output_limits(&self) -> Option<ToolOutputLimits> {
        self.publisher.as_ref().map(|_| ToolOutputLimits {
            max_serialized_bytes: NonZeroUsize::new(MAX_TERMINAL_COMPLETE_TOOL_OUTPUT_BYTES)
                .expect("fixed nonzero ceiling"),
            // Every JSON node requires at least one serialized byte. The closed
            // typed action contract imposes the tighter structural limits.
            max_json_nodes: NonZeroUsize::new(MAX_TERMINAL_COMPLETE_TOOL_OUTPUT_BYTES)
                .expect("fixed nonzero ceiling"),
        })
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: machine_god_core::ToolName::new("terminal").expect("fixed tool name"),
            description: "Execute foreground commands or manage durable interactive native PTY/tmux sessions. Actions: exec, start, read, screen, write, wait, monitor, inspect, list, resize, signal, close. Sessions belong to the current host session incarnation. Profiles default to user; clean is explicit. Raw bytes, styled screens, write receipts and retained history are preserved. Periodic monitor probes require explicit authorization.".to_owned(),
            input_schema: terminal_action_input_schema(),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        let arguments = JsonValueOwner::new(call.arguments);
        if call.name.as_str() != "terminal" {
            return Err(invalid());
        }
        let invocation = Self::normalize(arguments.get())?;
        let authority = authority_name(&invocation.draft);
        let effects = probe_authorities(&invocation.draft);
        let canonical = serde_json::to_value(Prepared {
            version: 1,
            call_id: call.id,
            host: self.identity.clone(),
            invocation,
        })
        .map_err(|_| invalid())?;
        let digest = digest_value(&canonical, MAX_TERMINAL_PREPARED_ARGUMENT_BYTES)?;
        let capability = Capability::Custom {
            name: authority.to_owned(),
            details: json!({
                "version": 1, "request_sha256": digest,
                "invocation": canonical["invocation"], "call_id": canonical["call_id"],
                "host": canonical["host"], "repeated_probe_authorities": effects,
            }),
        };
        // Even reads/waits can commit acknowledgement or attention changes.
        // The executor owns cancellation once first polled and must settle them.
        Ok(PreparedToolCall::new(capability, canonical).completion_wins_after_first_poll())
    }

    fn execute(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        let arguments = JsonValueOwner::new(arguments);
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(ToolError::new(
                    ToolErrorKind::Cancelled,
                    "terminal_cancelled",
                    "terminal action cancelled before submission",
                    false,
                ));
            }
            digest_value(arguments.get(), MAX_TERMINAL_PREPARED_ARGUMENT_BYTES)?;
            let prepared: Prepared =
                serde_json::from_value(arguments.get().clone()).map_err(|_| invalid())?;
            if prepared.version != 1
                || prepared.host != self.identity
                || prepared.call_id != context.call_id
                || serde_json::to_value(&prepared).map_err(|_| invalid())? != *arguments.get()
            {
                return Err(invalid());
            }
            prepared.invocation.validate()?;
            if cancellation.is_cancelled() {
                return Err(ToolError::new(
                    ToolErrorKind::Cancelled,
                    "terminal_cancelled",
                    "terminal action cancelled before submission",
                    false,
                ));
            }
            let result = self
                .executor
                .execute(context, prepared.invocation.clone(), cancellation)
                .await?;
            result
                .validate_for(&prepared.invocation.draft)
                .map_err(|_| invalid_result())?;
            // Count before allocating the large JSON Value; the complete typed
            // contracts already enforce vector/text/resource limits.
            count_encoded(&result, MAX_TERMINAL_ACTION_RESULT_BYTES)
                .map_err(|_| invalid_result())?;
            Ok(ToolOutput::success(
                serde_json::to_value(result).map_err(|_| invalid_result())?,
            ))
        })
    }

    fn execute_for_turn(
        &self,
        context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let execution = self.execute(context.clone(), arguments, cancellation);
        Box::pin(async move {
            let output = execution.await?;
            match &self.publisher {
                Some(publisher) => publisher.publish(context, output).await,
                None => Ok(ToolExecution::output(output)),
            }
        })
    }
}

fn authority_name(request: &TerminalActionRequest) -> &'static str {
    match request {
        TerminalActionRequest::Exec { .. } => "terminal_exec",
        TerminalActionRequest::Start { .. } => "terminal_start",
        TerminalActionRequest::Read { .. } => "terminal_read",
        TerminalActionRequest::Screen { .. } => "terminal_screen",
        TerminalActionRequest::Write { request, .. }
            if request.lease == TerminalWriteLeaseIntent::Revoke =>
        {
            "terminal_close"
        }
        TerminalActionRequest::Write { .. } => "terminal_write",
        TerminalActionRequest::Wait { .. } => "terminal_wait",
        TerminalActionRequest::Monitor { .. } => "terminal_monitor",
        TerminalActionRequest::Inspect { .. } => "terminal_inspect",
        TerminalActionRequest::List { .. } => "terminal_list",
        TerminalActionRequest::Resize { .. } => "terminal_resize",
        TerminalActionRequest::Signal { .. } => "terminal_signal",
        TerminalActionRequest::Close { .. } => "terminal_close",
    }
}

fn probe_authorities(request: &TerminalActionRequest) -> Vec<&'static str> {
    let mut classes = std::collections::BTreeSet::new();
    let mut add = |condition: &TerminalMonitorCondition| match condition {
        TerminalMonitorCondition::TcpReady { .. } | TerminalMonitorCondition::HttpReady { .. } => {
            classes.insert("network");
        }
        TerminalMonitorCondition::PathExists { .. }
        | TerminalMonitorCondition::PathChanged { .. }
        | TerminalMonitorCondition::PathSize { .. } => {
            classes.insert("filesystem");
        }
        TerminalMonitorCondition::CustomProbe { .. } => {
            classes.insert("process");
        }
        _ => {}
    };
    match request {
        TerminalActionRequest::Start { request } => {
            for definition in &request.initial_monitors {
                add(&definition.condition);
            }
        }
        TerminalActionRequest::Monitor {
            operation:
                TerminalMonitorOperation::Add { definition }
                | TerminalMonitorOperation::Update { definition, .. },
            ..
        } => add(&definition.condition),
        // Resume can restart an already-authorized periodic definition. The host
        // must retain and revalidate its exact grant; this does not grant new targets.
        TerminalActionRequest::Monitor {
            operation: TerminalMonitorOperation::Resume { .. },
            ..
        } => {
            classes.insert("resume_existing_grant");
        }
        _ => {}
    }
    classes.into_iter().collect()
}

fn invalid() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "terminal_invalid_arguments",
        "invalid terminal action arguments",
        false,
    )
}
fn invalid_result() -> ToolError {
    ToolError::new(
        ToolErrorKind::Execution,
        "terminal_invalid_result",
        "terminal executor returned an invalid result",
        false,
    )
}

struct EncodingBudget {
    remaining: usize,
    digest: Option<Sha256>,
}
impl std::io::Write for EncodingBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("terminal encoding bound"))?;
        if let Some(digest) = &mut self.digest {
            digest.update(bytes);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn count_encoded(value: &impl Serialize, maximum: usize) -> Result<EncodingBudget, ToolError> {
    let mut budget = EncodingBudget {
        remaining: maximum,
        digest: None,
    };
    serde_json::to_writer(&mut budget, value).map_err(|_| invalid())?;
    Ok(budget)
}
fn digest_value(value: &Value, maximum: usize) -> Result<String, ToolError> {
    fn bounded(value: &Value, depth: usize, remaining: &mut usize) -> Result<(), ToolError> {
        *remaining = remaining.checked_sub(1).ok_or_else(invalid)?;
        if depth > 64 {
            return Err(invalid());
        }
        match value {
            Value::Array(values) => {
                for value in values {
                    bounded(value, depth + 1, remaining)?;
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    bounded(value, depth + 1, remaining)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    bounded(value, 0, &mut maximum.saturating_add(1))?;
    let mut budget = EncodingBudget {
        remaining: maximum,
        digest: Some(Sha256::new()),
    };
    serde_json::to_writer(&mut budget, value).map_err(|_| invalid())?;
    Ok(format!(
        "{:x}",
        budget.digest.expect("digest enabled").finalize()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    fn identity() -> TerminalActionHostIdentity {
        TerminalActionHostIdentity {
            workspace: "/workspace".into(),
            default_cwd: "/workspace".into(),
            environment_sha256: "a".repeat(64),
            shell_selection_sha256: "b".repeat(64),
        }
    }

    #[derive(Default)]
    struct Publisher {
        calls: Mutex<Vec<ToolContext>>,
        fail: AtomicBool,
    }
    impl TerminalActionResultPublisher for Publisher {
        fn publish(
            &self,
            context: ToolContext,
            output: ToolOutput,
        ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(context);
                if self.fail.load(Ordering::Relaxed) {
                    return Err(invalid_result());
                }
                Ok(ToolExecution::with_persisted_output(
                    output,
                    ToolOutput::success(json!({"handle":"test-durable-reference"})),
                ))
            })
        }
    }

    #[test]
    fn publisher_is_explicit_inert_and_preserves_post_commit_cancellation() {
        let executor = Arc::new(Executor::default());
        let publisher = Arc::new(Publisher::default());
        let tool = TerminalActionTool::new(executor.clone(), identity()).unwrap();
        assert!(tool.complete_output_limits().is_none());
        let tool = tool.with_result_publisher(publisher.clone());
        assert_eq!(
            tool.complete_output_limits()
                .unwrap()
                .max_serialized_bytes
                .get(),
            MAX_TERMINAL_COMPLETE_TOOL_OUTPUT_BYTES
        );
        let prepared = tool.prepare(call(json!({"action":"list"}))).unwrap();
        drop(tool.execute_for_turn(
            context(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        ));
        assert!(executor.calls.lock().unwrap().is_empty());
        assert!(publisher.calls.lock().unwrap().is_empty());

        executor.cancel_after_commit.store(true, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        let prepared = tool.prepare(call(json!({"action":"list"}))).unwrap();
        let execution = futures_executor::block_on(tool.execute_for_turn(
            context(),
            prepared.arguments().clone(),
            cancellation.clone(),
        ))
        .unwrap();
        assert!(cancellation.is_cancelled());
        assert_eq!(publisher.calls.lock().unwrap().as_slice(), &[context()]);
        assert_eq!(execution.tool_output().content["action"], "list");
        assert_eq!(
            execution.persisted_output().unwrap().content["handle"],
            "test-durable-reference"
        );

        publisher.fail.store(true, Ordering::Relaxed);
        let prepared = tool.prepare(call(json!({"action":"list"}))).unwrap();
        assert!(
            futures_executor::block_on(tool.execute_for_turn(
                context(),
                prepared.arguments().clone(),
                CancellationToken::new()
            ))
            .is_err()
        );
        let prepared = tool.prepare(call(json!({"action":"list"}))).unwrap();
        assert!(
            futures_executor::block_on(tool.execute(
                context(),
                prepared.arguments().clone(),
                CancellationToken::new()
            ))
            .is_ok()
        );
        assert_eq!(publisher.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn deep_argument_owner_child() {
        let Ok(mode) = std::env::var("MACHINE_GOD_ACTION_DEEP_ARGUMENT_TEST") else {
            return;
        };
        let tool = TerminalActionTool::new(Arc::new(Executor::default()), identity()).unwrap();
        let mut value = Value::Null;
        for _ in 0..50_000 {
            value = Value::Array(vec![value]);
        }
        match mode.as_str() {
            "prepare" => {
                assert!(tool.prepare(call(value)).is_err());
            }
            "wrong-tool" => {
                let mut request = call(value);
                request.name = ToolName::new("other").unwrap();
                assert!(tool.prepare(request).is_err());
            }
            "unpolled" => {
                drop(tool.execute(context(), value, CancellationToken::new()));
            }
            "turn-unpolled" => {
                drop(tool.execute_for_turn(context(), value, CancellationToken::new()));
            }
            "cancelled" | "rejected" => {
                let cancellation = CancellationToken::new();
                if mode == "cancelled" {
                    cancellation.cancel();
                }
                assert!(
                    futures_executor::block_on(tool.execute(context(), value, cancellation))
                        .is_err()
                );
            }
            _ => panic!("unknown child mode"),
        }
    }

    #[test]
    fn rejected_and_unpolled_action_arguments_drop_without_recursion() {
        for mode in [
            "prepare",
            "wrong-tool",
            "unpolled",
            "turn-unpolled",
            "cancelled",
            "rejected",
        ] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "terminal_action_tool::tests::deep_argument_owner_child",
                    "--test-threads=1",
                ])
                .env("MACHINE_GOD_ACTION_DEEP_ARGUMENT_TEST", mode)
                .status()
                .unwrap();
            assert!(status.success(), "deep argument mode {mode}");
        }
    }
    fn context() -> ToolContext {
        ToolContext {
            session_id: SessionId::new("owner").unwrap(),
            session_incarnation_id: SessionIncarnationId::new("incarnation").unwrap(),
            turn_id: TurnId::new("turn").unwrap(),
            call_id: ToolCallId::new("call").unwrap(),
        }
    }
    fn call(arguments: Value) -> ToolCall {
        ToolCall {
            name: ToolName::new("terminal").unwrap(),
            id: ToolCallId::new("call").unwrap(),
            arguments,
        }
    }
    fn facts() -> TerminalSessionFacts {
        TerminalSessionFacts {
            session_id: TerminalSessionId::new("s").unwrap(),
            lifecycle: TerminalLifecycle::Running,
            attention: TerminalAttentionState::default(),
            backend: TerminalBackend::Native,
            persistence: TerminalPersistenceLevel::Durable,
            output_cursor: TerminalCursor::new(1, 0).unwrap(),
            unread_range: None,
            raw_gap: None,
            screen_recovery: TerminalScreenRecovery::Unavailable {
                reason: TerminalScreenUnavailableReason::Missing,
            },
            active_monitor_count: 0,
            next_actions: TerminalAllowedControls::default(),
        }
    }
    fn screen() -> TerminalScreen {
        TerminalScreen {
            dimensions: TerminalDimensions::new(1, 1).unwrap(),
            cursor: TerminalScreenCursor {
                row: 0,
                column: 0,
                visible: true,
                shape: TerminalCursorShape::Block,
                blinking: false,
            },
            modes: TerminalModes::default(),
            cells: vec![TerminalCell {
                kind: TerminalCellKind::Blank,
                text: String::new(),
                style: TerminalCellStyle::default(),
                hyperlink_id: None,
            }],
            hyperlinks: Vec::new(),
        }
    }
    fn response(request: &TerminalActionRequest) -> TerminalActionResult {
        let session = facts();
        match request {
            TerminalActionRequest::Exec { .. } => TerminalActionResult::Exec {
                result: TerminalExecResult {
                    status: TerminalExecStatus::Exited { exit_code: 7 },
                    stdout: TerminalExecCapturedOutput {
                        bytes: vec![0, 255],
                        total_bytes: 2,
                    },
                    stderr: TerminalExecCapturedOutput {
                        bytes: vec![],
                        total_bytes: 0,
                    },
                    duration: std::time::Duration::ZERO,
                },
            },
            TerminalActionRequest::Start { request } => {
                let mut session = session;
                session.backend = request.backend;
                TerminalActionResult::Start {
                    session,
                    outcome: TerminalReturnOutcome::Started {},
                }
            }
            TerminalActionRequest::Read { .. } => TerminalActionResult::Read {
                session,
                output: vec![0, 255],
                raw_range: None,
            },
            TerminalActionRequest::Screen { .. } => TerminalActionResult::Screen {
                session,
                snapshot: screen(),
            },
            TerminalActionRequest::Write { request, .. } => TerminalActionResult::Write {
                session,
                accepted_bytes: if request.payload.is_some() { 2 } else { 0 },
            },
            TerminalActionRequest::Wait { .. } => TerminalActionResult::Wait {
                session,
                outcome: TerminalReturnOutcome::SafetyCeiling {},
            },
            TerminalActionRequest::Monitor { .. } => TerminalActionResult::Monitor {
                session,
                monitor_id: None,
            },
            TerminalActionRequest::Inspect { .. } => TerminalActionResult::Inspect {
                session,
                shell: "/bin/bash".into(),
                cwd: "/workspace".into(),
                command: None,
                monitors: vec![],
                events: vec![],
                event_gap_through: 0,
                next_event_id: 1,
            },
            TerminalActionRequest::List { .. } => TerminalActionResult::List {
                sessions: vec![session],
            },
            TerminalActionRequest::Resize { dimensions, .. } => TerminalActionResult::Resize {
                session,
                dimensions: dimensions.clone(),
            },
            TerminalActionRequest::Signal { signal, .. } => TerminalActionResult::Signal {
                session,
                signal: *signal,
            },
            TerminalActionRequest::Close { policy, .. } => TerminalActionResult::Close {
                session,
                policy: *policy,
            },
        }
    }
    #[derive(Default)]
    struct Executor {
        calls: Mutex<Vec<(ToolContext, TerminalActionRequest)>>,
        wrong_result: AtomicBool,
        wrong_session: AtomicBool,
        cancel_after_commit: AtomicBool,
        large_read: AtomicBool,
        large_catalog: AtomicBool,
    }
    impl TerminalActionExecutor for Executor {
        fn execute(
            &self,
            context: ToolContext,
            invocation: TerminalActionInvocation,
            cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<TerminalActionResult, ToolError>> {
            // Deliberately observes invocation, not just polling; the adapter must
            // not even invoke an arbitrary executor until its own first poll.
            let request = invocation
                .resolve_cwd(|raw| {
                    Ok(if raw.starts_with('/') {
                        raw.to_owned()
                    } else {
                        format!("/workspace/{raw}")
                    })
                })
                .unwrap();
            self.calls.lock().unwrap().push((context, request.clone()));
            Box::pin(async move {
                let mut result = response(&request);
                if self.wrong_result.load(Ordering::SeqCst) {
                    result = TerminalActionResult::List { sessions: vec![] };
                }
                if self.wrong_session.load(Ordering::SeqCst) {
                    let mut value = serde_json::to_value(result).unwrap();
                    value["session"]["session_id"] = json!("other-session");
                    result = serde_json::from_value(value).unwrap();
                }
                if self.large_read.load(Ordering::SeqCst) {
                    result = TerminalActionResult::Read {
                        session: facts(),
                        output: vec![255; MAX_TERMINAL_ACTION_OUTPUT_BYTES],
                        raw_range: None,
                    };
                }
                if self.cancel_after_commit.load(Ordering::SeqCst) {
                    cancellation.cancel();
                }
                if self.large_catalog.load(Ordering::SeqCst) {
                    result = TerminalActionResult::List {
                        sessions: (0..MAX_TERMINAL_ACTION_RESULTS)
                            .map(|index| {
                                let mut session = facts();
                                session.session_id =
                                    TerminalSessionId::new(format!("session-{index}")).unwrap();
                                session
                            })
                            .collect(),
                    };
                }
                Ok(result)
            })
        }
    }
    fn tool() -> (TerminalActionTool, Arc<Executor>) {
        let executor = Arc::new(Executor::default());
        (
            TerminalActionTool::new(executor.clone(), identity()).unwrap(),
            executor,
        )
    }
    fn samples() -> Vec<Value> {
        vec![
            json!({"action":"exec","command":"printf hello"}),
            json!({"action":"start"}),
            json!({"action":"read","session_id":"s","cursor_segment":1}),
            json!({"action":"screen","session_id":"s"}),
            json!({"action":"write","session_id":"s","write":{"kind":"paste","text":"hi"}}),
            json!({"action":"wait","session_id":"s","return_when":{"kind":"exit"},"wait_ceiling_ms":100}),
            json!({"action":"monitor","session_id":"s","monitor":{"kind":"pause","monitor_id":"m"}}),
            json!({"action":"inspect","session_id":"s"}),
            json!({"action":"list"}),
            json!({"action":"resize","session_id":"s","rows":24,"columns":80}),
            json!({"action":"signal","session_id":"s","signal":"terminate"}),
            json!({"action":"close","session_id":"s","close_policy":"graceful"}),
        ]
    }
    #[test]
    fn all_twelve_actions_use_real_prepare_execute_and_preserve_results() {
        let (tool, executor) = tool();
        for arguments in samples() {
            let prepared = tool.prepare(call(arguments)).unwrap();
            assert!(prepared.capability().is_some());
            let expected: Prepared = serde_json::from_value(prepared.arguments().clone()).unwrap();
            let output = futures_executor::block_on(tool.execute(
                context(),
                prepared.arguments().clone(),
                CancellationToken::new(),
            ))
            .unwrap();
            assert!(!output.is_error);
            assert_eq!(
                output.content,
                serde_json::to_value(response(&expected.invocation.draft)).unwrap()
            );
        }
        let calls = executor.calls.lock().unwrap();
        assert_eq!(calls.len(), 12);
        assert!(calls.iter().all(|(actual, _)| actual == &context()));
        assert_eq!(
            tool.spec().input_schema["oneOf"].as_array().unwrap().len(),
            12
        );
    }
    #[test]
    fn constructors_prepare_and_unpolled_execution_are_inert() {
        let (tool, executor) = tool();
        let prepared = tool.prepare(call(json!({"action":"start"}))).unwrap();
        drop(tool.execute(
            context(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        ));
        assert!(executor.calls.lock().unwrap().is_empty());
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            futures_executor::block_on(tool.execute(
                context(),
                prepared.arguments().clone(),
                cancellation
            ))
            .unwrap_err()
            .kind,
            ToolErrorKind::Cancelled
        );
        assert!(executor.calls.lock().unwrap().is_empty());
    }
    #[test]
    fn canonical_identity_binds_payload_defaults_host_and_call() {
        let (tool, executor) = tool();
        let a = tool
            .prepare(call(
                json!({"action":"read","session_id":"s","cursor_segment":1}),
            ))
            .unwrap();
        let b = tool
            .prepare(call(
                json!({"action":"read","session_id":[115],"cursor_segment":"1","cursor_offset":0}),
            ))
            .unwrap();
        assert_eq!(a.capability(), b.capability());
        assert_eq!(a.arguments(), b.arguments());
        for field in [
            "environment_sha256",
            "shell_selection_sha256",
            "workspace",
            "default_cwd",
        ] {
            let mut altered = a.arguments().clone();
            altered["host"][field] = json!("different");
            assert!(
                futures_executor::block_on(tool.execute(
                    context(),
                    altered,
                    CancellationToken::new()
                ))
                .is_err()
            );
        }
        let mut other = context();
        other.call_id = ToolCallId::new("other").unwrap();
        assert!(
            futures_executor::block_on(tool.execute(
                other,
                a.arguments().clone(),
                CancellationToken::new()
            ))
            .is_err()
        );
        assert!(executor.calls.lock().unwrap().is_empty());
        let write_a = tool
            .prepare(call(
                json!({"action":"write","session_id":"s","write":{"kind":"text","text":"a"}}),
            ))
            .unwrap();
        let write_b = tool
            .prepare(call(
                json!({"action":"write","session_id":"s","write":{"kind":"text","text":"b"}}),
            ))
            .unwrap();
        assert_ne!(write_a.capability(), write_b.capability());
    }
    #[test]
    fn foreign_action_and_actor_fields_are_rejected_before_executor() {
        let (tool, executor) = tool();
        for field in ["actor", "writer", "owner", "session_incarnation_id"] {
            let mut arguments = json!({"action":"start"});
            arguments[field] = json!("human");
            assert!(tool.prepare(call(arguments)).is_err());
        }
        let mut invocation = call(json!({"action":"start"}));
        invocation.name = ToolName::new("other").unwrap();
        assert!(tool.prepare(invocation).is_err());
        assert!(executor.calls.lock().unwrap().is_empty());
    }
    #[test]
    fn invalid_results_are_not_published_and_commit_wins_cancellation() {
        let (tool, executor) = tool();
        let prepared = tool
            .prepare(call(
                json!({"action":"write","session_id":"s","write":{"kind":"paste","text":"ab"}}),
            ))
            .unwrap();
        executor.wrong_result.store(true, Ordering::SeqCst);
        assert_eq!(
            futures_executor::block_on(tool.execute(
                context(),
                prepared.arguments().clone(),
                CancellationToken::new()
            ))
            .unwrap_err()
            .code,
            "terminal_invalid_result"
        );
        executor.wrong_result.store(false, Ordering::SeqCst);
        executor.wrong_session.store(true, Ordering::SeqCst);
        assert_eq!(
            futures_executor::block_on(tool.execute(
                context(),
                prepared.arguments().clone(),
                CancellationToken::new()
            ))
            .unwrap_err()
            .code,
            "terminal_invalid_result"
        );
        executor.wrong_session.store(false, Ordering::SeqCst);
        executor.cancel_after_commit.store(true, Ordering::SeqCst);
        let cancellation = CancellationToken::new();
        let output = futures_executor::block_on(tool.execute(
            context(),
            prepared.arguments().clone(),
            cancellation.clone(),
        ))
        .unwrap();
        assert!(cancellation.is_cancelled());
        assert_eq!(output.content["accepted_bytes"], 2);
    }
    fn definition(kind: &str) -> Value {
        let mut condition = json!({"kind":kind});
        match kind {
            "custom_probe" => {
                condition["command"] = json!("x".repeat(MAX_TERMINAL_ACTION_COMMAND_BYTES));
                condition["cwd"] = json!("/workspace");
            }
            "tcp_ready" => {
                condition["host"] = json!("localhost");
                condition["port"] = json!(80);
            }
            "path_exists" => {
                condition["path"] = json!("a");
            }
            _ => panic!("fixture"),
        }
        json!({"condition":condition,"check_interval_ms":10,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_match"}})
    }
    #[test]
    fn revoke_and_all_periodic_probe_classes_have_explicit_authority() {
        let (tool, _) = tool();
        let prepared = tool
            .prepare(call(
                json!({"action":"write","session_id":"s","lease":"revoke"}),
            ))
            .unwrap();
        assert!(
            matches!(prepared.capability(),Some(Capability::Custom { name, .. }) if name == "terminal_close")
        );
        let prepared = tool.prepare(call(json!({"action":"start","initial_monitors":[definition("custom_probe"),definition("tcp_ready"),definition("path_exists")]}))).unwrap();
        match prepared.capability().unwrap() {
            Capability::Custom { details, .. } => assert_eq!(
                details["repeated_probe_authorities"],
                json!(["filesystem", "network", "process"])
            ),
            _ => panic!("custom capability"),
        }
    }
    #[test]
    fn full_command_initial_monitor_and_raw_output_bounds_survive_tool() {
        let (tool, executor) = tool();
        let mut monitor = definition("custom_probe");
        monitor["condition"]["command"] =
            json!("\u{0001}".repeat(MAX_TERMINAL_ACTION_COMMAND_BYTES));
        monitor["condition"]["cwd"] = json!(format!(
            "/{}",
            "\u{0001}".repeat(MAX_TERMINAL_ACTION_TEXT_BYTES - 1)
        ));
        let monitors = vec![monitor; MAX_TERMINAL_INITIAL_MONITORS];
        let prepared = tool.prepare(call(json!({"action":"start","command":"\u{0001}".repeat(MAX_TERMINAL_ACTION_COMMAND_BYTES),"initial_monitors":serde_json::to_string(&monitors).unwrap()}))).unwrap();
        futures_executor::block_on(tool.execute(
            context(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        ))
        .unwrap();
        let calls = executor.calls.lock().unwrap();
        assert!(
            matches!(&calls[0].1, TerminalActionRequest::Start { request } if request.command.as_ref().unwrap().len()==MAX_TERMINAL_ACTION_COMMAND_BYTES && request.initial_monitors.len()==MAX_TERMINAL_INITIAL_MONITORS)
        );
        drop(calls);
        executor.large_read.store(true, Ordering::SeqCst);
        let prepared = tool
            .prepare(call(
                json!({"action":"read","session_id":"s","cursor_segment":1}),
            ))
            .unwrap();
        let output = futures_executor::block_on(tool.execute(
            context(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(
            output.content["output"].as_array().unwrap().len(),
            MAX_TERMINAL_ACTION_OUTPUT_BYTES
        );
        executor.large_read.store(false, Ordering::SeqCst);
        executor.large_catalog.store(true, Ordering::SeqCst);
        let prepared = tool.prepare(call(json!({"action":"list"}))).unwrap();
        let output = futures_executor::block_on(tool.execute(
            context(),
            prepared.arguments().clone(),
            CancellationToken::new(),
        ))
        .unwrap();
        assert_eq!(
            output.content["sessions"].as_array().unwrap().len(),
            MAX_TERMINAL_ACTION_RESULTS
        );
    }
    #[test]
    fn cwd_spelling_preserves_native_symlink_semantics() {
        let (tool, _) = tool();
        let prepared = tool
            .prepare(call(
                json!({"action":"exec","command":"pwd","cwd":"symlink/../child"}),
            ))
            .unwrap();
        assert_eq!(
            prepared.arguments()["invocation"]["requested_cwd"],
            "symlink/../child"
        );
        let prepared: Prepared = serde_json::from_value(prepared.arguments().clone()).unwrap();
        let request = prepared
            .invocation
            .resolve_cwd(|raw| {
                assert_eq!(raw, "symlink/../child");
                Ok("/workspace/resolved-child".to_owned())
            })
            .unwrap();
        assert!(
            matches!(request, TerminalActionRequest::Exec { request } if request.cwd == "/workspace/resolved-child")
        );
    }

    #[test]
    fn workspace_filter_resolver_preserves_raw_predicate_and_never_resolves_a_command_cwd() {
        let (tool, _) = tool();
        let prepared = tool
            .prepare(call(json!({"action":"list","workspace_root":"link/.."})))
            .unwrap();
        let prepared: Prepared = serde_json::from_value(prepared.arguments().clone()).unwrap();
        assert!(prepared.invocation.has_workspace_filter());
        assert!(
            prepared
                .invocation
                .clone()
                .resolve_workspace_filter(|_| Ok("relative".into()))
                .is_err()
        );
        let invocation = prepared
            .invocation
            .resolve_workspace_filter(|raw| {
                assert_eq!(raw, "link/..");
                Ok("/workspace/real".into())
            })
            .unwrap();
        let request = invocation
            .resolve_cwd(|_| panic!("a predicate is not a command cwd"))
            .unwrap();
        assert!(
            matches!(request, TerminalActionRequest::List { filters } if filters.workspace_root.as_deref() == Some("/workspace/real"))
        );
        let prepared = tool.prepare(call(json!({"action":"list"}))).unwrap();
        let prepared: Prepared = serde_json::from_value(prepared.arguments().clone()).unwrap();
        assert!(!prepared.invocation.has_workspace_filter());
        prepared
            .invocation
            .resolve_workspace_filter(|_| panic!("unfiltered list must not resolve"))
            .unwrap();
    }

    #[test]
    fn maximum_relative_cwd_with_long_default_is_not_joined_or_resolved_in_prepare() {
        let executor = Arc::new(Executor::default());
        let mut host = identity();
        host.default_cwd = format!("/{}", "b".repeat(MAX_TERMINAL_ACTION_TEXT_BYTES - 1));
        let tool = TerminalActionTool::new(executor.clone(), host).unwrap();
        let raw = "c".repeat(MAX_TERMINAL_ACTION_TEXT_BYTES);
        let prepared = tool
            .prepare(call(json!({"action":"exec","command":"pwd","cwd":raw})))
            .unwrap();
        assert!(executor.calls.lock().unwrap().is_empty());
        let prepared: Prepared = serde_json::from_value(prepared.arguments().clone()).unwrap();
        let request = prepared
            .invocation
            .resolve_cwd(|received| {
                assert_eq!(received, raw);
                Ok("/workspace/native-canonical".into())
            })
            .unwrap();
        assert!(
            matches!(request,TerminalActionRequest::Exec { request } if request.cwd == "/workspace/native-canonical")
        );
        for arguments in samples().into_iter().skip(2) {
            let prepared = tool.prepare(call(arguments)).unwrap();
            let prepared: Prepared = serde_json::from_value(prepared.arguments().clone()).unwrap();
            assert!(
                prepared.invocation.session_id().is_some()
                    || prepared.invocation.action() == TerminalAction::List
            );
            prepared
                .invocation
                .resolve_cwd(|_| panic!("non-command must not resolve cwd"))
                .unwrap();
        }
    }
    #[test]
    fn encoder_bound_covers_maximum_screen_and_catalog_without_truncation() {
        let mut screen = screen();
        screen.dimensions = TerminalDimensions::new(512, 512).unwrap();
        let cell = TerminalCell {
            kind: TerminalCellKind::Single,
            text: "\0".repeat(32),
            style: TerminalCellStyle {
                foreground: TerminalColor::Rgb {
                    red: 255,
                    green: 255,
                    blue: 255,
                },
                background: TerminalColor::Rgb {
                    red: 255,
                    green: 255,
                    blue: 255,
                },
                ..TerminalCellStyle::default()
            },
            hyperlink_id: Some(u32::MAX),
        };
        let overhead = serde_json::to_vec(&cell).unwrap().len() - 6 * cell.text.len();
        assert!(overhead < 512);
        screen.cells = vec![cell; MAX_TERMINAL_SCREEN_CELLS];
        screen.hyperlinks = (1..=MAX_TERMINAL_HYPERLINKS)
            .map(|index| TerminalHyperlink {
                id: if index == MAX_TERMINAL_HYPERLINKS {
                    u32::MAX
                } else {
                    u32::try_from(index).unwrap()
                },
                uri: vec![
                    255;
                    if index == MAX_TERMINAL_HYPERLINKS {
                        128
                    } else {
                        64
                    }
                ],
            })
            .collect();
        screen.validate().unwrap();
        let result = TerminalActionResult::Screen {
            session: facts(),
            snapshot: screen,
        };
        count_encoded(&result, MAX_TERMINAL_ACTION_RESULT_BYTES).unwrap();
        let list = TerminalActionResult::List {
            sessions: vec![facts(); MAX_TERMINAL_ACTION_RESULTS],
        };
        list.validate().unwrap();
        count_encoded(&list, MAX_TERMINAL_ACTION_RESULT_BYTES).unwrap();
        assert!(serde_json::to_vec(&facts()).unwrap().len() < 4096);
    }
}
