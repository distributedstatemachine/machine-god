//! Effect-free parsing of human-invoked MCP management commands.
//!
//! Inputs omit the `/mcp` prefix. The grammar follows the pinned command
//! provider: ASCII space/tab tokenization, no shell quoting or interpolation,
//! and an exact remaining string for resource reads and completion values.
//! Parsed operations carry intent, never runtime or publication authority.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::{
    MAX_MCP_FEATURE_ARGUMENTS_BYTES, MAX_MCP_FEATURE_COMPLETION_VALUE_BYTES,
    MAX_MCP_FEATURE_NAME_BYTES as MAX_NAME_BYTES,
    MAX_MCP_FEATURE_PROMPT_ARGUMENTS as MAX_PROMPT_ARGUMENTS,
    MAX_MCP_FEATURE_SERVER_BYTES as MAX_SERVER_BYTES, MAX_MCP_FEATURE_URI_BYTES as MAX_URI_BYTES,
};

/// Inclusive bound checked before scanning or copying the input.
pub const MAX_MCP_COMMAND_BYTES: usize = 128 * 1024;
/// Inclusive bound on prompt-get argument JSON before decoding.
pub const MAX_MCP_COMMAND_ARGUMENT_BYTES: usize = MAX_MCP_FEATURE_ARGUMENTS_BYTES;
const MAX_TOKEN_BYTES: usize = 4096;
const MAX_ARGUMENTS: usize = 256;

/// Human command intent. Adapters must separately validate live effect authority.
#[derive(Clone, Eq, PartialEq)]
pub enum McpCommand {
    Summary,
    List,
    Path,
    Reload,
    Add {
        server: String,
        command: String,
        arguments: Vec<String>,
    },
    Remove {
        server: String,
    },
    Authenticate {
        server: String,
        open_browser: bool,
    },
    Logout {
        server: String,
    },
    Feature(McpFeatureCommand),
}

/// Read-only resource/prompt intent; returned content remains untrusted data.
#[derive(Clone, Eq, PartialEq)]
pub enum McpFeatureCommand {
    ResourceList {
        server: String,
    },
    ResourceTemplates {
        server: String,
    },
    ResourceRead {
        server: String,
        uri: String,
    },
    ResourceComplete {
        server: String,
        uri_template: String,
        argument: String,
        value: String,
    },
    PromptList {
        server: String,
    },
    PromptGet {
        server: String,
        prompt: String,
        arguments: BTreeMap<String, String>,
    },
    PromptComplete {
        server: String,
        prompt: String,
        argument: String,
        value: String,
    },
}

impl fmt::Debug for McpCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Summary => "McpCommand::Summary",
            Self::List => "McpCommand::List",
            Self::Path => "McpCommand::Path",
            Self::Reload => "McpCommand::Reload",
            Self::Add { .. } => "McpCommand::Add(..)",
            Self::Remove { .. } => "McpCommand::Remove(..)",
            Self::Authenticate { .. } => "McpCommand::Authenticate(..)",
            Self::Logout { .. } => "McpCommand::Logout(..)",
            Self::Feature(_) => "McpCommand::Feature(..)",
        })
    }
}

impl fmt::Debug for McpFeatureCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpFeatureCommand { .. }")
    }
}

/// Fixed, redacted reason for rejecting command input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpCommandParseErrorKind {
    InvalidSyntax,
    ResourceLimit,
}

/// A parse failure never includes input, paths, configuration or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpCommandParseError(McpCommandParseErrorKind);

impl McpCommandParseError {
    #[must_use]
    pub const fn kind(self) -> McpCommandParseErrorKind {
        self.0
    }
}

impl fmt::Display for McpCommandParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            McpCommandParseErrorKind::InvalidSyntax => "invalid MCP command",
            McpCommandParseErrorKind::ResourceLimit => "MCP command resource limit exceeded",
        })
    }
}

impl std::error::Error for McpCommandParseError {}

type Result<T> = std::result::Result<T, McpCommandParseError>;

impl FromStr for McpCommand {
    type Err = McpCommandParseError;

    fn from_str(input: &str) -> Result<Self> {
        bounded(input, MAX_MCP_COMMAND_BYTES)?;
        if input.chars().any(|c| c.is_control() && c != '\t') {
            return Err(invalid());
        }
        let mut rest = input.trim_matches([' ', '\t']);
        let Some(verb) = take_token(&mut rest) else {
            return Ok(Self::Summary);
        };
        match verb {
            "list" | "path" | "reload" => {
                end(rest)?;
                Ok(match verb {
                    "list" => Self::List,
                    "path" => Self::Path,
                    _ => Self::Reload,
                })
            }
            "add" => {
                let server = server(&mut rest)?;
                let command = token(&mut rest, MAX_TOKEN_BYTES)?;
                let mut arguments = Vec::new();
                while let Some(argument) = take_token(&mut rest) {
                    if arguments.len() == MAX_ARGUMENTS {
                        return Err(limit());
                    }
                    bounded(argument, MAX_TOKEN_BYTES)?;
                    arguments.push(argument.to_owned());
                }
                Ok(Self::Add {
                    server,
                    command,
                    arguments,
                })
            }
            "remove" | "logout" => {
                let server = server(&mut rest)?;
                end(rest)?;
                Ok(if verb == "remove" {
                    Self::Remove { server }
                } else {
                    Self::Logout { server }
                })
            }
            "auth" => {
                let server = server(&mut rest)?;
                let open_browser = match take_token(&mut rest) {
                    None => false,
                    Some("--open") => true,
                    Some(_) => return Err(invalid()),
                };
                end(rest)?;
                Ok(Self::Authenticate {
                    server,
                    open_browser,
                })
            }
            "resource" | "prompt" => parse_feature(verb, rest).map(Self::Feature),
            _ => Err(invalid()),
        }
    }
}

fn parse_feature(family: &str, mut rest: &str) -> Result<McpFeatureCommand> {
    let action = take_token(&mut rest).ok_or_else(invalid)?;
    let server = server(&mut rest)?;
    match (family, action) {
        ("resource", "list" | "templates") | ("prompt", "list") => {
            end(rest)?;
            Ok(match (family, action) {
                ("resource", "list") => McpFeatureCommand::ResourceList { server },
                ("resource", _) => McpFeatureCommand::ResourceTemplates { server },
                _ => McpFeatureCommand::PromptList { server },
            })
        }
        ("resource", "read") => {
            if rest.is_empty() {
                return Err(invalid());
            }
            bounded(rest, MAX_URI_BYTES)?;
            Ok(McpFeatureCommand::ResourceRead {
                server,
                uri: rest.to_owned(),
            })
        }
        ("resource" | "prompt", "complete") => {
            let identity = token(
                &mut rest,
                if family == "resource" {
                    MAX_URI_BYTES
                } else {
                    MAX_NAME_BYTES
                },
            )?;
            let argument = token(&mut rest, MAX_NAME_BYTES)?;
            bounded(rest, MAX_MCP_FEATURE_COMPLETION_VALUE_BYTES)?;
            let value = rest.to_owned();
            Ok(if family == "resource" {
                McpFeatureCommand::ResourceComplete {
                    server,
                    uri_template: identity,
                    argument,
                    value,
                }
            } else {
                McpFeatureCommand::PromptComplete {
                    server,
                    prompt: identity,
                    argument,
                    value,
                }
            })
        }
        ("prompt", "get") => {
            let prompt = token(&mut rest, MAX_NAME_BYTES)?;
            bounded(rest, MAX_MCP_COMMAND_ARGUMENT_BYTES)?;
            let arguments = if rest.is_empty() {
                BTreeMap::new()
            } else {
                serde_json::from_str::<PromptArguments>(rest)
                    .map_err(|_| invalid())?
                    .0
            };
            Ok(McpFeatureCommand::PromptGet {
                server,
                prompt,
                arguments,
            })
        }
        _ => Err(invalid()),
    }
}

fn take_token<'a>(rest: &mut &'a str) -> Option<&'a str> {
    let input = rest.trim_matches([' ', '\t']);
    if input.is_empty() {
        *rest = "";
        return None;
    }
    let split = input.find([' ', '\t']).unwrap_or(input.len());
    *rest = input[split..].trim_matches([' ', '\t']);
    Some(&input[..split])
}

fn token(rest: &mut &str, maximum: usize) -> Result<String> {
    let value = take_token(rest).ok_or_else(invalid)?;
    bounded(value, maximum)?;
    Ok(value.to_owned())
}

fn server(rest: &mut &str) -> Result<String> {
    let name = take_token(rest).ok_or_else(invalid)?;
    bounded(name, MAX_SERVER_BYTES)?;
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(invalid());
    }
    Ok(name.to_owned())
}

fn end(rest: &str) -> Result<()> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn bounded(value: &str, maximum: usize) -> Result<()> {
    if value.len() <= maximum {
        Ok(())
    } else {
        Err(limit())
    }
}

const fn invalid() -> McpCommandParseError {
    McpCommandParseError(McpCommandParseErrorKind::InvalidSyntax)
}

const fn limit() -> McpCommandParseError {
    McpCommandParseError(McpCommandParseErrorKind::ResourceLimit)
}

struct PromptArguments(BTreeMap<String, String>);

impl<'de> Deserialize<'de> for PromptArguments {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ArgumentsVisitor;
        impl<'de> Visitor<'de> for ArgumentsVisitor {
            type Value = PromptArguments;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bounded object of unique string arguments")
            }

            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut arguments = BTreeMap::new();
                while let Some(name) = map.next_key::<String>()? {
                    if arguments.len() == MAX_PROMPT_ARGUMENTS
                        || name.is_empty()
                        || name.len() > MAX_NAME_BYTES
                        || arguments.contains_key(&name)
                    {
                        return Err(serde::de::Error::custom("invalid prompt arguments"));
                    }
                    let value = map.next_value::<String>()?;
                    arguments.insert(name, value);
                }
                Ok(PromptArguments(arguments))
            }
        }
        deserializer.deserialize_map(ArgumentsVisitor)
    }
}

#[cfg(test)]
mod tests;
