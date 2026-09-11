use super::{McpHttpBody, McpHttpError, McpHttpLimits, Result, body::Framing, io::Buffered};
use std::fmt;

type Header = (Box<str>, Box<[u8]>);
/// Bounded raw header observations. Duplicate names remain distinct; callers must
/// reject ambiguous protocol/auth metadata before committing it to a generation.
pub struct McpHttpHeaders(pub(super) Box<[Header]>);
impl fmt::Debug for McpHttpHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpHeaders { <redacted> }")
    }
}
impl McpHttpHeaders {
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &[u8])> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_ref(), value.as_ref()))
    }
}

/// Status/head are uncommitted observations; body owns the remaining socket.
pub struct McpHttpResponse {
    pub status: u16,
    pub headers: McpHttpHeaders,
    pub body: McpHttpBody,
}
impl fmt::Debug for McpHttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpHttpResponse { <redacted> }")
    }
}

pub(super) async fn receive(mut io: Buffered, limits: McpHttpLimits) -> Result<McpHttpResponse> {
    let mut remaining = limits.head_bytes;
    for _ in 0..=8 {
        let bytes = head(&mut io, remaining).await?;
        remaining -= bytes.len();
        let mut headers = vec![httparse::EMPTY_HEADER; limits.header_count];
        let mut response = httparse::Response::new(&mut headers);
        if response.parse(&bytes).map_err(|_| McpHttpError::Protocol)?
            != httparse::Status::Complete(bytes.len())
        {
            return Err(McpHttpError::Protocol);
        }
        let status = response.code.ok_or(McpHttpError::Protocol)?;
        if !(100..=599).contains(&status) {
            return Err(McpHttpError::Protocol);
        }
        let framing = framing(
            status,
            response.version,
            response.headers,
            limits.body_bytes,
        )?;
        if status < 200 {
            if status == 101 {
                return Err(McpHttpError::Protocol);
            }
            continue;
        }
        let headers = owned(response.headers);
        return Ok(McpHttpResponse {
            status,
            headers,
            body: McpHttpBody::new(io, framing, limits),
        });
    }
    Err(McpHttpError::Limit)
}

pub(super) async fn head(io: &mut Buffered, maximum: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        let line = io.line(maximum.saturating_sub(bytes.len())).await?;
        bytes.extend_from_slice(&line);
        if line == b"\r\n" {
            return Ok(bytes);
        }
        tokio::task::yield_now().await;
    }
}

pub(super) fn owned(headers: &[httparse::Header<'_>]) -> McpHttpHeaders {
    McpHttpHeaders(
        headers
            .iter()
            .map(|header| {
                (
                    header.name.to_ascii_lowercase().into_boxed_str(),
                    header.value.into(),
                )
            })
            .collect(),
    )
}

fn framing(
    status: u16,
    version: Option<u8>,
    headers: &[httparse::Header<'_>],
    maximum: u64,
) -> Result<Framing> {
    let mut length = None;
    let mut transfer = false;
    for header in headers {
        if header.name.eq_ignore_ascii_case("content-length") {
            for value in header.value.split(|byte| *byte == b',') {
                let value = trim(value);
                if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                    return Err(McpHttpError::Protocol);
                }
                let parsed = value
                    .iter()
                    .try_fold(0u64, |total, byte| {
                        total.checked_mul(10)?.checked_add(u64::from(*byte - b'0'))
                    })
                    .ok_or(McpHttpError::Protocol)?;
                if length.is_some_and(|current| current != parsed) {
                    return Err(McpHttpError::Protocol);
                }
                length = Some(parsed);
            }
        } else if header.name.eq_ignore_ascii_case("transfer-encoding") {
            if transfer
                || !trim(header.value).eq_ignore_ascii_case(b"chunked")
                || version != Some(1)
            {
                return Err(McpHttpError::Protocol);
            }
            transfer = true;
        } else if header.name.eq_ignore_ascii_case("content-encoding")
            && !trim(header.value).eq_ignore_ascii_case(b"identity")
        {
            return Err(McpHttpError::Protocol);
        }
    }
    if transfer && length.is_some() {
        return Err(McpHttpError::Protocol);
    }
    if status < 200 || status == 204 {
        if transfer || length.is_some() {
            return Err(McpHttpError::Protocol);
        }
        return Ok(Framing::Done);
    }
    if status == 304 {
        return Ok(Framing::Done);
    }
    if transfer {
        return Ok(Framing::ChunkSize);
    }
    match length {
        Some(value) if value > maximum => Err(McpHttpError::Limit),
        Some(0) => Ok(Framing::Done),
        Some(value) => Ok(Framing::Length(value)),
        None => Ok(Framing::Eof),
    }
}

fn trim(mut bytes: &[u8]) -> &[u8] {
    while bytes
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        bytes = &bytes[1..];
    }
    while bytes
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

pub(super) async fn trailers(io: &mut Buffered, limits: McpHttpLimits) -> Result<McpHttpHeaders> {
    let bytes = head(io, limits.head_bytes).await?;
    let mut slots = vec![httparse::EMPTY_HEADER; limits.header_count];
    let httparse::Status::Complete((consumed, headers)) =
        httparse::parse_headers(&bytes, &mut slots).map_err(|_| McpHttpError::Protocol)?
    else {
        return Err(McpHttpError::Protocol);
    };
    if consumed != bytes.len()
        || headers.iter().any(|header| {
            matches!(
                header.name.to_ascii_lowercase().as_str(),
                "content-length" | "transfer-encoding" | "content-encoding" | "host" | "trailer"
            )
        })
    {
        return Err(McpHttpError::Protocol);
    }
    Ok(owned(headers))
}

#[cfg(test)]
mod tests;
