//! Bounded metadata shared by human-invoked skill discovery and management.
//!
//! This deliberately implements the pinned skill grammar, not YAML. Unknown
//! fields and the body are opaque. Parsing grants no authority or trust.

use std::fmt;

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
#[path = "skills_metadata/parser.rs"]
mod parser;
#[cfg(test)]
#[path = "skills_metadata/tests.rs"]
mod tests;

pub const MAX_NATIVE_SKILL_HEADER_BYTES: usize = 65_536;
pub const MAX_NATIVE_SKILL_METADATA_NAME_BYTES: usize = 256;
pub const MAX_NATIVE_SKILL_DESCRIPTION_BYTES: usize = 4_096;

/// Owned recognized metadata; debug never prints externally supplied text.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeSkillMetadata {
    pub name: String,
    pub description: String,
    /// Byte immediately after the closing delimiter (zero without a header).
    /// Leading blank body lines are deliberately not consumed by this parser.
    pub body_offset: usize,
    pub has_frontmatter: bool,
}

impl fmt::Debug for NativeSkillMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillMetadata")
            .field("has_frontmatter", &self.has_frontmatter)
            .finish_non_exhaustive()
    }
}

/// Fixed parser failures, without skill content or paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillMetadataError {
    HeaderTooLong,
    MissingClosingDelimiter,
    MissingName,
    DuplicateRecognizedKey,
    InvalidName,
    NameTooLong,
    DescriptionTooLong,
    MalformedQuote,
    UnsupportedMultiline,
    InvalidUtf8,
    ControlByte,
}

impl fmt::Display for NativeSkillMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skill metadata: {self:?}")
    }
}

impl std::error::Error for NativeSkillMetadataError {}

/// Parses at most the bounded metadata prefix; the body is never interpreted.
/// Quotes remove only matching outer quote bytes; no escape language is added.
///
/// # Errors
/// Returns a fixed error for malformed or oversized recognized metadata.
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
pub(crate) fn parse_skill_metadata(
    bytes: &[u8],
    fallback: &str,
) -> Result<NativeSkillMetadata, NativeSkillMetadataError> {
    parser::parse(bytes, fallback)
}

#[cfg(any(test, target_os = "linux", target_os = "macos"))]
pub(crate) fn header_start(bytes: &[u8]) -> Option<usize> {
    if bytes.starts_with(b"---\r\n") {
        Some(5)
    } else if bytes.starts_with(b"---\n") {
        Some(4)
    } else if bytes == b"---" {
        Some(3)
    } else {
        None
    }
}

/// Finds an exact bounded closing line, excluding an unterminated CR suffix.
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
pub(crate) fn closing_delimiter(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut offset = start;
    while offset <= bytes.len().min(MAX_NATIVE_SKILL_HEADER_BYTES) {
        let remaining = &bytes[offset..bytes.len().min(MAX_NATIVE_SKILL_HEADER_BYTES)];
        let newline = remaining.iter().position(|byte| *byte == b'\n');
        let end = newline.map_or(remaining.len(), |index| index);
        let raw = &remaining[..end];
        let line = if newline.is_some() {
            raw.strip_suffix(b"\r").unwrap_or(raw)
        } else {
            raw
        };
        if line == b"---" && (newline.is_some() || offset + end == bytes.len()) {
            return Some((offset, offset + end + usize::from(newline.is_some())));
        }
        offset += end + 1;
        if newline.is_none() {
            break;
        }
    }
    None
}
