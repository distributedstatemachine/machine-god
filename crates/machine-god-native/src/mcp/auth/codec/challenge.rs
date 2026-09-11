use super::{McpAuthError, Result, redacted};
use std::fmt;

#[derive(Default)]
pub struct McpAuthChallenge {
    pub(in crate::mcp::auth) resource_metadata: Option<Box<str>>,
    pub(in crate::mcp::auth) scope: Option<Box<str>>,
    pub(in crate::mcp::auth) insufficient_scope: bool,
}
redacted!(McpAuthChallenge);
impl McpAuthChallenge {
    /// Parses a bounded combined WWW-Authenticate field list.
    /// # Errors
    /// Rejects invalid quoting, controls and aggregate fields over 16 KiB.
    pub fn parse(value: &[u8]) -> Result<Self> {
        if value.len() > 16 * 1024 {
            return Err(McpAuthError::Limit);
        }
        if value.iter().any(|b| b.is_ascii_control() && *b != b'\t') {
            return Err(McpAuthError::Invalid);
        }
        let value = std::str::from_utf8(value).map_err(|_| McpAuthError::Invalid)?;
        let bytes = value.as_bytes();
        let mut result = Self::default();
        let Some(start) = bytes.windows(6).enumerate().find_map(|(i, part)| {
            (part.eq_ignore_ascii_case(b"bearer")
                && (i == 0 || b", \t".contains(&bytes[i - 1]))
                && bytes.get(i + 6).is_none_or(|b| b" \t".contains(b)))
            .then_some(i + 6)
        }) else {
            return Ok(result);
        };
        let mut cursor = start;
        while cursor < bytes.len() {
            while bytes.get(cursor).is_some_and(|b| b", \t".contains(b)) {
                cursor += 1;
            }
            let start = cursor;
            while bytes
                .get(cursor)
                .is_some_and(|b| b.is_ascii_alphanumeric() || b"_-".contains(b))
            {
                cursor += 1;
            }
            if start == cursor {
                break;
            }
            let key = &value[start..cursor];
            while bytes.get(cursor).is_some_and(|b| b" \t".contains(b)) {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b'=') {
                break;
            }
            cursor += 1;
            while bytes.get(cursor).is_some_and(|b| b" \t".contains(b)) {
                cursor += 1;
            }
            let parsed = parse_value(bytes, &mut cursor)?;
            if key.eq_ignore_ascii_case("resource_metadata") {
                result.resource_metadata = Some(parsed);
            } else if key.eq_ignore_ascii_case("scope") {
                result.scope = Some(parsed);
            } else if key.eq_ignore_ascii_case("error") && parsed.as_ref() == "insufficient_scope" {
                result.insufficient_scope = true;
            }
        }
        Ok(result)
    }
    #[must_use]
    pub fn insufficient_scope(&self) -> bool {
        self.insufficient_scope
    }
}
fn parse_value(bytes: &[u8], cursor: &mut usize) -> Result<Box<str>> {
    let Some(first) = bytes.get(*cursor) else {
        return Err(McpAuthError::Invalid);
    };
    let mut output = Vec::new();
    if *first == b'"' {
        *cursor += 1;
        loop {
            let byte = *bytes.get(*cursor).ok_or(McpAuthError::Invalid)?;
            *cursor += 1;
            if byte == b'"' {
                break;
            }
            if byte == b'\\' {
                output.push(*bytes.get(*cursor).ok_or(McpAuthError::Invalid)?);
                *cursor += 1;
            } else {
                output.push(byte);
            }
        }
    } else {
        let start = *cursor;
        while bytes.get(*cursor).is_some_and(|b| !b", \t".contains(b)) {
            *cursor += 1;
        }
        output.extend_from_slice(&bytes[start..*cursor]);
    }
    String::from_utf8(output)
        .map(String::into_boxed_str)
        .map_err(|_| McpAuthError::Invalid)
}
