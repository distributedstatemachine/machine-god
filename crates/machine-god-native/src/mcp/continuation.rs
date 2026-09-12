//! Native-only sealed response and exact human-answer custody. Codec data alone
//! cannot construct a public continuation capability.

use super::{
    browser_launcher::NativeMcpBrowserLauncher,
    interaction::{McpElicitationPresenter, McpElicitationPromptError},
    mrtr::{
        McpElicitationAction, McpElicitationMode, McpInputRequestPayload, McpValidatedResponses,
    },
    runtime::{NativeMcpRuntimeFeatureCall, NativeMcpRuntimeToolCall},
    tool_result::{McpToolInputRequired, McpToolProtocolFailure},
};

mod source;
#[cfg(test)]
mod tests;
mod url;
use futures_util::future::{Either, select};
use machine_god_core::{CancellationToken, ToolError, ToolErrorKind, ToolOutput};
use serde_json::value::RawValue;
use source::InputSource;
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
pub(crate) enum InputOutcome {
    Consented(ContinuationConsent),
    Unresolved(ContinuationInput),
}

struct PromptCancellation(CancellationToken);
impl Drop for PromptCancellation {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) async fn collect_input(
    call: &NativeMcpRuntimeToolCall,
    input: ContinuationInput,
    presenter: &dyn McpElicitationPresenter,
    launcher: Option<&NativeMcpBrowserLauncher>,
) -> Result<InputOutcome, ToolError> {
    let responses = collect(
        &InputSource::Tool {
            call,
            input: &input,
        },
        presenter,
        launcher,
    )
    .await?;
    Ok(match responses {
        Some(responses) => InputOutcome::Consented(ContinuationConsent { input, responses }),
        None => InputOutcome::Unresolved(input),
    })
}

pub(crate) async fn collect_feature_input(
    call: &NativeMcpRuntimeFeatureCall,
    presenter: &dyn McpElicitationPresenter,
    launcher: Option<&NativeMcpBrowserLauncher>,
) -> Result<Option<McpValidatedResponses>, ToolError> {
    collect(&InputSource::Feature(call), presenter, launcher).await
}

async fn collect(
    call: &InputSource<'_>,
    presenter: &dyn McpElicitationPresenter,
    launcher: Option<&NativeMcpBrowserLauncher>,
) -> Result<Option<McpValidatedResponses>, ToolError> {
    call.revalidate()?;
    call.check_interaction_deadline()?;
    let required = call.required();
    let requests = required.requests();
    // The pinned responder rejects empty maps, including state-only responses.
    // Preflight all methods/modes before displaying any partial interaction.
    if !supported(required, launcher.is_some()) {
        return Ok(None);
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
            let prompt = call.prompt(form.clone())?;
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
                    let answer = if form.mode() == McpElicitationMode::Url
                        && answer.action() == McpElicitationAction::Accept
                    {
                        let Some(answer) = url::complete(
                            call,
                            form,
                            &answer,
                            presenter,
                            launcher.ok_or_else(rejected)?,
                        )
                        .await?
                        else {
                            return Ok(None);
                        };
                        answer
                    } else {
                        answer
                    };
                    cancelled = answer.action() == McpElicitationAction::Cancel;
                    answer.canonical_json().to_owned()
                }
                Err(McpElicitationPromptError::Cancelled) => return Err(rejected()),
                Err(_) => return Ok(None),
            }
        };
        // Charge the actual bounded escaped key before assembling the aggregate
        // map, then strictly validate the exact keyset and response shape.
        charge_response(&mut bytes, request.key(), &answer)?;
        responses.insert(request.key(), answer);
    }
    let json = serde_json::to_string(&responses).map_err(|_| rejected())?;
    let raw = RawValue::from_string(json).map_err(|_| rejected())?;
    let responses = required.validate_responses(&raw).map_err(|_| rejected())?;
    call.revalidate()?;
    call.check_interaction_deadline()?;
    Ok(Some(responses))
}

fn supported(required: &super::mrtr::McpInputRequired, has_launcher: bool) -> bool {
    !required.requests().is_empty()
        && required.requests().iter().all(|request| {
            matches!(request.payload(), McpInputRequestPayload::Elicitation(form)
                if form.mode() == McpElicitationMode::Form || has_launcher)
        })
}

fn charge_response(bytes: &mut usize, key: &str, answer: &RawValue) -> Result<(), ToolError> {
    let key_bytes = serde_json::to_string(key).map_err(|_| rejected())?.len();
    let total = bytes
        .checked_add(key_bytes)
        .and_then(|bytes| bytes.checked_add(answer.get().len()))
        .and_then(|bytes| bytes.checked_add(2))
        .filter(|bytes| *bytes <= 128 * 1024)
        .ok_or_else(rejected)?;
    *bytes = total;
    Ok(())
}

fn rejected() -> ToolError {
    ToolError::new(
        ToolErrorKind::Unavailable,
        "mcp_continuation_unavailable",
        "The MCP continuation is no longer available",
        false,
    )
}
