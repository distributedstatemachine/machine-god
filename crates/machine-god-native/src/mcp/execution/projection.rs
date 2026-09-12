use super::{ToolError, ToolOutput, Value, invalid_response};
use crate::mcp::tool_result::{McpToolInputRequired, McpToolProtocolFailure};

pub(super) fn continuation_exhausted() -> ToolOutput {
    ToolOutput {
        content: serde_json::json!({"resultType":"protocol_failure", "error":{"message":"MCP protocol failure: input-required continuation limit exceeded"}}),
        is_error: true,
    }
}

pub(super) fn protocol_failure(failure: &McpToolProtocolFailure) -> Result<ToolOutput, ToolError> {
    let error = machine_god_core::json::from_str(failure.raw_json().get())
        .map_err(|_| invalid_response())?;
    let mut content = serde_json::Map::new();
    content.insert(
        "resultType".into(),
        Value::String("protocol_failure".into()),
    );
    content.insert("error".into(), error);
    Ok(ToolOutput {
        content: Value::Object(content),
        is_error: true,
    })
}

pub(super) fn input_required(required: &McpToolInputRequired) -> Result<ToolOutput, ToolError> {
    let required = required.required();
    let requests = required
        .render_requests_json()
        .map_err(|_| invalid_response())?;
    let requests =
        machine_god_core::json::from_str(requests.get()).map_err(|_| invalid_response())?;
    let mut content = serde_json::Map::new();
    content.insert("resultType".into(), Value::String("input_required".into()));
    content.insert("inputRequests".into(), requests);
    if let Some(state) = required.request_state_json() {
        let state =
            machine_god_core::json::from_str(state.get()).map_err(|_| invalid_response())?;
        content.insert("requestState".into(), state);
    }
    Ok(ToolOutput {
        content: Value::Object(content),
        is_error: true,
    })
}
