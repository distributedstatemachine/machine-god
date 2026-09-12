//! Typed URL choices only; the native collector owns effects and retry budgets.

use super::escaped;
use machine_god_native::{
    NativeInteractivePromptResponse,
    mcp::interaction::{
        McpLegacyUrlCompletionAnswer, McpLegacyUrlCompletionPromptRequest, McpUrlRecoveryAnswer,
        McpUrlRecoveryPromptRequest,
    },
};
use std::fmt::Write;

pub(crate) fn render_recovery(request: &McpUrlRecoveryPromptRequest) -> Result<Vec<u8>, ()> {
    render(
        request.source().server(),
        request.source().tool().as_str(),
        "Browser handoff was not confirmed. Continue manually, retry the browser, or cancel?\n[m] Continue manually  [r] Retry browser  [c] Cancel",
    )
}

pub(crate) fn render_completion(
    request: &McpLegacyUrlCompletionPromptRequest,
) -> Result<Vec<u8>, ()> {
    render(
        request.server(),
        request.tool().as_str(),
        "Complete the browser flow. The operation can continue automatically if the server confirms every URL request. Otherwise explicitly confirm completion to request a retry.\n[r] I completed it / Retry  [c] Cancel",
    )
}

fn render(server: &str, tool: &str, instructions: &str) -> Result<Vec<u8>, ()> {
    let mut output = super::super::bounded_output();
    output
        .write_str("\n[MCP URL input]\nServer: ")
        .map_err(|_| ())?;
    escaped(&mut output, server)?;
    output.write_str("\nTool: ").map_err(|_| ())?;
    escaped(&mut output, tool)?;
    output.write_char('\n').map_err(|_| ())?;
    output.write_str(instructions).map_err(|_| ())?;
    output
        .write_str("\n/cancel-input dismisses this request; /cancel stops the turn.\n> ")
        .map_err(|_| ())?;
    Ok(output.finish().into_bytes())
}

pub(crate) fn answer_recovery(line: &str) -> Result<NativeInteractivePromptResponse, ()> {
    let answer = match line.trim() {
        "m" => McpUrlRecoveryAnswer::ContinueManually,
        "r" => McpUrlRecoveryAnswer::RetryBrowser,
        "c" | "/cancel-input" => McpUrlRecoveryAnswer::Cancel,
        _ => return Err(()),
    };
    Ok(NativeInteractivePromptResponse::UrlRecovery(answer))
}

pub(crate) fn answer_completion(line: &str) -> Result<NativeInteractivePromptResponse, ()> {
    let answer = match line.trim() {
        "r" => McpLegacyUrlCompletionAnswer::Retry,
        "c" | "/cancel-input" => McpLegacyUrlCompletionAnswer::Cancel,
        _ => return Err(()),
    };
    Ok(NativeInteractivePromptResponse::LegacyUrlCompletion(answer))
}

#[cfg(test)]
mod tests;
