//! Consented URL handoff and at most three explicit recovery questions.

use super::{InputSource, PromptCancellation, rejected};
use crate::mcp::{
    browser_launcher::{NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLauncher},
    interaction::{
        McpElicitationAnswer, McpElicitationAnswerInput, McpElicitationPresenter,
        McpElicitationPromptError, McpUrlRecoveryAnswer, McpUrlRecoveryPromptRequest,
    },
    mrtr::{McpElicitationAction, McpElicitationRequest},
};
use futures_util::future::{Either, select};
use machine_god_core::{CancellationToken, ToolError};
use serde_json::value::RawValue;
use std::sync::Arc;

pub(super) async fn complete(
    call: &InputSource<'_>,
    request: &Arc<McpElicitationRequest>,
    consent: &McpElicitationAnswer,
    presenter: &dyn McpElicitationPresenter,
    launcher: &NativeMcpBrowserLauncher,
) -> Result<Option<McpElicitationAnswer>, ToolError> {
    if launch(call, request, consent, launcher).await? {
        return answer(request, McpElicitationAction::Accept).map(Some);
    }
    for _ in 0..3 {
        call.revalidate()?;
        call.check_interaction_deadline()?;
        let prompt = McpUrlRecoveryPromptRequest::new(call.prompt(request.clone())?)
            .map_err(|_| rejected())?;
        let cancellation = PromptCancellation(CancellationToken::new());
        let recovery = match select(
            presenter.recover_url(prompt, cancellation.0.clone()),
            call.interaction_cancelled(),
        )
        .await
        {
            Either::Left((answer, _)) => answer,
            Either::Right(_) => return Err(rejected()),
        };
        call.revalidate()?;
        call.check_interaction_deadline()?;
        match recovery {
            Ok(McpUrlRecoveryAnswer::ContinueManually) => {
                return answer(request, McpElicitationAction::Accept).map(Some);
            }
            Ok(McpUrlRecoveryAnswer::Cancel) => {
                return answer(request, McpElicitationAction::Cancel).map(Some);
            }
            Ok(McpUrlRecoveryAnswer::RetryBrowser) => {
                if launch(call, request, consent, launcher).await? {
                    return answer(request, McpElicitationAction::Accept).map(Some);
                }
            }
            Err(McpElicitationPromptError::Cancelled) => return Err(rejected()),
            Err(_) => return Ok(None),
        }
    }
    answer(request, McpElicitationAction::Cancel).map(Some)
}

async fn launch(
    call: &InputSource<'_>,
    request: &Arc<McpElicitationRequest>,
    consent: &McpElicitationAnswer,
    launcher: &NativeMcpBrowserLauncher,
) -> Result<bool, ToolError> {
    let cancellation = PromptCancellation(CancellationToken::new());
    let handoff = call.launch_url(request, consent, launcher, cancellation.0.clone())?;
    let result = match select(handoff, call.interaction_cancelled()).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => return Err(rejected()),
    };
    call.revalidate()?;
    call.check_interaction_deadline()?;
    Ok(matches!(result, Ok(NativeMcpBrowserLaunchOutcome::Opened)))
}

fn answer(
    request: &McpElicitationRequest,
    action: McpElicitationAction,
) -> Result<McpElicitationAnswer, ToolError> {
    let json = match action {
        McpElicitationAction::Accept => "{\"action\":\"accept\"}",
        McpElicitationAction::Cancel => "{\"action\":\"cancel\"}",
        McpElicitationAction::Decline => "{\"action\":\"decline\"}",
    };
    let raw = RawValue::from_string(json.into()).expect("fixed JSON");
    let input = McpElicitationAnswerInput::new(raw).map_err(|_| rejected())?;
    McpElicitationAnswer::validate(request, &input).map_err(|_| rejected())
}
