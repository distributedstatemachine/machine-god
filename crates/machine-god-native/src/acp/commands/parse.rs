use super::{Action, NativeAcpCommand, NativeAcpCommandError as Error};
use crate::acp::prompt::NativeAcpPrompt;
use crate::{NativeSlashCommand as Command, NativeSlashRoute, route_native_slash};

/// Ordinary prompts remain unchanged. Invalid or unavailable local commands are
/// explicit errors, never instructions silently forwarded to a provider.
/// # Errors
/// Rejects malformed, unsupported or oversized local commands.
pub fn classify(prompt: &NativeAcpPrompt) -> Result<Option<NativeAcpCommand>, Error> {
    classify_text(&prompt.prompt().text)
}

pub(super) fn classify_text(text: &str) -> Result<Option<NativeAcpCommand>, Error> {
    let input_bytes = text.len();
    let text = text.trim_start_matches([' ', '\t', '\r', '\n']);
    if !text.starts_with('/') {
        return Ok(None);
    }
    if input_bytes > crate::MAX_NATIVE_SLASH_INPUT_BYTES {
        return Err(Error::Limit);
    }
    let route = route_native_slash(text).map_err(|_| Error::Limit)?;
    let invocation = match route {
        NativeSlashRoute::Valid(invocation) => invocation,
        NativeSlashRoute::KnownInvalid { .. } => return Err(Error::Invalid),
        NativeSlashRoute::NotLocal => {
            let token = text.split_ascii_whitespace().next().unwrap_or(text);
            return if token[1..].contains('/') {
                Ok(None)
            } else {
                Err(Error::Unsupported)
            };
        }
    };
    let payload = invocation.payload;
    let action = match invocation.command {
        Command::Help => Action::Help,
        Command::Status => Action::Status,
        Command::Models => Action::Models,
        Command::Permissions if payload.is_empty() => Action::Permissions,
        Command::Allowlist if matches!(payload, "" | "view" | "view effective") => {
            Action::Allowlist
        }
        Command::Model if payload.is_empty() => Action::Status,
        Command::Model if payload == "save" => Action::SaveModel,
        Command::Model if payload == "save-default" => return Err(Error::Unsupported),
        Command::Model => {
            if let Some(effort) = payload.strip_prefix("effort ") {
                Action::Effort(
                    crate::NativeReasoningEffort::parse(effort).map_err(|_| Error::Invalid)?,
                )
            } else {
                machine_god_core::validate_model_id(payload).map_err(|_| Error::Invalid)?;
                Action::Model(payload.to_owned())
            }
        }
        Command::Fast => Action::Fast,
        Command::Compact => Action::Compact,
        Command::Undo => Action::Undo,
        Command::Skills => match payload
            .parse::<crate::NativeSkillsCommand>()
            .map_err(|_| Error::Invalid)?
        {
            crate::NativeSkillsCommand::List => Action::Skills,
            _ => return Err(Error::Unsupported),
        },
        Command::Mcp => match payload
            .parse::<crate::mcp::commands::McpCommand>()
            .map_err(|_| Error::Invalid)?
        {
            crate::mcp::commands::McpCommand::Summary | crate::mcp::commands::McpCommand::List => {
                Action::Mcp
            }
            crate::mcp::commands::McpCommand::Feature(feature) => Action::McpFeature(feature),
            _ => return Err(Error::Unsupported),
        },
        _ => return Err(Error::Unsupported),
    };
    Ok(Some(NativeAcpCommand {
        name: invocation.command,
        action,
    }))
}
