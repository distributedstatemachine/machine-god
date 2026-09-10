//! Pinned leading-invocation grammar, not fuzzy or arbitrary inline matching.

use crate::skills_metadata::MAX_NATIVE_SKILL_METADATA_NAME_BYTES;

pub(super) struct PromptReference<'a> {
    sigil: Option<&'a [u8]>,
    natural: Option<NaturalReference>,
}

impl<'a> PromptReference<'a> {
    pub(super) fn new(prompt: &'a str) -> Self {
        let text = trim_start(prompt.as_bytes(), b" \t\r\n");
        let sigil = matches!(text.first(), Some(b'$' | b'/')).then_some(text);
        Self {
            sigil,
            natural: NaturalReference::new(prompt.as_bytes()),
        }
    }

    pub(super) fn can_match(&self) -> bool {
        self.sigil.is_some() || self.natural.is_some()
    }

    pub(super) fn matches(&self, name: &str) -> bool {
        self.sigil
            .is_some_and(|text| matches_sigil(text, name.as_bytes()))
            || self
                .natural
                .as_ref()
                .is_some_and(|reference| reference.matches(name.as_bytes()))
    }
}

fn matches_sigil(text: &[u8], name: &[u8]) -> bool {
    let end = 1 + name.len();
    text.get(1..end)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
        && text
            .get(end)
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-'))
}

struct NaturalReference {
    normalized: Vec<u8>,
    name_start: usize,
    optional_the_start: Option<usize>,
}

impl NaturalReference {
    fn new(prompt: &[u8]) -> Option<Self> {
        let mut text = trim_start(prompt, b" \t\r\n");
        if text
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"please"))
            && text.get(6).is_none_or(|byte| !byte.is_ascii_alphanumeric())
        {
            text = trim_start(&text[6..], b" \t\r\n,:");
        }
        if text.is_empty()
            || matches!(text.first(), Some(b'"' | b'\'' | b'`'))
            || text.starts_with("“".as_bytes())
            || text.starts_with("‘".as_bytes())
        {
            return None;
        }
        let mut normalized = Vec::with_capacity(text.len());
        normalize(text, |byte| normalized.push(byte));
        let verb = ["use", "apply", "activate", "invoke", "run"]
            .into_iter()
            .find(|verb| {
                normalized.starts_with(verb.as_bytes()) && normalized.get(verb.len()) == Some(&b' ')
            })?;
        let name_start = verb.len() + 1;
        let optional_the_start = normalized[name_start..]
            .starts_with(b"the ")
            .then_some(name_start + 4);
        Some(Self {
            normalized,
            name_start,
            optional_the_start,
        })
    }

    fn matches(&self, name: &[u8]) -> bool {
        // Metadata already bounds names. A fixed scratch buffer avoids a prompt
        // copy or a fresh allocation per catalog entry.
        let mut normalized = [0_u8; MAX_NATIVE_SKILL_METADATA_NAME_BYTES];
        let mut length = 0;
        normalize(name, |byte| {
            if let Some(slot) = normalized.get_mut(length) {
                *slot = byte;
            }
            length += 1;
        });
        if length == 0 || length > normalized.len() {
            return false;
        }
        let name = &normalized[..length];
        [Some(self.name_start), self.optional_the_start]
            .into_iter()
            .flatten()
            .any(|start| {
                let text = &self.normalized[start..];
                let end = name.len() + b" skill".len();
                text.starts_with(name)
                    && text.get(name.len()..end) == Some(&b" skill"[..])
                    && text.get(end).is_none_or(|byte| *byte == b' ')
            })
    }
}

fn trim_start<'a>(bytes: &'a [u8], separators: &[u8]) -> &'a [u8] {
    let start = bytes
        .iter()
        .position(|byte| !separators.contains(byte))
        .unwrap_or(bytes.len());
    &bytes[start..]
}

fn normalize(text: &[u8], mut emit: impl FnMut(u8)) {
    let mut seen = false;
    let mut space = false;
    for byte in text {
        if byte.is_ascii_alphanumeric() {
            if space {
                emit(b' ');
            }
            emit(byte.to_ascii_lowercase());
            seen = true;
            space = false;
        } else if seen {
            space = true;
        }
    }
}
