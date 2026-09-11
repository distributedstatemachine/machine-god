use super::{Error, McpHostClassification, Result};

pub(super) fn classify_host(host: &[u8]) -> McpHostClassification {
    if host.split(|byte| *byte == b'.').any(|label| {
        label
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"xn--"))
    }) {
        McpHostClassification::Punycode
    } else if !host.is_ascii() {
        McpHostClassification::NonAscii
    } else {
        McpHostClassification::Ordinary
    }
}
pub(super) fn url_host(input: &str) -> Result<Box<[u8]>> {
    if input.is_empty() || input.trim_matches([' ', '\t', '\r', '\n']) != input {
        return Err(Error::InvalidUrl);
    }
    let parsed = parse_uri(input)?;
    if parsed.user {
        return Err(Error::InvalidUrl);
    }
    let host = parsed.host.ok_or(Error::InvalidUrl)?;
    let mut decoded = Vec::with_capacity(host.len().min(255));
    if host.contains('%') {
        let bytes = host.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let mut byte = bytes[index];
            if byte == b'%'
                && let (Some(high), Some(low)) =
                    (bytes.get(index + 1).copied(), bytes.get(index + 2).copied())
                && let Some(value) = percent_pair(high, low)
            {
                byte = value;
                index += 2;
            }
            if decoded.len() == 255 {
                return Err(Error::InvalidUrl);
            }
            decoded.push(byte);
            index += 1;
        }
    } else {
        decoded.extend_from_slice(host.as_bytes());
    }
    if decoded.is_empty() {
        return Err(Error::InvalidUrl);
    }
    if !parsed.scheme.eq_ignore_ascii_case("https")
        && !(parsed.scheme.eq_ignore_ascii_case("http")
            && (decoded.eq_ignore_ascii_case(b"localhost")
                || decoded == b"127.0.0.1"
                || decoded == b"[::1]"))
    {
        return Err(Error::InsecureUrl);
    }
    Ok(decoded.into_boxed_slice())
}
fn hex(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}
fn port(text: &str) -> Result<()> {
    digits(text.as_bytes()).map(|_| ()).ok_or(Error::InvalidUrl)
}
fn percent_pair(first: u8, second: u8) -> Option<u8> {
    match first {
        b'+' => hex(second),
        b'-' if second == b'0' => Some(0),
        _ => Some(hex(first)? * 16 + hex(second)?),
    }
}

struct ParsedUri<'a> {
    scheme: &'a str,
    host: Option<&'a str>,
    user: bool,
}
fn parse_uri(value: &str) -> Result<ParsedUri<'_>> {
    let (scheme, tail) = value.split_once(':').ok_or(Error::InvalidUrl)?;
    if scheme.is_empty()
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        return Err(Error::InvalidUrl);
    }
    let mut parsed = ParsedUri {
        scheme,
        host: None,
        user: false,
    };
    let Some(tail) = tail.strip_prefix("//") else {
        return Ok(parsed);
    };
    let authority = tail
        .split(['/', '?', '#'])
        .next()
        .ok_or(Error::InvalidUrl)?;
    if authority.is_empty() {
        return if tail.starts_with('/') {
            Ok(parsed)
        } else {
            Err(Error::InvalidUrl)
        };
    }
    let host = if let Some((_, host)) = authority.split_once('@') {
        parsed.user = true;
        host
    } else {
        authority
    };
    if host.is_empty() {
        return Ok(parsed);
    }
    if host.starts_with(']') {
        return Err(Error::InvalidUrl);
    }
    let mut end = host.len();
    if host.starts_with('[') {
        end = host.rfind(']').ok_or(Error::InvalidUrl)? + 1;
        if let Some(index) = host.rfind(':')
            && index >= end
        {
            port(&host[index + 1..])?;
        }
    } else if let Some(index) = host.rfind(':') {
        end = index;
        port(&host[index + 1..])?;
    }
    if end == 0 {
        return Err(Error::InvalidUrl);
    }
    parsed.host = Some(&host[..end]);
    Ok(parsed)
}

/// Pinned URI format checks syntax, not absolute network reachability or consent.
pub(super) fn uri(value: &str) -> bool {
    parse_uri(value).is_ok()
}
pub(super) fn format(name: &str, value: &str) -> bool {
    match name {
        "email" => value.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.contains('@') && domain.contains('.')
        }),
        "uri" => uri(value),
        "date" => date(value),
        "date-time" => date_time(value),
        _ => false,
    }
}
fn date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let (Some(year), Some(month), Some(day)) = (
        digits(&bytes[..4]),
        digits(&bytes[5..7]),
        digits(&bytes[8..]),
    ) else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    day <= days[usize::from(month - 1)]
}
fn digits(bytes: &[u8]) -> Option<u16> {
    let negative = bytes.first() == Some(&b'-');
    let bytes = if matches!(bytes.first(), Some(b'+' | b'-')) {
        &bytes[1..]
    } else {
        bytes
    };
    if bytes.is_empty() || bytes.first() == Some(&b'_') || bytes.last() == Some(&b'_') {
        return None;
    }
    let value = bytes.iter().try_fold(0_u16, |value, byte| {
        if *byte == b'_' {
            return Some(value);
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        value.checked_mul(10)?.checked_add(u16::from(byte - b'0'))
    })?;
    (!negative || value == 0).then_some(value)
}
fn date_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || !value.get(..10).is_some_and(date)
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    let (Some(hour), Some(minute), Some(second)) = (
        digits(&bytes[11..13]),
        digits(&bytes[14..16]),
        digits(&bytes[17..19]),
    ) else {
        return false;
    };
    if hour > 23 || minute > 59 || second > 60 {
        return false;
    }
    let mut index = 19;
    if bytes[index] == b'.' {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    if index + 1 == bytes.len() && matches!(bytes[index], b'Z' | b'z') {
        return true;
    }
    if index + 6 != bytes.len() || !matches!(bytes[index], b'+' | b'-') || bytes[index + 3] != b':'
    {
        return false;
    }
    digits(&bytes[index + 1..index + 3]).is_some_and(|v| v <= 23)
        && digits(&bytes[index + 4..]).is_some_and(|v| v <= 59)
}

const SINGLE: &[&str] = &[
    "token",
    "secret",
    "credential",
    "credentials",
    "password",
    "passcode",
    "passphrase",
    "otp",
    "pin",
    "apikey",
    "apitoken",
    "accesstoken",
    "refreshtoken",
    "authtoken",
    "bearertoken",
    "privatekey",
    "secretkey",
    "signingkey",
    "sshkey",
    "clientsecret",
    "authorizationcode",
    "recoverycode",
    "backupcode",
    "seedphrase",
    "recoveryphrase",
    "creditcard",
    "cardnumber",
    "securitycode",
    "paymentcredential",
    "paymentcredentials",
    "bankaccount",
    "socialsecurity",
    "cvv",
    "cvc",
    "paymentcard",
    "paymenttoken",
];
const PAIRS: &[(&str, &str)] = &[
    ("api", "key"),
    ("api", "token"),
    ("access", "token"),
    ("refresh", "token"),
    ("auth", "token"),
    ("bearer", "token"),
    ("private", "key"),
    ("secret", "key"),
    ("signing", "key"),
    ("ssh", "key"),
    ("client", "secret"),
    ("authorization", "code"),
    ("recovery", "code"),
    ("backup", "code"),
    ("seed", "phrase"),
    ("recovery", "phrase"),
    ("credit", "card"),
    ("card", "number"),
    ("security", "code"),
    ("payment", "credential"),
    ("payment", "credentials"),
    ("payment", "card"),
    ("payment", "token"),
    ("bank", "account"),
    ("social", "security"),
];
pub(super) fn secret(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut cursor = 0;
    let mut previous: Option<&str> = None;
    while cursor < bytes.len() {
        while cursor < bytes.len() && !bytes[cursor].is_ascii_alphanumeric() {
            cursor += 1;
        }
        if cursor == bytes.len() {
            break;
        }
        let start = cursor;
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_alphanumeric() {
            if bytes[cursor].is_ascii_uppercase()
                && (bytes[cursor - 1].is_ascii_lowercase() || bytes[cursor - 1].is_ascii_digit())
            {
                break;
            }
            if bytes[cursor].is_ascii_lowercase()
                && bytes[cursor - 1].is_ascii_uppercase()
                && cursor > start + 1
                && bytes[cursor - 2].is_ascii_uppercase()
            {
                cursor -= 1;
                break;
            }
            cursor += 1;
        }
        let word = &value[start..cursor];
        if SINGLE.iter().any(|term| word.eq_ignore_ascii_case(term))
            || previous.is_some_and(|prior| {
                PAIRS.iter().any(|(left, right)| {
                    prior.eq_ignore_ascii_case(left) && word.eq_ignore_ascii_case(right)
                })
            })
        {
            return true;
        }
        previous = Some(word);
    }
    false
}
