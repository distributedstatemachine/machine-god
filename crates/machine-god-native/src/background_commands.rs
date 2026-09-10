//! Inert grammar for the interactive `/background` payload.
//!
//! This surface selects native terminal sessions, not the legacy numeric
//! records used by the read-only top-level `background` command. Parsing grants
//! no authority and performs no I/O; the native host resolves each selection.

use std::{fmt, str::FromStr};

use machine_god_core::TerminalSessionId;

#[cfg(test)]
pub(crate) mod url;

/// Maximum UTF-8 payload size, checked before scanning or allocating.
pub const MAX_NATIVE_BACKGROUND_COMMAND_BYTES: usize = 256;

/// An inert selection whose identity is redacted from debug output.
#[derive(Clone, Eq, PartialEq)]
pub enum NativeBackgroundTarget {
    /// Resolve the latest eligible session at execution time.
    Last,
    /// Resolve exactly this terminal session; this value grants no access.
    Session(TerminalSessionId),
}

impl fmt::Debug for NativeBackgroundTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeBackgroundTarget { .. }")
    }
}

/// An inert interactive command, parsed without the `/background` prefix.
///
/// Empty input lists sessions. Exact lowercase `stop`, `open`, and `logs`
/// accept one optional target; omission and `last` both select `Last`, matching
/// the pinned upstream default. Only ASCII space/tab separators are accepted.
/// Explicit targets use the native generated spelling `terminal-` followed by
/// exactly 32 lowercase hexadecimal digits. Legacy numeric IDs, arbitrary
/// paths, other terminal-ID spellings, flags, and aliases are not accepted.
#[derive(Clone, Eq, PartialEq)]
pub enum NativeBackgroundCommand {
    List,
    Stop(NativeBackgroundTarget),
    Open(NativeBackgroundTarget),
    Logs(NativeBackgroundTarget),
}

impl fmt::Debug for NativeBackgroundCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::List => "NativeBackgroundCommand::List",
            Self::Stop(_) => "NativeBackgroundCommand::Stop(..)",
            Self::Open(_) => "NativeBackgroundCommand::Open(..)",
            Self::Logs(_) => "NativeBackgroundCommand::Logs(..)",
        })
    }
}

/// A fixed, content-free rejection of malformed or oversized input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeBackgroundCommandError;

impl fmt::Display for NativeBackgroundCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid native background command")
    }
}

impl std::error::Error for NativeBackgroundCommandError {}

impl FromStr for NativeBackgroundCommand {
    type Err = NativeBackgroundCommandError;

    fn from_str(payload: &str) -> Result<Self, Self::Err> {
        if payload.len() > MAX_NATIVE_BACKGROUND_COMMAND_BYTES {
            return Err(NativeBackgroundCommandError);
        }
        if !payload
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || matches!(byte, b' ' | b'\t'))
        {
            return Err(NativeBackgroundCommandError);
        }
        let mut tokens = payload.split_ascii_whitespace();
        let Some(verb) = tokens.next() else {
            return Ok(Self::List);
        };
        if !matches!(verb, "stop" | "open" | "logs") {
            return Err(NativeBackgroundCommandError);
        }
        let target = tokens.next();
        if tokens.next().is_some() {
            return Err(NativeBackgroundCommandError);
        }
        // Complete syntax validation precedes the only allocation: the exact
        // generated session identity retained by a successful explicit target.
        let target = match target {
            None | Some("last") => NativeBackgroundTarget::Last,
            Some(value) => {
                let suffix = value
                    .strip_prefix("terminal-")
                    .ok_or(NativeBackgroundCommandError)?;
                if suffix.len() != 32
                    || !suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                {
                    return Err(NativeBackgroundCommandError);
                }
                NativeBackgroundTarget::Session(
                    TerminalSessionId::new(value).map_err(|_| NativeBackgroundCommandError)?,
                )
            }
        };
        Ok(match verb {
            "stop" => Self::Stop(target),
            "open" => Self::Open(target),
            "logs" => Self::Logs(target),
            _ => return Err(NativeBackgroundCommandError),
        })
    }
}

#[cfg(test)]
mod parser_tests;
