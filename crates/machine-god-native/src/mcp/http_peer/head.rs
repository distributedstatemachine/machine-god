use super::{McpHttpAuthentication, McpHttpPeerError, Result};
use crate::mcp::http::{McpHttpHeaders, McpHttpResponse};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Media {
    Json,
    Sse,
}
pub(super) fn singleton<'a>(headers: &'a McpHttpHeaders, name: &str) -> Result<Option<&'a [u8]>> {
    let mut matches = headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name));
    let result = matches.next().map(|(_, value)| value);
    if matches.next().is_some() {
        return Err(McpHttpPeerError::Protocol);
    }
    Ok(result)
}
pub(super) fn media(headers: &McpHttpHeaders) -> Result<Media> {
    let value = singleton(headers, "content-type")?.ok_or(McpHttpPeerError::Protocol)?;
    let value = value
        .split(|byte| *byte == b';')
        .next()
        .unwrap_or_default()
        .trim_ascii();
    if value.eq_ignore_ascii_case(b"application/json") {
        Ok(Media::Json)
    } else if value.eq_ignore_ascii_case(b"text/event-stream") {
        Ok(Media::Sse)
    } else {
        Err(McpHttpPeerError::Protocol)
    }
}
pub(super) fn status(response: &McpHttpResponse, has_session: bool) -> Result<()> {
    if (300..400).contains(&response.status) {
        return Err(McpHttpPeerError::Redirect);
    }
    let mut challenges = Vec::new();
    let mut bytes = 0usize;
    for (_, value) in response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("www-authenticate"))
    {
        bytes = bytes
            .checked_add(value.len())
            .ok_or(McpHttpPeerError::Limit)?;
        if challenges.len() == 8 || bytes > 16 * 1024 {
            return Err(McpHttpPeerError::Limit);
        }
        challenges.push(value.into());
    }
    if response.status == 401 || response.status == 403 && !challenges.is_empty() {
        return Err(McpHttpPeerError::Authentication(McpHttpAuthentication {
            status: response.status,
            challenges: challenges.into_boxed_slice(),
        }));
    }
    if response.status == 404 && has_session {
        return Err(McpHttpPeerError::SessionExpired);
    }
    Ok(())
}
pub(super) fn session(headers: &McpHttpHeaders) -> Result<Option<Box<str>>> {
    singleton(headers, "mcp-session-id")?
        .map(|bytes| {
            if bytes.is_empty()
                || bytes.len() > 1024
                || !bytes.iter().all(|byte| (0x21..=0x7e).contains(byte))
            {
                return Err(McpHttpPeerError::Protocol);
            }
            Ok(std::str::from_utf8(bytes)
                .map_err(|_| McpHttpPeerError::Protocol)?
                .into())
        })
        .transpose()
}
pub(super) fn stable_session(headers: &McpHttpHeaders, selected: Option<&str>) -> Result<()> {
    if let Some(observed) = session(headers)?
        && Some(observed.as_ref()) != selected
    {
        return Err(McpHttpPeerError::Protocol);
    }
    Ok(())
}
