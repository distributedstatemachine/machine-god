//! Pinned `text_utils.maskSecrets` spans shared by action rejection and proven
//! root-text masking. Matching never creates authority or retains diagnostics.

use std::ops::Range;

fn key(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn token(b: u8) -> bool {
    key(b) || b == b'-' || b == b'.'
}
fn sensitive(key: &[u8]) -> bool {
    [
        "password",
        "api_key",
        "apikey",
        "secret",
        "token",
        "private_key",
        "access_key",
    ]
    .iter()
    .any(|word| {
        key.windows(word.len())
            .any(|part| part.eq_ignore_ascii_case(word.as_bytes()))
    }) || key.eq_ignore_ascii_case(b"passwd")
}
fn assignment(bytes: &[u8], mut start: usize) -> Option<Range<usize>> {
    let first = *bytes.get(start)?;
    let quoted = matches!(first, b'\'' | b'"');
    if quoted {
        start += 1;
    }
    let mut end = start;
    while let Some(&b) = bytes.get(end) {
        if b == b'\n'
            || b == b'\r'
            || if quoted {
                b == first
            } else {
                b.is_ascii_whitespace() || matches!(b, b'\'' | b'"')
            }
        {
            break;
        }
        end += 1;
    }
    (end > start).then_some(start..end)
}
fn aws(bytes: &[u8], i: usize) -> Option<Range<usize>> {
    let tail = &bytes[i..];
    (tail.len() >= 20
        && [b"AKIA", b"ASIA", b"AIDA", b"AGPA", b"AROA", b"ANPA"]
            .iter()
            .any(|p| tail.starts_with(*p))
        && tail[..20]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && (i == 0 || !token(bytes[i - 1]))
        && tail.get(20).is_none_or(|b| !token(*b)))
    .then_some(i..i + 20)
}
fn inline(bytes: &[u8], i: usize) -> Option<Range<usize>> {
    if i > 0 && token(bytes[i - 1]) {
        return None;
    }
    let tail = &bytes[i..];
    for prefix in [
        "sk-",
        "sk_live_",
        "pk_live_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "Bearer ",
    ] {
        if tail.starts_with(prefix.as_bytes()) {
            let len = prefix.len()
                + tail[prefix.len()..]
                    .iter()
                    .take_while(|b| token(**b))
                    .count();
            if len >= 16 {
                return Some(i..i + len);
            }
        }
    }
    if [b"ghp_", b"gho_", b"ghu_", b"ghs_", b"ghr_"]
        .iter()
        .any(|p| tail.starts_with(*p))
    {
        let len = tail[4..].iter().take_while(|b| token(**b)).count();
        if len >= 36 {
            return Some(i..i + 4 + len);
        }
    }
    None
}
fn span(bytes: &[u8], i: usize) -> Option<Range<usize>> {
    let tail = &bytes[i..];
    if tail.starts_with(b"https://") {
        let mut colon = false;
        for (offset, &b) in tail[8..].iter().enumerate() {
            if b.is_ascii_whitespace() || matches!(b, b'/' | b'?' | b'#') {
                break;
            }
            if b == b':' {
                colon = true;
            }
            if b == b'@' {
                if colon {
                    return Some(i + 8..i + 8 + offset);
                }
                break;
            }
        }
    }
    if let Some(span) = aws(bytes, i) {
        return Some(span);
    }
    if (i == 0 || !key(bytes[i - 1])) && key(bytes[i]) {
        let n = tail.iter().take_while(|b| key(**b)).count();
        if tail.get(n) == Some(&b'=')
            && sensitive(&tail[..n])
            && let Some(span) = assignment(bytes, i + n + 1)
        {
            return Some(span);
        }
    }
    for prefix in [
        "OPENAI_API_KEY=",
        "ANTHROPIC_API_KEY=",
        "AI_GATEWAY_API_KEY=",
        "VERCEL_OIDC_TOKEN=",
        "GITHUB_TOKEN=",
        "AWS_SECRET_ACCESS_KEY=",
        "DATABASE_URL=",
        "SECRET_KEY=",
        "API_KEY=",
        "TOKEN=",
        "PASSWORD=",
        "PRIVATE_KEY=",
    ] {
        if tail.len() > prefix.len()
            && tail[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
            && let Some(span) = assignment(bytes, i + prefix.len())
        {
            return Some(span);
        }
    }
    inline(bytes, i)
}
pub(super) fn contains_secret(text: &str) -> bool {
    contains_secret_bytes(text.as_bytes())
}
pub(super) fn contains_secret_bytes(bytes: &[u8]) -> bool {
    (0..bytes.len()).any(|i| span(bytes, i).is_some())
}

/// Bounded masking, only for already-proven root-user text. Action review still
/// rejects matching spans rather than approving masked/incomplete evidence.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn mask_root_text(text: &str, max_bytes: usize) -> Option<String> {
    if text.len() > max_bytes {
        return None;
    }
    let limit = max_bytes.checked_mul(3)?;
    let mut output = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if let Some(secret) = span(bytes, i) {
            if output.len().checked_add(secret.start - i + 10)? > limit {
                return None;
            }
            output.extend_from_slice(&bytes[i..secret.start]);
            output.extend_from_slice(b"[redacted]");
            i = secret.end;
        } else {
            if output.len() == limit {
                return None;
            }
            output.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(output).ok()
}
