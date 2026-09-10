//! Effect-free detection from an explicitly captured, bounded terminal snapshot.

use std::fmt;

use ::url::{Host, Url};

pub(crate) const MAX_BACKGROUND_URL_INPUT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_BACKGROUND_SERVER_URL_BYTES: usize = 2048;

/// An HTTP(S) URL validated for a separately authorized opener, never raw output.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BackgroundServerUrl(String);

impl BackgroundServerUrl {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackgroundServerUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackgroundServerUrl([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackgroundUrlError {
    ResourceLimit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackgroundUrlCaptureEnd {
    Snapshot,
    Truncated,
}

/// Invalid and overlong candidates are ignored, never shortened into a URL.
/// An oversized snapshot is rejected before scanning; callers own its capture.
/// A truncated capture cannot establish termination of its final candidate.
pub(crate) fn detect_server_url(
    captured: &[u8],
    end: BackgroundUrlCaptureEnd,
) -> Result<Option<BackgroundServerUrl>, BackgroundUrlError> {
    if captured.len() > MAX_BACKGROUND_URL_INPUT_BYTES {
        return Err(BackgroundUrlError::ResourceLimit);
    }

    let mut best: Option<(i32, BackgroundServerUrl)> = None;
    let mut lines = captured.split(|byte| *byte == b'\n').peekable();
    while let Some(line) = lines.next() {
        // Fixed hint patterns scan each line only once each, never per candidate.
        let hint_score = line_hint_score(line);
        let mut offset = 0;
        while offset < line.len() {
            let remainder = &line[offset..];
            if !starts_with_scheme(remainder) {
                offset += 1;
                continue;
            }
            let delimiter = remainder.iter().position(|byte| is_delimiter(*byte));
            if delimiter.is_none()
                && lines.peek().is_none()
                && end == BackgroundUrlCaptureEnd::Truncated
            {
                // EOF here is a byte/page/gap boundary, not the URL's end.
                // Earlier delimited candidates remain eligible.
                break;
            }
            let length = delimiter.unwrap_or(remainder.len());
            let candidate = &remainder[..length];
            // Advance over every candidate, including invalid/overlong ones.
            // Each byte participates in at most one candidate validation.
            offset += length;
            if let Some((host_score, url)) = validate_candidate(candidate) {
                let score = host_score + hint_score;
                if best.as_ref().is_none_or(|(previous, _)| score >= *previous) {
                    best = Some((score, url));
                }
            }
        }
    }
    Ok(best.map(|(_, url)| url))
}

fn starts_with_scheme(bytes: &[u8]) -> bool {
    bytes
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"http://"))
        || bytes
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"https://"))
}

fn is_delimiter(byte: u8) -> bool {
    byte.is_ascii_control()
        || byte.is_ascii_whitespace()
        || matches!(byte, b')' | b'"' | b'\'' | b'<' | b'>' | b'`')
}

fn validate_candidate(bytes: &[u8]) -> Option<(i32, BackgroundServerUrl)> {
    if bytes.len() > MAX_BACKGROUND_SERVER_URL_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    if text.chars().any(|ch| ch.is_control() || ch.is_whitespace()) || text.contains('\\') {
        return None;
    }
    let (_, after_scheme) = text.split_once("://")?;
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    // WHATWG parsing repairs an absent authority or backslashes. Do not admit
    // such repairs, and exclude all userinfo (even the otherwise empty `@`).
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let parsed = Url::parse(text).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let host_score = match parsed.host()? {
        Host::Domain(host) if host.eq_ignore_ascii_case("localhost") => 100,
        Host::Ipv4(host) if host.octets() == [127, 0, 0, 1] => 100,
        Host::Ipv6(host) if host.is_loopback() => 100,
        Host::Ipv4(host) if host.is_unspecified() => 80,
        // The pinned rank includes every 172.* address, not just RFC1918.
        Host::Ipv4(host)
            if host.octets()[..2] == [192, 168] || matches!(host.octets()[0], 10 | 172) =>
        {
            40
        }
        _ => 0,
    };
    // URL serialization can expand Unicode, so admission bounds both forms.
    if parsed.as_str().len() > MAX_BACKGROUND_SERVER_URL_BYTES {
        return None;
    }
    Some((host_score, BackgroundServerUrl(parsed.into())))
}

fn line_hint_score(line: &[u8]) -> i32 {
    let contains = |pattern: &[u8]| {
        line.windows(pattern.len())
            .any(|window| window.eq_ignore_ascii_case(pattern))
    };
    let mut score = 0;
    if contains(b"local") {
        score += 20;
    }
    if contains(b"url") {
        score += 6;
    }
    if [b"ready".as_slice(), b"started", b"listening", b"server"]
        .into_iter()
        .any(contains)
    {
        score += 10;
    }
    if contains(b"network") {
        score -= 15;
    }
    if contains(b"error") || contains(b"warn") {
        score -= 25;
    }
    score
}

#[cfg(test)]
#[path = "url/tests.rs"]
mod tests;
