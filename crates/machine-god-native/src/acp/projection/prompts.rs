use std::fmt;

use machine_god_core::{ToolCall, ToolContext};
use serde_json::{Map, Value, json, value::RawValue};

use super::{ProjectionError as Error, checked, protocol};
use crate::{
    MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES, NativeInteractivePromptResponse,
    NativeInteractivePromptView, PermissionPromptDecision, QuestionPromptAnswers,
    QuestionPromptOutcome, QuestionPromptRequest,
    acp::interaction::NativeAcpElicitationId,
    mcp::{
        interaction::{
            MAX_MCP_ELICITATION_ANSWER_BYTES, McpElicitationAnswerInput,
            McpElicitationPromptRequest, McpElicitationPromptSource,
        },
        mrtr::McpElicitationMode,
    },
};

/// Expected reply shape, derived from the actual native inbox payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAcpReplyKind {
    Permission,
    ExecutionConsent,
    Question,
    Form,
    Url,
}

/// An admitted outbound body. It retains no native token or execution grant.
pub struct NativeAcpClientRequest {
    pub method: &'static str,
    pub params: Value,
    pub reply_kind: NativeAcpReplyKind,
}
impl fmt::Debug for NativeAcpClientRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeAcpClientRequest { <redacted> }")
    }
}

/// Project an exact native prompt for the ACP client.
///
/// URL IDs must come from the presenter's exact registration, never the peer.
/// The driver must validate that registration and retain the inbox token.
///
/// # Errors
/// Rejects unsupported sources, missing/extraneous URL registration and bounds.
pub fn project_prompt(
    view: &NativeInteractivePromptView,
    url_id: Option<NativeAcpElicitationId>,
) -> Result<NativeAcpClientRequest, Error> {
    if let Some(request) = view.elicitation() {
        return elicitation(request, url_id);
    }
    if url_id.is_some() {
        return Err(Error::InvalidSource);
    }
    if let Some((context, request)) = view.question() {
        return question(context, request);
    }
    if let Some(request) = view.execution_consent() {
        return execution_consent(request);
    }
    // A PermissionRequest alone has no actual tool-call identity. It cannot be
    // promoted to an ACP toolCall by inventing an ID from the permission ID.
    Err(Error::InvalidSource)
}

fn execution_consent(
    request: &crate::NativeExecutionConsentRequest,
) -> Result<NativeAcpClientRequest, Error> {
    if !request.is_live() {
        return Err(Error::InvalidSource);
    }
    super::bounded_text(request.reason())?;
    let call = request.call();
    protocol::validate_value(&call.arguments, 5).map_err(|_| Error::Limit)?;
    // The actual admitted call supplies identity; the frozen proposal supplies
    // exact generation/revision and old/new parent details for human review.
    let proposal = serde_json::to_value(request.capability()).map_err(|_| Error::Limit)?;
    let details = super::serialize_bounded(&proposal, protocol::ACP_MAX_FRAME_BYTES)?;
    Ok(NativeAcpClientRequest {
        method: "session/request_permission",
        params: checked(json!({
            "sessionId": request.context().session_id.as_str(),
            "toolCall": {
                "toolCallId": call.id.as_str(), "title": request.reason(),
                "kind": super::tool_kind(call.name.as_str()), "status": "pending",
                "rawInput": call.arguments,
                "content": [{"type":"content", "content":{"type":"text", "text":details}}]
            },
            "options": [
                {"optionId":"allow_once", "name":"Approve this exact proposal", "kind":"allow_once"},
                {"optionId":"reject_once", "name":"Reject", "kind":"reject_once"}
            ]
        }))?,
        reply_kind: NativeAcpReplyKind::ExecutionConsent,
    })
}

/// Project permission with its actual call from the native review context.
/// The driver must verify that context is live and matches the exact request;
/// an observed provider event or matching tool name alone is not provenance.
///
/// # Errors
/// Rejects non-permission views and over-budget actual call data.
pub fn project_permission(
    view: &NativeInteractivePromptView,
    call: &ToolCall,
) -> Result<NativeAcpClientRequest, Error> {
    let request = view.permission().ok_or(Error::InvalidSource)?;
    if let machine_god_core::Capability::Tool { name, call_id, .. } = &request.capability
        && (name != &call.name || call_id != &call.id)
    {
        return Err(Error::InvalidSource);
    }
    super::bounded_text(&request.reason)?;
    protocol::validate_value(&call.arguments, 5).map_err(|_| Error::Limit)?;
    let mut params = json!({
        "sessionId":request.session_id.as_str(),
        "toolCall":{
            "toolCallId":call.id.as_str(), "title":request.reason,
            "kind":super::tool_kind(call.name.as_str()), "status":"pending"
        },
        "options":[
            {"optionId":"allow_once","name":"Allow once","kind":"allow_once"},
            {"optionId":"allow_always","name":"Allow for this session","kind":"allow_always"},
            {"optionId":"reject_once","name":"Reject","kind":"reject_once"}
        ]
    });
    params["toolCall"]["rawInput"] = call.arguments.clone();
    Ok(NativeAcpClientRequest {
        method: "session/request_permission",
        params: checked(params)?,
        reply_kind: NativeAcpReplyKind::Permission,
    })
}

/// Decode client result data against the exact displayed native request.
///
/// This validation does not establish freshness. Only the driver's retained
/// native token, returned to `NativeInteractivePromptInbox::reply`, can settle
/// a still-live request. Stale and cross-session tokens remain invalid.
///
/// # Errors
/// Rejects malformed, over-budget, foreign-option and schema-invalid results.
pub fn decode_reply(
    view: &NativeInteractivePromptView,
    result: &Value,
) -> Result<NativeInteractivePromptResponse, Error> {
    protocol::validate_value(result, 3).map_err(|_| Error::Limit)?;
    if view.permission().is_some() {
        return permission_reply(result).map(NativeInteractivePromptResponse::Permission);
    }
    if view.execution_consent().is_some() {
        return match permission_reply(result)? {
            PermissionPromptDecision::AllowOnce => {
                Ok(NativeInteractivePromptResponse::ExecutionConsent(true))
            }
            PermissionPromptDecision::Deny => {
                Ok(NativeInteractivePromptResponse::ExecutionConsent(false))
            }
            PermissionPromptDecision::AllowTurn | PermissionPromptDecision::AllowSession => {
                Err(Error::InvalidResponse)
            }
        };
    }
    if let Some((_, request)) = view.question() {
        return question_reply(request, result).map(NativeInteractivePromptResponse::Question);
    }
    if let Some(request) = view.elicitation() {
        let fields = result.as_object().ok_or(Error::InvalidResponse)?;
        if fields.keys().any(|key| key != "action" && key != "content") {
            return Err(Error::InvalidResponse);
        }
        let expected_fields = match fields.get("action").and_then(Value::as_str) {
            Some("accept") if request.request().mode() == McpElicitationMode::Form => 2,
            Some("accept" | "decline" | "cancel") => 1,
            _ => return Err(Error::InvalidResponse),
        };
        if fields.len() != expected_fields {
            return Err(Error::InvalidResponse);
        }
        let raw = super::serialize_bounded(result, MAX_MCP_ELICITATION_ANSWER_BYTES)?;
        let raw = RawValue::from_string(raw).map_err(|_| Error::InvalidResponse)?;
        let (_, canonical) = request
            .request()
            .validate_response(&raw)
            .map_err(|_| Error::InvalidResponse)?;
        return McpElicitationAnswerInput::new(canonical)
            .map(NativeInteractivePromptResponse::Elicitation)
            .map_err(|_| Error::Limit);
    }
    // A client-managed endpoint has no local browser recovery operation.
    Err(Error::Unsupported)
}

fn permission_reply(result: &Value) -> Result<PermissionPromptDecision, Error> {
    let root = result.as_object().ok_or(Error::InvalidResponse)?;
    if root.len() != 1 {
        return Err(Error::InvalidResponse);
    }
    let outcome = root
        .get("outcome")
        .and_then(Value::as_object)
        .ok_or(Error::InvalidResponse)?;
    match outcome.get("outcome").and_then(Value::as_str) {
        Some("cancelled") if outcome.len() == 1 => Ok(PermissionPromptDecision::Deny),
        Some("selected") if outcome.len() == 2 => {
            match outcome.get("optionId").and_then(Value::as_str) {
                Some("allow_once") => Ok(PermissionPromptDecision::AllowOnce),
                Some("allow_always") => Ok(PermissionPromptDecision::AllowSession),
                Some("reject_once") => Ok(PermissionPromptDecision::Deny),
                _ => Err(Error::InvalidResponse),
            }
        }
        _ => Err(Error::InvalidResponse),
    }
}

fn elicitation(
    request: &McpElicitationPromptRequest,
    url_id: Option<NativeAcpElicitationId>,
) -> Result<NativeAcpClientRequest, Error> {
    let wire = request.request();
    let Value::Object(mut source) = protocol::decode_value(wire.raw_params_json().get().as_bytes())
        .map_err(|_| Error::Limit)?
    else {
        return Err(Error::InvalidSource);
    };
    let mut params = Map::new();
    match request.source() {
        McpElicitationPromptSource::ModelTool { context, .. } => {
            params.insert("sessionId".into(), json!(context.session_id.as_str()));
            params.insert("toolCallId".into(), json!(context.call_id.as_str()));
        }
        McpElicitationPromptSource::HumanFeature { owner, .. } => {
            params.insert("sessionId".into(), json!(owner.session_id().as_str()));
        }
    }
    // Prefix peer text with independently captured server identity and the
    // validated URL host, so consent is not attributed solely by peer wording.
    super::bounded_text(wire.message())?;
    let message = match wire.mode() {
        McpElicitationMode::Form => format!(
            "Machine God received a form request from MCP server {}. {}",
            request.server(),
            wire.message()
        ),
        McpElicitationMode::Url => format!(
            "Machine God received a URL request from MCP server {} for host {}. {}",
            request.server(),
            wire.url_host().unwrap_or("non-UTF-8 host"),
            wire.message()
        ),
    };
    params.insert("message".into(), Value::String(message));
    let reply_kind = match wire.mode() {
        McpElicitationMode::Form => {
            if url_id.is_some() {
                return Err(Error::InvalidSource);
            }
            let mut schema = source
                .remove("requestedSchema")
                .ok_or(Error::InvalidSource)?;
            // Bounded admission occurred before this traversal. ACP omits the
            // MCP dialect marker and historical enumNames presentation hint.
            strip_mcp_schema_annotations(&mut schema);
            params.insert("mode".into(), json!("form"));
            params.insert("requestedSchema".into(), schema);
            NativeAcpReplyKind::Form
        }
        McpElicitationMode::Url => {
            let id = url_id.ok_or(Error::InvalidSource)?;
            params.insert("mode".into(), json!("url"));
            params.insert("url".into(), json!(wire.url().ok_or(Error::InvalidSource)?));
            params.insert("elicitationId".into(), json!(id.to_string()));
            NativeAcpReplyKind::Url
        }
    };
    if let Some(metadata) = source.remove("_meta") {
        if !metadata.is_object() {
            return Err(Error::InvalidSource);
        }
        params.insert("_meta".into(), metadata);
    }
    Ok(NativeAcpClientRequest {
        method: "elicitation/create",
        params: checked(Value::Object(params))?,
        reply_kind,
    })
}

fn strip_mcp_schema_annotations(value: &mut Value) {
    if let Value::Object(object) = value {
        object.remove("$schema");
        object.remove("enumNames");
        // Property names and literal defaults are data, not schema keywords.
        if let Some(Value::Object(properties)) = object.get_mut("properties") {
            for child in properties.values_mut() {
                strip_mcp_schema_annotations(child);
            }
        }
        if let Some(items) = object.get_mut("items") {
            strip_mcp_schema_annotations(items);
        }
        for keyword in ["anyOf", "oneOf", "allOf"] {
            if let Some(Value::Array(variants)) = object.get_mut(keyword) {
                for child in variants {
                    strip_mcp_schema_annotations(child);
                }
            }
        }
    }
}

fn question(
    context: &ToolContext,
    request: &QuestionPromptRequest,
) -> Result<NativeAcpClientRequest, Error> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (index, question) in request.questions().iter().enumerate() {
        // Native normalized questions are already bounded (at most four).
        // Suggestions remain suggestions: ordinary questions permit free text.
        let mut description = String::new();
        for option in question.options() {
            if !description.is_empty() {
                description.push('\n');
            }
            description.push_str(option.label());
            if let Some(text) = option.description() {
                description.push_str(": ");
                description.push_str(text);
            }
        }
        let key = format!("question_{}", index + 1);
        properties.insert(
            key.clone(),
            json!({
                "type":"string", "title":question.question(),
                "description":description,
                "maxLength":MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES
            }),
        );
        required.push(Value::String(key));
    }
    Ok(NativeAcpClientRequest {
        method: "elicitation/create",
        params: checked(json!({
            "sessionId":context.session_id.as_str(), "toolCallId":context.call_id.as_str(),
            "mode":"form", "message":"Answer the agent's questions",
            "requestedSchema":{
                "type":"object", "properties":properties, "required":required,
                "additionalProperties":false
            }
        }))?,
        reply_kind: NativeAcpReplyKind::Question,
    })
}

fn question_reply(
    request: &QuestionPromptRequest,
    result: &Value,
) -> Result<QuestionPromptOutcome, Error> {
    let fields = result.as_object().ok_or(Error::InvalidResponse)?;
    match fields.get("action").and_then(Value::as_str) {
        Some("cancel" | "decline") if fields.len() == 1 => Ok(QuestionPromptOutcome::Cancelled),
        Some("accept") if fields.len() == 2 => {
            let content = fields
                .get("content")
                .and_then(Value::as_object)
                .ok_or(Error::InvalidResponse)?;
            if content.len() != request.questions().len() {
                return Err(Error::InvalidResponse);
            }
            let mut answers = QuestionPromptAnswers::new();
            let mut bytes = 0usize;
            for index in 0..request.questions().len() {
                let answer = content
                    .get(&format!("question_{}", index + 1))
                    .and_then(Value::as_str)
                    .ok_or(Error::InvalidResponse)?;
                bytes = bytes.checked_add(answer.len()).ok_or(Error::Limit)?;
                if bytes > MAX_ASK_USER_QUESTION_TOTAL_RAW_ANSWER_BYTES {
                    return Err(Error::Limit);
                }
                if answer.trim_matches([' ', '\t', '\r', '\n']).is_empty() {
                    return Err(Error::InvalidResponse);
                }
                answers
                    .try_push(answer.to_owned())
                    .map_err(|_| Error::Limit)?;
            }
            Ok(QuestionPromptOutcome::Answered(answers))
        }
        _ => Err(Error::InvalidResponse),
    }
}
