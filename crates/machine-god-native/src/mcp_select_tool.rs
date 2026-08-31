//! Bounded exact MCP selection with next-round executable advertisement.

use std::fmt;
use std::io;
use std::sync::Arc;

use machine_god_core::{
    BoxFuture, CancellationToken, PreparedToolCall, Tool, ToolCall, ToolContext, ToolError,
    ToolErrorKind, ToolExecution, ToolName, ToolOutput, ToolSpec, TurnToolRegistration,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::mcp_search_tools::{
    McpToolCatalog, McpToolCatalogError, McpToolCatalogErrorKind, McpToolCatalogSnapshot,
    McpToolCatalogState, acquire_catalog_snapshot, encode_model_scalar,
};

/// Registered name of [`McpSelectTool`].
pub const MCP_SELECT_TOOL_NAME: &str = "mcp_select_tool";
/// Maximum serialized canonical prepared arguments.
pub const MAX_MCP_SELECT_SERIALIZED_ARGUMENT_BYTES: usize = 512;
/// Maximum serialized complete [`ToolOutput`].
pub const MAX_MCP_SELECT_SERIALIZED_RESULT_BYTES: usize = 4 * 1024;

const DESCRIPTION: &str = "Exact-select one configured MCP/dynamic tool by name so its executable schema is advertised on the next model step. When to use: after discovering the exact specialized tool name in configured metadata. When NOT to use: guessing partial names, selecting built-in tools, or executing the dynamic tool directly.";

/// Exact-name selection over an injected immutable executable catalog.
pub struct McpSelectTool {
    catalog: Arc<dyn McpToolCatalog>,
}

impl McpSelectTool {
    /// Constructs the tool from one owned catalog implementation.
    #[must_use]
    pub fn new(catalog: impl McpToolCatalog) -> Self {
        Self {
            catalog: Arc::new(catalog),
        }
    }

    /// Constructs the tool over one explicitly shared catalog allocation.
    #[must_use]
    pub fn shared_catalog(catalog: Arc<dyn McpToolCatalog>) -> Self {
        Self { catalog }
    }
}

impl fmt::Debug for McpSelectTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpSelectTool")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SelectArguments {
    name: String,
}

impl SelectArguments {
    fn as_json(&self) -> Value {
        json!({"name": self.name})
    }
}

impl Tool for McpSelectTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: tool_name(),
            description: DESCRIPTION.to_owned(),
            input_schema: input_schema(),
        }
    }

    fn prepare(&self, call: ToolCall) -> Result<PreparedToolCall, ToolError> {
        if call.name != tool_name() {
            return Err(invalid_arguments());
        }
        let arguments = decode_arguments(&call.arguments)?;
        let canonical = arguments.as_json();
        ensure_serialized_arguments(&canonical)?;
        Ok(PreparedToolCall::without_authority(canonical))
    }

    fn execute(
        &self,
        _context: ToolContext,
        _arguments: Value,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolOutput, ToolError>> {
        Box::pin(async { Err(turn_orchestration_required()) })
    }

    fn execute_for_turn(
        &self,
        _context: ToolContext,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ToolExecution, ToolError>> {
        let selection = self.select(arguments, cancellation);
        Box::pin(async move {
            let selected = selection.await?;
            Ok(match selected.registration {
                Some(registration) => {
                    ToolExecution::with_next_round_tool(selected.output, registration)
                }
                None => ToolExecution::output(selected.output),
            })
        })
    }
}

struct Selection {
    output: ToolOutput,
    registration: Option<Arc<TurnToolRegistration>>,
}

impl McpSelectTool {
    fn select(
        &self,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<Selection, ToolError>> {
        Box::pin(async move {
            check_cancellation(&cancellation)?;
            let decoded = decode_arguments(&arguments)?;
            let canonical = decoded.as_json();
            ensure_serialized_arguments(&canonical)?;
            if canonical != arguments {
                return Err(invalid_arguments());
            }

            let snapshot = acquire_catalog_snapshot(
                self.catalog.as_ref(),
                &cancellation,
                map_catalog_error,
                cancelled,
            )
            .await?;
            check_cancellation(&cancellation)?;
            select_from_snapshot(&snapshot, &decoded.name, &cancellation)
        })
    }
}

fn tool_name() -> ToolName {
    ToolName::new(MCP_SELECT_TOOL_NAME).expect("mcp_select_tool is a valid tool name")
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "minLength": 1,
                "maxLength": 128,
                "pattern": "^[A-Za-z0-9_.:-]+$",
                "description": "Exact admitted dynamic MCP tool name"
            }
        },
        "required": ["name"],
    })
}

fn decode_arguments(arguments: &Value) -> Result<SelectArguments, ToolError> {
    let Value::Object(object) = arguments else {
        return Err(invalid_arguments());
    };
    let Some(Value::String(name)) = object.get("name") else {
        return Err(invalid_arguments());
    };
    ToolName::validate(name).map_err(|_| invalid_arguments())?;
    Ok(SelectArguments { name: name.clone() })
}

fn select_from_snapshot(
    snapshot: &McpToolCatalogSnapshot,
    requested_name: &str,
    cancellation: &CancellationToken,
) -> Result<Selection, ToolError> {
    let encoded_requested = encode_model_scalar(requested_name);
    if snapshot.state() == McpToolCatalogState::Discovering {
        return Ok(Selection {
            output: bounded_output(json!({
            "name": encoded_requested,
            "selected": false,
            "state": "discovering",
            "retryable": true,
            "schema_advertised": false,
            }))?,
            registration: None,
        });
    }

    for metadata in snapshot.tools() {
        check_cancellation(cancellation)?;
        if metadata.name() == requested_name {
            let Some(registration) = metadata.executable() else {
                return Err(not_found());
            };
            let name = encode_model_scalar(metadata.name());
            let output = format!(
                "Selected dynamic MCP tool `{name}`. Its executable schema will be available on the next model step; call `{name}` with arguments matching the selected schema."
            );
            check_cancellation(cancellation)?;
            return Ok(Selection {
                output: bounded_output(Value::String(output))?,
                registration: Some(registration),
            });
        }
    }
    check_cancellation(cancellation)?;
    Err(not_found())
}

fn bounded_output(content: Value) -> Result<ToolOutput, ToolError> {
    let output = ToolOutput::success(content);
    if serialized_value_fits(&output, MAX_MCP_SELECT_SERIALIZED_RESULT_BYTES) {
        Ok(output)
    } else {
        Err(resource_limit())
    }
}

fn ensure_serialized_arguments(arguments: &Value) -> Result<(), ToolError> {
    if serialized_value_fits(arguments, MAX_MCP_SELECT_SERIALIZED_ARGUMENT_BYTES) {
        Ok(())
    } else {
        Err(resource_limit())
    }
}

fn serialized_value_fits(value: &impl Serialize, limit: usize) -> bool {
    serde_json::to_writer(JsonByteCounter { written: 0, limit }, value).is_ok()
}

struct JsonByteCounter {
    written: usize,
    limit: usize,
}

impl io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(written) = self.written.checked_add(bytes.len()) else {
            return Err(io::Error::other("serialized JSON byte count overflowed"));
        };
        if written > self.limit {
            return Err(io::Error::other("serialized JSON exceeded its byte limit"));
        }
        self.written = written;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn map_catalog_error(error: McpToolCatalogError) -> ToolError {
    match error.kind() {
        McpToolCatalogErrorKind::Unavailable => unavailable(),
        McpToolCatalogErrorKind::ResourceLimit => resource_limit(),
        McpToolCatalogErrorKind::Cancelled => cancelled(),
    }
}

fn check_cancellation(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn invalid_arguments() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "mcp_select_tool_invalid_arguments",
        "mcp_select_tool arguments are invalid",
        false,
    )
}

fn not_found() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "mcp_select_tool_not_found",
        "the exact admitted MCP tool was not found",
        false,
    )
}

fn resource_limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "mcp_select_tool_resource_limit",
        "mcp_select_tool resource limit exceeded",
        false,
    )
}

fn unavailable() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_select_tool_unavailable",
        "MCP tool catalog is unavailable",
        true,
    )
}

fn turn_orchestration_required() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_select_tool_turn_orchestration_required",
        "mcp_select_tool requires turn orchestration",
        false,
    )
}

fn cancelled() -> ToolError {
    ToolError::new(
        ToolErrorKind::Cancelled,
        "mcp_select_tool_cancelled",
        "mcp_select_tool was cancelled",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_the_next_round_selection_boundary() {
        let tool = McpSelectTool::new(EmptyCatalog);
        let spec = tool.spec();
        assert_eq!(spec.name.as_str(), MCP_SELECT_TOOL_NAME);
        assert!(spec.input_schema.get("additionalProperties").is_none());
        assert_eq!(spec.input_schema["required"], json!(["name"]));
        assert!(spec.description.contains("next model step"));
    }

    struct EmptyCatalog;

    impl McpToolCatalog for EmptyCatalog {
        fn snapshot(
            &self,
            _cancellation: CancellationToken,
        ) -> BoxFuture<'_, Result<McpToolCatalogSnapshot, McpToolCatalogError>> {
            Box::pin(async {
                McpToolCatalogSnapshot::new(Vec::new())
                    .map_err(|_| McpToolCatalogError::new(McpToolCatalogErrorKind::ResourceLimit))
            })
        }
    }
}
