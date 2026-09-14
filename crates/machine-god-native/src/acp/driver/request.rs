//! Pure modern request projection. No session, profile or filesystem authority.

use crate::NativeSessionCatalogCursor;
use crate::acp::protocol::{
    ACP_MAX_FRAME_BYTES, AcpRpcError, validate_protocol_version, validate_value,
};
use crate::acp::session::{NativeAcpPrompt, NativeAcpSessionSelection, decode_prompt_input};
use crate::mcp::{config::MAX_CONFIG_BYTES, ephemeral::NativeMcpEphemeralConfiguration};
use machine_god_core::{SessionId, validate_model_id};
use serde_json::{Map, Value};
use std::{io, io::Write, path::Component, path::PathBuf};

const MAX_CWD_BYTES: usize = 4096;

pub(super) enum Request {
    Initialize,
    Select {
        selection: NativeAcpSessionSelection,
        cwd: PathBuf,
        mcp: NativeMcpEphemeralConfiguration,
    },
    Close {
        session: SessionId,
    },
    List {
        cwd: Option<PathBuf>,
        cursor: Option<NativeSessionCatalogCursor>,
    },
    Prompt {
        session: SessionId,
        prompt: NativeAcpPrompt,
    },
    Cancel {
        session: SessionId,
    },
    SetMode {
        session: SessionId,
        mode: String,
    },
    SetConfig {
        session: SessionId,
        config: String,
        value: String,
    },
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Request(<redacted>)")
    }
}

pub(super) fn decode(method: &str, params: Option<Value>) -> Result<Request, AcpRpcError> {
    // Even rejected programmatically constructed trees must be reclaimed without
    // recursive Value destruction (the wire decoder has already bounded depth).
    let params = Params(params);
    if !matches!(
        method,
        "initialize"
            | "session/new"
            | "session/load"
            | "session/resume"
            | "session/close"
            | "session/list"
            | "session/prompt"
            | "session/cancel"
            | "session/set_mode"
            | "session/set_config_option"
    ) {
        return Err(error(-32601, "Method not found"));
    }
    let empty = Value::Object(Map::new());
    let value = match params.0.as_ref() {
        Some(value) => value,
        None if method == "session/list" => &empty,
        None => return Err(invalid()),
    };
    validate_value(value, 1).map_err(|_| invalid())?;
    let mut count = BoundedWriter::counting(ACP_MAX_FRAME_BYTES);
    serde_json::to_writer(&mut count, value).map_err(|_| invalid())?;
    let object = value.as_object().ok_or_else(invalid)?;
    match method {
        "initialize" => {
            validate_protocol_version(value).map_err(|_| invalid())?;
            Ok(Request::Initialize)
        }
        "session/new" | "session/load" | "session/resume" => {
            let selection = match method {
                "session/new" => NativeAcpSessionSelection::New,
                "session/load" => NativeAcpSessionSelection::Load(session(object)?),
                _ => NativeAcpSessionSelection::Resume(session(object)?),
            };
            Ok(Request::Select {
                selection,
                cwd: cwd(required(object, "cwd")?)?,
                mcp: mcp(object.get("mcpServers"))?,
            })
        }
        "session/close" => Ok(Request::Close {
            session: session(object)?,
        }),
        "session/list" => Ok(Request::List {
            cwd: object.get("cwd").map(cwd).transpose()?,
            cursor: object
                .get("cursor")
                .map(|value| {
                    NativeSessionCatalogCursor::parse(value.as_str().ok_or_else(invalid)?)
                        .map_err(|_| invalid())
                })
                .transpose()?,
        }),
        "session/prompt" => Ok(Request::Prompt {
            session: session(object)?,
            prompt: decode_prompt_input(value).map_err(|_| invalid())?,
        }),
        "session/cancel" => Ok(Request::Cancel {
            session: session(object)?,
        }),
        "session/set_mode" => {
            let mode = string(object, "modeId")?;
            validate_mode(mode)?;
            Ok(Request::SetMode {
                session: session(object)?,
                mode: mode.to_owned(),
            })
        }
        "session/set_config_option" => {
            let config = string(object, "configId")?;
            let value = string(object, "value")?;
            match config {
                "mode" => validate_mode(value)?,
                "model" => validate_model_id(value).map_err(|_| invalid())?,
                _ => return Err(invalid()),
            }
            Ok(Request::SetConfig {
                session: session(object)?,
                config: config.to_owned(),
                value: value.to_owned(),
            })
        }
        _ => unreachable!("method admission above is exhaustive"),
    }
}

fn required<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Value, AcpRpcError> {
    object.get(key).ok_or_else(invalid)
}
fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, AcpRpcError> {
    required(object, key)?.as_str().ok_or_else(invalid)
}
fn session(object: &Map<String, Value>) -> Result<SessionId, AcpRpcError> {
    SessionId::new(string(object, "sessionId")?).map_err(|_| invalid())
}
fn validate_mode(value: &str) -> Result<(), AcpRpcError> {
    if matches!(value, "ask" | "auto" | "yolo") {
        Ok(())
    } else {
        Err(invalid())
    }
}
fn cwd(value: &Value) -> Result<PathBuf, AcpRpcError> {
    let raw = value.as_str().ok_or_else(invalid)?;
    if raw.is_empty() || raw.len() > MAX_CWD_BYTES || raw.chars().any(char::is_control) {
        return Err(invalid());
    }
    let path = std::path::Path::new(raw);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(invalid());
    }
    Ok(path.components().collect())
}
fn mcp(value: Option<&Value>) -> Result<NativeMcpEphemeralConfiguration, AcpRpcError> {
    let Some(value) = value else {
        return NativeMcpEphemeralConfiguration::decode(None).map_err(|_| invalid());
    };
    let mut writer = BoundedWriter::retaining(MAX_CONFIG_BYTES);
    serde_json::to_writer(&mut writer, value).map_err(|_| invalid())?;
    NativeMcpEphemeralConfiguration::decode(writer.bytes.as_deref()).map_err(|_| invalid())
}
fn invalid() -> AcpRpcError {
    error(-32602, "Invalid params")
}
fn error(code: i64, message: &str) -> AcpRpcError {
    AcpRpcError {
        code,
        message: message.to_owned(),
        data: None,
    }
}

/// Counting never retains input; MCP retains at most its independent raw limit.
struct BoundedWriter {
    remaining: usize,
    bytes: Option<Vec<u8>>,
}
impl BoundedWriter {
    const fn counting(limit: usize) -> Self {
        Self {
            remaining: limit,
            bytes: None,
        }
    }
    const fn retaining(limit: usize) -> Self {
        Self {
            remaining: limit,
            bytes: Some(Vec::new()),
        }
    }
}
impl Write for BoundedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.remaining {
            return Err(io::Error::other("request byte limit"));
        }
        if let Some(bytes) = &mut self.bytes {
            if bytes.capacity() - bytes.len() < buf.len() {
                let capacity = bytes
                    .capacity()
                    .saturating_mul(2)
                    .max(bytes.len() + buf.len())
                    .min(bytes.len() + self.remaining);
                bytes.reserve_exact(capacity - bytes.len());
            }
            bytes.extend_from_slice(buf);
        }
        self.remaining -= buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Params(Option<Value>);

pub(super) fn discard(params: Option<Value>) {
    drop(Params(params));
}
impl Drop for Params {
    fn drop(&mut self) {
        enum Children {
            Array(std::vec::IntoIter<Value>),
            Object(serde_json::map::IntoValues),
        }
        let mut frames = Vec::new();
        let mut current = self.0.take();
        loop {
            if let Some(value) = current.take() {
                match value {
                    Value::Array(items) => frames.push(Children::Array(items.into_iter())),
                    Value::Object(items) => frames.push(Children::Object(items.into_values())),
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
                }
            }
            loop {
                let Some(frame) = frames.last_mut() else {
                    return;
                };
                let next = match frame {
                    Children::Array(items) => items.next(),
                    Children::Object(items) => items.next(),
                };
                if next.is_some() {
                    current = next;
                    break;
                }
                frames.pop();
            }
        }
    }
}

#[cfg(test)]
#[path = "request/tests.rs"]
mod tests;
