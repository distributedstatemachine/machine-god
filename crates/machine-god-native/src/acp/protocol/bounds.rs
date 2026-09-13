use std::io::{self, Write};

use serde_json::Value;

use super::{
    ACP_MAX_FRAME_BYTES, ACP_MAX_JSON_DEPTH, ACP_MAX_JSON_NODES, ACP_MAX_RETAINED_BYTES,
    AcpMessage, AcpProtocolError,
};

// Charge both tree payload bytes and parsing/serialization scratch. The token
// charge covers Value, String/Vec, and map-node overhead, including spare slots.
const TOKEN_CHARGE: usize = 256;

pub(super) fn preflight(bytes: &[u8]) -> Result<(), AcpProtocolError> {
    if bytes.len() > ACP_MAX_FRAME_BYTES {
        return Err(AcpProtocolError::FrameTooLarge);
    }
    let (mut quoted, mut escaped, mut scalar) = (false, false, false);
    let (mut depth, mut tokens) = (0usize, 0usize);
    for &byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        let starts = match byte {
            b'"' => {
                quoted = true;
                scalar = false;
                true
            }
            b'[' | b'{' => {
                depth += 1;
                if depth > ACP_MAX_JSON_DEPTH {
                    return Err(AcpProtocolError::JsonBudgetExceeded);
                }
                scalar = false;
                true
            }
            b']' | b'}' => {
                depth = depth.saturating_sub(1);
                scalar = false;
                false
            }
            b',' | b':' | b' ' | b'\t' | b'\r' | b'\n' => {
                scalar = false;
                false
            }
            _ => {
                let starts = !scalar;
                scalar = true;
                starts
            }
        };
        if starts {
            tokens += 1;
            if tokens > ACP_MAX_JSON_NODES
                || tokens * TOKEN_CHARGE + bytes.len() * 2 > ACP_MAX_RETAINED_BYTES
            {
                return Err(AcpProtocolError::JsonBudgetExceeded);
            }
        }
    }
    Ok(())
}

// Walk borrowed values with depth-bounded iterator storage, not a queue of all
// siblings. This runs before recursive serde traversal or output allocation.
enum Children<'a> {
    Array(std::slice::Iter<'a, Value>),
    Object(serde_json::map::Iter<'a>),
}

pub(super) fn encode(message: &AcpMessage) -> Result<Vec<u8>, AcpProtocolError> {
    let (root, initial_depth) = match message {
        AcpMessage::Request { params, .. } | AcpMessage::Notification { params, .. } => {
            (params.as_ref(), 1)
        }
        AcpMessage::Response {
            outcome: Ok(result),
            ..
        } => (Some(result), 1),
        AcpMessage::Response {
            outcome: Err(error),
            ..
        } => {
            if error.message.len() > ACP_MAX_FRAME_BYTES {
                return Err(AcpProtocolError::FrameTooLarge);
            }
            (error.data.as_ref(), 2)
        }
    };
    if let Some(root) = root {
        validate_tree(root, initial_depth)?;
    }
    let mut writer = FrameWriter { bytes: Vec::new() };
    serde_json::to_writer(&mut writer, message).map_err(|_| AcpProtocolError::FrameTooLarge)?;
    // Account for envelope strings, keys and escaping with the exact same
    // lexical budget as inbound frames. Output never exceeds the frame cap.
    preflight(&writer.bytes)?;
    if writer.bytes.len() == writer.bytes.capacity() {
        writer.bytes.reserve_exact(1);
    }
    writer.bytes.push(b'\n');
    Ok(writer.bytes)
}

fn validate_tree(root: &Value, initial_depth: usize) -> Result<(), AcpProtocolError> {
    let mut frames = Vec::<Children<'_>>::new();
    let mut next = Some(root);
    let mut nodes = 0usize;
    let mut raw_bytes = 0usize;
    loop {
        if let Some(value) = next.take() {
            nodes += 1;
            if nodes > ACP_MAX_JSON_NODES {
                return Err(AcpProtocolError::JsonBudgetExceeded);
            }
            let children = match value {
                Value::Array(values) => Some(Children::Array(values.iter())),
                Value::Object(values) => Some(Children::Object(values.iter())),
                Value::String(text) => {
                    charge_bytes(&mut raw_bytes, text.len())?;
                    None
                }
                Value::Number(number) => {
                    // Number::from_string_unchecked is public. Do not let a
                    // caller inject punctuation or non-number raw JSON through
                    // serde's private arbitrary-precision representation.
                    let token = number.as_str();
                    charge_bytes(&mut raw_bytes, token.len())?;
                    if !matches!(token.as_bytes().first(), Some(b'-' | b'0'..=b'9'))
                        || !serde_json::from_str::<&serde_json::value::RawValue>(token)
                            .is_ok_and(|raw| raw.get() == token)
                    {
                        return Err(AcpProtocolError::InvalidRequest);
                    }
                    None
                }
                _ => None,
            };
            if let Some(children) = children {
                if initial_depth + frames.len() + 1 > ACP_MAX_JSON_DEPTH {
                    return Err(AcpProtocolError::JsonBudgetExceeded);
                }
                frames.push(children);
            }
        }
        loop {
            let Some(frame) = frames.last_mut() else {
                return Ok(());
            };
            next = match frame {
                Children::Array(children) => children.next(),
                Children::Object(children) => match children.next() {
                    Some((key, value)) => {
                        charge_bytes(&mut raw_bytes, key.len())?;
                        nodes += 1;
                        Some(value)
                    }
                    None => None,
                },
            };
            if next.is_some() {
                break;
            }
            frames.pop();
        }
    }
}

fn charge_bytes(total: &mut usize, bytes: usize) -> Result<(), AcpProtocolError> {
    if bytes > ACP_MAX_FRAME_BYTES - *total {
        return Err(AcpProtocolError::FrameTooLarge);
    }
    *total += bytes;
    Ok(())
}

struct FrameWriter {
    bytes: Vec<u8>,
}

impl Write for FrameWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let remaining = ACP_MAX_FRAME_BYTES - self.bytes.len();
        if bytes.len() > remaining {
            return Err(io::Error::other("ACP output frame limit"));
        }
        // Vec's geometric growth is bounded to the frame ceiling as well.
        if self.bytes.capacity() - self.bytes.len() < bytes.len() {
            let target = self
                .bytes
                .len()
                .saturating_add(bytes.len())
                .max(self.bytes.capacity().saturating_mul(2))
                .min(ACP_MAX_FRAME_BYTES);
            self.bytes.reserve_exact(target - self.bytes.len());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
