//! Typed URL choices only; the native collector owns effects and retry budgets.

use super::escaped;
use machine_god_native::{
    NativeInteractivePromptResponse,
    mcp::interaction::{McpUrlRecoveryAnswer, McpUrlRecoveryPromptRequest},
};
use std::fmt::Write;

pub(crate) fn render_recovery(request: &McpUrlRecoveryPromptRequest) -> Result<Vec<u8>, ()> {
    render(
        request.source().server(),
        request.source().source(),
        "Browser handoff was not confirmed. Continue manually, retry the browser, or cancel?\n[m] Continue manually  [r] Retry browser  [c] Cancel",
    )
}

fn render(
    server: &str,
    source: &super::McpElicitationPromptSource,
    instructions: &str,
) -> Result<Vec<u8>, ()> {
    let mut output = super::super::bounded_output();
    output
        .write_str("\n[MCP URL input]\nServer: ")
        .map_err(|_| ())?;
    escaped(&mut output, server)?;
    super::render_source(&mut output, source)?;
    output.write_char('\n').map_err(|_| ())?;
    output.write_str(instructions).map_err(|_| ())?;
    output
        .write_str("\n/cancel-input dismisses this request; ")
        .map_err(|_| ())?;
    output
        .write_str(super::cancellation_notice(source))
        .map_err(|_| ())?;
    output.write_str("\n> ").map_err(|_| ())?;
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

#[cfg(test)]
mod tests;
