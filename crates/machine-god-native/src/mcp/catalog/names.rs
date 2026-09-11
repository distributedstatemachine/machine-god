use super::{McpCatalogError as Error, Result};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Names {
    used: BTreeSet<Box<str>>,
    next: BTreeMap<String, usize>,
    attempts: usize,
    maximum: usize,
}
impl Names {
    pub fn new(reserved: &[&str], maximum: usize) -> Self {
        Self {
            used: reserved.iter().copied().map(Into::into).collect(),
            next: BTreeMap::new(),
            attempts: 0,
            maximum,
        }
    }
    pub fn allocate(&mut self, server: &str, remote: &str) -> Result<Box<str>> {
        let mut base = String::from("mcp_");
        for byte in server
            .bytes()
            .chain(std::iter::once(b'_'))
            .chain(remote.bytes())
        {
            if base.len() == 64 {
                break;
            }
            base.push(char::from(
                if byte.is_ascii_alphanumeric() || b"_-".contains(&byte) {
                    byte
                } else {
                    b'_'
                },
            ));
        }
        let mut suffix = self.next.get(&base).copied().unwrap_or(1);
        loop {
            self.attempts = self.attempts.checked_add(1).ok_or(Error::Limit)?;
            if self.attempts > self.maximum {
                return Err(Error::Limit);
            }
            let candidate = if suffix == 1 {
                base.clone()
            } else {
                let suffix = format!("_{suffix}");
                format!("{}{}", &base[..base.len().min(64 - suffix.len())], suffix)
            };
            suffix = suffix.checked_add(1).ok_or(Error::Limit)?;
            if self.used.insert(candidate.clone().into_boxed_str()) {
                self.next.insert(base, suffix);
                return Ok(candidate.into_boxed_str());
            }
        }
    }
}
pub(super) fn valid_reserved(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}
pub(super) fn tags(server: &str, remote: &str) -> Box<[Box<str>]> {
    let mut tags: Vec<Box<str>> = vec!["mcp".into()];
    for text in [server, remote] {
        for token in
            text.split(|value: char| !value.is_ascii_alphanumeric() && value != '_' && value != '-')
        {
            if token.is_empty() || tags.len() == 16 {
                continue;
            }
            let token = token.to_ascii_lowercase().into_boxed_str();
            if !tags.contains(&token) {
                tags.push(token);
            }
        }
    }
    tags.into_boxed_slice()
}
