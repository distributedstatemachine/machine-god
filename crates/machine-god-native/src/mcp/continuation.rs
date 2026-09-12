//! Native-only sealed response and exact human-answer custody. Codec data alone
//! cannot construct a public continuation capability.

use super::{
    interaction::{
        McpElicitationPresenter, McpElicitationPromptError, McpElicitationPromptRequest,
    },
    mrtr::{
        McpElicitationAction, McpElicitationMode, McpInputRequestPayload, McpValidatedResponses,
    },
    runtime::NativeMcpRuntimeToolCall,
    tool_result::{McpToolInputRequired, McpToolProtocolFailure},
};
use futures_util::future::{Either, select};
use machine_god_core::{CancellationToken, ToolError, ToolErrorKind, ToolOutput};
use serde_json::value::RawValue;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

pub(crate) enum AdmittedResponse {
    Complete(ToolOutput),
    ProtocolFailure(McpToolProtocolFailure),
    InputRequired(ContinuationInput),
}
pub(crate) struct ContinuationInput {
    pub(crate) required: Box<McpToolInputRequired>,
    pub(crate) round: Arc<AtomicBool>,
}
pub(crate) struct ContinuationConsent {
    pub(crate) input: ContinuationInput,
    pub(crate) responses: McpValidatedResponses,
}
pub(crate) enum FormOutcome {
    Consented(ContinuationConsent),
    Unresolved(ContinuationInput),
}

struct PromptCancellation(CancellationToken);
impl Drop for PromptCancellation {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) async fn collect_form(
    call: &NativeMcpRuntimeToolCall,
    input: ContinuationInput,
    presenter: &dyn McpElicitationPresenter,
) -> Result<FormOutcome, ToolError> {
    call.revalidate()?;
    call.check_interaction_deadline()?;
    let requests = input.required.required().requests();
    // The pinned responder rejects empty maps, including state-only responses.
    // Preflight all methods/modes before displaying any partial interaction.
    if call.protocol().version != super::protocol::ProtocolVersion::Modern
        || requests.is_empty()
        || requests.iter().any(|request| !matches!(request.payload(), McpInputRequestPayload::Elicitation(form) if form.mode() == McpElicitationMode::Form))
    {
        return Ok(FormOutcome::Unresolved(input));
    }
    let mut responses = BTreeMap::<&str, Box<RawValue>>::new();
    // Nonempty object: braces plus commas contribute one more byte than the
    // sum of each entry's key, value, colon and comma.
    let mut bytes = 1usize;
    let mut cancelled = false;
    for request in requests {
        call.revalidate()?;
        call.check_interaction_deadline()?;
        let McpInputRequestPayload::Elicitation(form) = request.payload() else {
            unreachable!("preflight")
        };
        let answer = if cancelled {
            RawValue::from_string("{\"action\":\"cancel\"}".into()).expect("fixed JSON")
        } else {
            let prompt = McpElicitationPromptRequest::new(
                call.context().clone(),
                Arc::from(call.server_name()),
                call.tool_name().clone(),
                form.clone(),
            )
            .map_err(|_| rejected())?;
            let cancellation = PromptCancellation(CancellationToken::new());
            let answer = match select(
                presenter.present(prompt, cancellation.0.clone()),
                call.interaction_cancelled(),
            )
            .await
            {
                Either::Left((answer, _)) => answer,
                Either::Right(_) => return Err(rejected()),
            };
            call.revalidate()?;
            call.check_interaction_deadline()?;
            match answer {
                Ok(answer) => {
                    cancelled = answer.action() == McpElicitationAction::Cancel;
                    answer.canonical_json().to_owned()
                }
                Err(McpElicitationPromptError::Cancelled) => return Err(rejected()),
                Err(_) => return Ok(FormOutcome::Unresolved(input)),
            }
        };
        // Charge the actual bounded escaped key before assembling the aggregate
        // map, then strictly validate the exact keyset and response shape.
        let key_bytes = serde_json::to_string(request.key())
            .map_err(|_| rejected())?
            .len();
        bytes = bytes
            .checked_add(key_bytes + answer.get().len() + 2)
            .ok_or_else(rejected)?;
        if bytes > 128 * 1024 {
            return Err(rejected());
        }
        responses.insert(request.key(), answer);
    }
    let json = serde_json::to_string(&responses).map_err(|_| rejected())?;
    let raw = RawValue::from_string(json).map_err(|_| rejected())?;
    let responses = input
        .required
        .required()
        .validate_responses(&raw)
        .map_err(|_| rejected())?;
    call.revalidate()?;
    call.check_interaction_deadline()?;
    Ok(FormOutcome::Consented(ContinuationConsent {
        input,
        responses,
    }))
}

fn rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_continuation_unavailable",
        "The MCP continuation is no longer available",
        false,
    )
}
