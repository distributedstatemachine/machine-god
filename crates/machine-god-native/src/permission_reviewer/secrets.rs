//! Detection-only equivalent of pinned `text_utils.maskSecrets`. Any match makes
//! action evidence incomplete, so the reviewer rejects it without transmission.

fn key(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn token(b: u8) -> bool {
    key(b) || b == b'-' || b == b'.'
}
fn assignment_value(bytes: &[u8]) -> bool {
    match bytes.first() {
        Some(quote @ (b'\'' | b'"')) => bytes
            .get(1)
            .is_some_and(|b| b != quote && *b != b'\n' && *b != b'\r'),
        Some(b) => !b.is_ascii_whitespace() && *b != b'\'' && *b != b'"',
        None => false,
    }
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

pub(super) fn contains_secret(text: &str) -> bool {
    contains_secret_bytes(text.as_bytes())
}

pub(super) fn contains_secret_bytes(bytes: &[u8]) -> bool {
    for i in 0..bytes.len() {
        let tail = &bytes[i..];
        if tail.starts_with(b"https://") {
            let mut colon = false;
            for &b in &tail[8..] {
                if b.is_ascii_whitespace() || matches!(b, b'/' | b'?' | b'#') {
                    break;
                }
                if b == b':' {
                    colon = true;
                }
                if b == b'@' {
                    if colon {
                        return true;
                    }
                    break;
                }
            }
        }
        if (i == 0 || !key(bytes[i - 1])) && key(bytes[i]) {
            let n = tail.iter().take_while(|b| key(**b)).count();
            if tail.get(n) == Some(&b'=')
                && sensitive(&tail[..n])
                && assignment_value(&tail[n + 1..])
            {
                return true;
            }
        }
        // These legacy prefixes intentionally do not require a key boundary.
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
                && assignment_value(&tail[prefix.len()..])
            {
                return true;
            }
        }
        if i > 0 && token(bytes[i - 1]) {
            continue;
        }
        if tail.len() >= 20
            && [b"AKIA", b"ASIA", b"AIDA", b"AGPA", b"AROA", b"ANPA"]
                .iter()
                .any(|prefix| tail.starts_with(*prefix))
            && tail[..20]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && tail.get(20).is_none_or(|b| !token(*b))
        {
            return true;
        }
        for prefix in [
            "sk-",
            "sk_live_",
            "pk_live_",
            "github_pat_",
            "xoxb-",
            "xoxp-",
            "Bearer ",
        ] {
            if tail.starts_with(prefix.as_bytes())
                && prefix.len()
                    + tail[prefix.len()..]
                        .iter()
                        .take_while(|b| token(**b))
                        .count()
                    >= 16
            {
                return true;
            }
        }
        if [b"ghp_", b"gho_", b"ghu_", b"ghs_", b"ghr_"]
            .iter()
            .any(|prefix| tail.starts_with(*prefix))
            && tail[4..].iter().take_while(|b| token(**b)).count() >= 36
        {
            return true;
        }
    }
    false
}
