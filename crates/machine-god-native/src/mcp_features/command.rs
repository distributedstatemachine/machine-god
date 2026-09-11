//! Human-command projection into the existing bounded feature request contract.

use super::{
    MAX_MCP_FEATURE_ARGUMENTS_BYTES, MAX_MCP_FEATURE_COMPLETION_VALUE_BYTES,
    MAX_MCP_FEATURE_NAME_BYTES, MAX_MCP_FEATURE_PROMPT_ARGUMENTS,
    MAX_MCP_FEATURE_SERIALIZED_ARGUMENT_BYTES, MAX_MCP_FEATURE_SERVER_BYTES,
    MAX_MCP_FEATURE_URI_BYTES, McpFeatureRequest, decode_request, ensure_serialized,
    invalid_arguments, resource_limit,
};
use crate::mcp::commands::McpFeatureCommand;
use machine_god_core::ToolError;
use serde_json::{Value, json};

impl TryFrom<McpFeatureCommand> for McpFeatureRequest {
    type Error = ToolError;

    /// Validates human intent through the same request decoder and canonical
    /// byte budget as the model-facing feature tool. This grants no authority.
    fn try_from(command: McpFeatureCommand) -> Result<Self, Self::Error> {
        validate(&command)?;
        let value = into_json(command);
        let request = decode_request(&value)?;
        ensure_serialized(
            &request.as_json(),
            MAX_MCP_FEATURE_SERIALIZED_ARGUMENT_BYTES,
        )?;
        Ok(request)
    }
}

fn validate(command: &McpFeatureCommand) -> Result<(), ToolError> {
    use McpFeatureCommand as Command;
    let server = match command {
        Command::ResourceList { server }
        | Command::ResourceTemplates { server }
        | Command::ResourceRead { server, .. }
        | Command::ResourceComplete { server, .. }
        | Command::PromptList { server }
        | Command::PromptGet { server, .. }
        | Command::PromptComplete { server, .. } => server,
    };
    if server.is_empty()
        || server.len() > MAX_MCP_FEATURE_SERVER_BYTES
        || !server
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(invalid_arguments());
    }
    match command {
        Command::ResourceList { .. }
        | Command::ResourceTemplates { .. }
        | Command::PromptList { .. } => Ok(()),
        Command::ResourceRead { uri, .. } => text(uri, MAX_MCP_FEATURE_URI_BYTES, false),
        Command::ResourceComplete {
            uri_template,
            argument,
            value,
            ..
        } => {
            token(uri_template, MAX_MCP_FEATURE_URI_BYTES)?;
            completion(argument, value)
        }
        Command::PromptComplete {
            prompt,
            argument,
            value,
            ..
        } => {
            token(prompt, MAX_MCP_FEATURE_NAME_BYTES)?;
            completion(argument, value)
        }
        Command::PromptGet {
            prompt, arguments, ..
        } => {
            token(prompt, MAX_MCP_FEATURE_NAME_BYTES)?;
            if arguments.len() > MAX_MCP_FEATURE_PROMPT_ARGUMENTS {
                return Err(resource_limit());
            }
            // Keys/values are JSON data: escaped controls remain literal data.
            // Check before constructing another owned map or cloning any value.
            for (key, value) in arguments {
                if key.is_empty() || key.len() > MAX_MCP_FEATURE_NAME_BYTES {
                    return Err(invalid_arguments());
                }
                if value.len() > MAX_MCP_FEATURE_ARGUMENTS_BYTES {
                    return Err(resource_limit());
                }
            }
            ensure_serialized(arguments, MAX_MCP_FEATURE_ARGUMENTS_BYTES)
        }
    }
}

fn text(value: &str, limit: usize, allow_empty: bool) -> Result<(), ToolError> {
    if value.len() > limit {
        return Err(resource_limit());
    }
    if (!allow_empty && value.is_empty()) || value.chars().any(|c| c.is_control() && c != '\t') {
        return Err(invalid_arguments());
    }
    Ok(())
}

fn token(value: &str, limit: usize) -> Result<(), ToolError> {
    text(value, limit, false)?;
    if value.contains([' ', '\t']) {
        return Err(invalid_arguments());
    }
    Ok(())
}

fn completion(argument: &str, value: &str) -> Result<(), ToolError> {
    token(argument, MAX_MCP_FEATURE_NAME_BYTES)?;
    text(value, MAX_MCP_FEATURE_COMPLETION_VALUE_BYTES, true)
}

fn into_json(command: McpFeatureCommand) -> Value {
    use McpFeatureCommand as Command;
    match command {
        Command::ResourceList { server } => json!({"action": "resource_list", "server": server}),
        Command::ResourceTemplates { server } => {
            json!({"action": "resource_templates", "server": server})
        }
        Command::PromptList { server } => json!({"action": "prompt_list", "server": server}),
        Command::ResourceRead { server, uri } => {
            json!({"action": "resource_read", "server": server, "uri": uri})
        }
        Command::ResourceComplete {
            server,
            uri_template,
            argument,
            value,
        } => {
            json!({"action": "resource_complete", "server": server,
                "uri_template": uri_template, "argument": argument, "value": value})
        }
        Command::PromptGet {
            server,
            prompt,
            arguments,
        } => {
            json!({"action": "prompt_get", "server": server,
                "prompt": prompt, "arguments": arguments})
        }
        Command::PromptComplete {
            server,
            prompt,
            argument,
            value,
        } => {
            json!({"action": "prompt_complete", "server": server,
                "prompt": prompt, "argument": argument, "value": value})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::commands::McpCommand;
    use std::collections::BTreeMap;

    fn command(input: &str) -> McpFeatureCommand {
        match input.parse::<McpCommand>().unwrap() {
            McpCommand::Feature(command) => command,
            _ => panic!("expected a feature command"),
        }
    }

    #[test]
    fn all_seven_commands_reuse_canonical_feature_requests() {
        let cases = [
            (
                "resource list srv",
                json!({"action":"resource_list", "server":"srv"}),
            ),
            (
                "resource templates srv",
                json!({"action":"resource_templates", "server":"srv"}),
            ),
            (
                "resource read srv custom://a b",
                json!({"action":"resource_read", "server":"srv", "uri":"custom://a b"}),
            ),
            (
                "resource complete srv custom://{id} id a b",
                json!({"action":"resource_complete", "server":"srv", "uri_template":"custom://{id}", "argument":"id", "value":"a b"}),
            ),
            (
                "prompt list srv",
                json!({"action":"prompt_list", "server":"srv"}),
            ),
            (
                "prompt get srv question {\"topic\":\"line\\nnext\"}",
                json!({"action":"prompt_get", "server":"srv", "prompt":"question", "arguments":{"topic":"line\nnext"}}),
            ),
            (
                "prompt complete srv question topic",
                json!({"action":"prompt_complete", "server":"srv", "prompt":"question", "argument":"topic"}),
            ),
        ];
        for (input, expected) in cases {
            let request = McpFeatureRequest::try_from(command(input)).unwrap();
            assert_eq!(request.as_json(), expected);
            assert!(request.context().is_empty());
        }
    }

    #[test]
    fn direct_enum_construction_cannot_bypass_command_validation() {
        for server in ["", "foreign.server", "bad server", "bad\nserver"] {
            assert!(
                McpFeatureRequest::try_from(McpFeatureCommand::PromptList {
                    server: server.into(),
                })
                .is_err()
            );
        }
        for prompt in ["", "two words", "bad\nvalue"] {
            assert!(
                McpFeatureRequest::try_from(McpFeatureCommand::PromptGet {
                    server: "srv".into(),
                    prompt: prompt.into(),
                    arguments: BTreeMap::new(),
                })
                .is_err()
            );
        }
        assert!(
            McpFeatureRequest::try_from(McpFeatureCommand::ResourceRead {
                server: "srv".into(),
                uri: "x".repeat(MAX_MCP_FEATURE_URI_BYTES + 1),
            })
            .is_err()
        );
    }

    #[test]
    fn aggregate_canonical_and_escaped_json_budgets_are_checked() {
        // Individual URI bound isn't the whole canonical request bound.
        assert!(
            McpFeatureRequest::try_from(McpFeatureCommand::ResourceRead {
                server: "srv".into(),
                uri: "x".repeat(MAX_MCP_FEATURE_URI_BYTES),
            })
            .is_err()
        );
        for arguments in [
            BTreeMap::from([("x".into(), "\0".repeat(MAX_MCP_FEATURE_ARGUMENTS_BYTES / 6))]),
            BTreeMap::from([("x".repeat(MAX_MCP_FEATURE_NAME_BYTES + 1), "v".into())]),
            (0..=MAX_MCP_FEATURE_PROMPT_ARGUMENTS)
                .map(|i| (i.to_string(), String::new()))
                .collect(),
        ] {
            assert!(
                McpFeatureRequest::try_from(McpFeatureCommand::PromptGet {
                    server: "srv".into(),
                    prompt: "q".into(),
                    arguments,
                })
                .is_err()
            );
        }
    }

    #[test]
    fn omitted_empty_defaults_do_not_reduce_the_canonical_byte_budget() {
        let fixed = serde_json::to_vec(&json!({
            "action":"resource_complete", "server":"srv", "uri_template":"",
            "argument":"id"
        }))
        .unwrap()
        .len();
        let make = |length| McpFeatureCommand::ResourceComplete {
            server: "srv".into(),
            uri_template: "x".repeat(length),
            argument: "id".into(),
            value: String::new(),
        };
        let exact = MAX_MCP_FEATURE_SERIALIZED_ARGUMENT_BYTES - fixed;
        assert!(McpFeatureRequest::try_from(make(exact)).is_ok());
        assert!(McpFeatureRequest::try_from(make(exact + 1)).is_err());
    }
}
