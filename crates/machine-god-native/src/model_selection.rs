//! Bounded pure pinned model-query matching, without fetching or fallback effects.

use std::fmt;

use machine_god_core::ModelCatalog;

use crate::{AI_GATEWAY_MODEL_CATALOG_MAX_MODEL_ID_BYTES, AI_GATEWAY_MODEL_CATALOG_MAX_MODELS};

/// Query byte bound, independent of the pinned interactive picker's smaller buffer.
pub const MAX_NATIVE_MODEL_QUERY_BYTES: usize = crate::MAX_NATIVE_SLASH_INPUT_BYTES;

/// Fixed failures that never retain or echo a query or catalog entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeModelSelectionError {
    QueryTooLong,
    CatalogLimit,
}

impl fmt::Display for NativeModelSelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::QueryTooLong => "native model query exceeds its byte limit",
            Self::CatalogLimit => "native model catalog exceeds its matching limit",
        })
    }
}

impl std::error::Error for NativeModelSelectionError {}

/// Returns the exact catalog spelling of a match, or `None` for caller fallback.
///
/// Catalog order breaks ties. ASCII-case-insensitive exact matching precedes
/// fuzzy matching. The query is not trimmed: slash payload trimming belongs to
/// routing, while the pinned pure scorer treats edge spaces as query bytes.
/// Empty queries produce no match. Matching makes no allocation until copying
/// the one bounded selected ID and never infers model capabilities.
///
/// # Errors
/// Rejects queries above the native slash-input byte bound; rejects
/// catalogs above 512 entries or 24 KiB aggregate ID bytes before any match.
pub fn resolve_model_query(
    query: &str,
    catalog: &ModelCatalog,
) -> Result<Option<String>, NativeModelSelectionError> {
    if query.len() > MAX_NATIVE_MODEL_QUERY_BYTES {
        return Err(NativeModelSelectionError::QueryTooLong);
    }
    let models = catalog.models();
    if models.len() > AI_GATEWAY_MODEL_CATALOG_MAX_MODELS {
        return Err(NativeModelSelectionError::CatalogLimit);
    }
    let mut bytes = 0_usize;
    for model in models {
        bytes = bytes
            .checked_add(model.id().len())
            .filter(|bytes| *bytes <= AI_GATEWAY_MODEL_CATALOG_MAX_MODEL_ID_BYTES)
            .ok_or(NativeModelSelectionError::CatalogLimit)?;
    }
    if query.is_empty() {
        return Ok(None);
    }
    for model in models {
        if model.id().eq_ignore_ascii_case(query) {
            return Ok(Some(model.id().to_owned()));
        }
    }
    let query = Query::new(query.as_bytes());
    let mut best_score = 0;
    let mut best = None;
    for model in models {
        let score = query.score(model.id().as_bytes());
        if score > best_score {
            best_score = score;
            best = Some(model.id());
        }
    }
    Ok(best.map(str::to_owned))
}

struct Query<'a> {
    bytes: &'a [u8],
    tokens: [&'a [u8]; 16],
    token_count: usize,
}

impl<'a> Query<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let mut query = Self {
            bytes,
            tokens: [&[]; 16],
            token_count: 0,
        };
        for token in bytes
            .split(|byte| matches!(byte, b' ' | b'-' | b'/' | b'_'))
            .filter(|token| !token.is_empty())
            .take(16)
        {
            query.tokens[query.token_count] = token;
            query.token_count += 1;
        }
        query
    }

    fn score(&self, id: &[u8]) -> usize {
        if id.is_empty() || self.bytes.is_empty() {
            return 0;
        }
        if contains_ignore_ascii_case(id, self.bytes) {
            let bonus = self.bytes.len().min(50);
            if id.starts_with(self.bytes)
                || (id.len() > self.bytes.len()
                    && id[id.len() - self.bytes.len() - 1] == b'/'
                    && id[id.len() - self.bytes.len()..].eq_ignore_ascii_case(self.bytes))
            {
                return 120 + bonus;
            }
            return 100 + bonus;
        }
        let tokens = &self.tokens[..self.token_count];
        if tokens.len() > 1
            && tokens
                .iter()
                .all(|token| contains_ignore_ascii_case(id, token))
        {
            return 80 + tokens.len() * 5;
        }
        let mut matched = 0;
        for byte in id {
            if matched < self.bytes.len() && byte.eq_ignore_ascii_case(&self.bytes[matched]) {
                matched += 1;
            }
        }
        if matched == self.bytes.len() {
            return 40 + matched.min(20);
        }
        if tokens
            .iter()
            .any(|token| contains_ignore_ascii_case(id, token))
        {
            return 20;
        }
        0
    }
}

fn contains_ignore_ascii_case(id: &[u8], query: &[u8]) -> bool {
    !query.is_empty()
        && id
            .windows(query.len())
            .any(|window| window.eq_ignore_ascii_case(query))
}

#[cfg(test)]
mod tests {
    use super::Query;

    #[test]
    fn exact_pinned_scores_and_bonus_caps() {
        for (id, query, expected) in [
            ("model/other", "model", 125),
            ("MODEL/other", "model", 105),
            ("provider/MODEL", "model", 125),
            ("provider/model-end", "model", 105),
            ("alphaXXbeta", "alpha beta", 90),
            ("axbyc", "abc", 43),
            ("alpha", "alpha absent", 20),
            ("alpha", "xyz", 0),
            ("", "query", 0),
            ("model", "", 0),
            ("model", " model ", 20),
            ("model", "model.suffix", 0),
        ] {
            assert_eq!(Query::new(query.as_bytes()).score(id.as_bytes()), expected);
        }
        assert_eq!(Query::new(&[b'x'; 80]).score(&[b'x'; 100]), 170);
        assert_eq!(Query::new(&[b'x'; 80]).score(&[b'y'; 100]), 0);
        let interleaved = "ax".repeat(40);
        assert_eq!(Query::new(&[b'a'; 40]).score(interleaved.as_bytes()), 60);
    }
}
