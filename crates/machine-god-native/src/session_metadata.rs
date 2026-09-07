//! Native presentation metadata. None of these values grant tool authority.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};

/// Reserved entry in a core record's otherwise provider-neutral metadata map.
pub const NATIVE_SESSION_METADATA_KEY: &str = "machine_god.native_session";
/// Maximum UTF-8 byte length of a nonempty session title after edge trimming.
pub const MAX_NATIVE_SESSION_TITLE_BYTES: usize = 240;
/// Pinned conversation-language byte limit after SP/TAB/CR/LF edge trimming.
pub const MAX_NATIVE_SESSION_LANGUAGE_BYTES: usize = 24;
/// Maximum byte length of a stored Unix workspace path.
pub const MAX_NATIVE_SESSION_WORKSPACE_BYTES: usize = 4096;

const FIELDS: &[&str] = &[
    "schema_version",
    "workspace_hex",
    "created_at_ms",
    "updated_at_ms",
    "title",
    "language",
    "origin",
];

/// Provenance explicitly supplied by the native session owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSessionOrigin {
    Cli,
    Recovered,
    Imported,
}

impl NativeSessionOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Recovered => "recovered",
            Self::Imported => "imported",
        }
    }
}

/// Bounded, versioned native metadata. Missing historical facts remain absent.
///
/// A workspace association is descriptive, not an opened filesystem capability.
/// Hosts must establish actual root authority independently before tool use.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct NativeSessionMetadata {
    workspace: Option<PathBuf>,
    created_at_ms: Option<i64>,
    updated_at_ms: Option<i64>,
    title: Option<String>,
    language: Option<String>,
    origin: Option<NativeSessionOrigin>,
}

impl fmt::Debug for NativeSessionMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSessionMetadata")
            .field("has_workspace", &self.workspace.is_some())
            .field("has_title", &self.title.is_some())
            .field("has_language", &self.language.is_some())
            .finish_non_exhaustive()
    }
}

/// Fixed, redacted metadata failure without retained user or record content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeSessionMetadataError {
    Malformed,
    UnsupportedVersion,
    InvalidWorkspace,
    InvalidTitle,
    InvalidLanguage,
    TimeRegression,
}

impl fmt::Display for NativeSessionMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "native session metadata is malformed",
            Self::UnsupportedVersion => "native session metadata version is unsupported",
            Self::InvalidWorkspace => "native session workspace association is invalid",
            Self::InvalidTitle => "native session title is invalid",
            Self::InvalidLanguage => "native session language is invalid",
            Self::TimeRegression => "native session metadata time would regress",
        })
    }
}

impl std::error::Error for NativeSessionMetadataError {}

impl NativeSessionMetadata {
    /// Records explicit facts for a newly created session, without I/O or a clock read.
    ///
    /// `canonical_workspace` must come from the host's verified root selection;
    /// this pure codec checks lexical shape but does not canonicalize or open it.
    ///
    /// # Errors
    /// Rejects relative, non-normalized, NUL-containing or oversized paths.
    pub fn new(
        canonical_workspace: &Path,
        now_ms: i64,
        origin: NativeSessionOrigin,
    ) -> Result<Self, NativeSessionMetadataError> {
        validate_workspace(canonical_workspace)?;
        Ok(Self {
            workspace: Some(canonical_workspace.to_owned()),
            created_at_ms: Some(now_ms),
            updated_at_ms: Some(now_ms),
            origin: Some(origin),
            ..Self::default()
        })
    }

    /// Decodes only the reserved entry; unrelated metadata is not traversed.
    /// An absent entry is legacy metadata with all historical facts unknown.
    ///
    /// # Errors
    /// Rejects unknown versions/fields, wrong types, invalid bounds or times.
    pub fn from_metadata(
        metadata: &BTreeMap<String, Value>,
    ) -> Result<Self, NativeSessionMetadataError> {
        let Some(value) = metadata.get(NATIVE_SESSION_METADATA_KEY) else {
            return Ok(Self::default());
        };
        Self::from_value(value)
    }

    fn from_value(value: &Value) -> Result<Self, NativeSessionMetadataError> {
        let object = value
            .as_object()
            .ok_or(NativeSessionMetadataError::Malformed)?;
        let version = object
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or(NativeSessionMetadataError::Malformed)?;
        if version != 1 {
            return Err(NativeSessionMetadataError::UnsupportedVersion);
        }
        if object.len() > FIELDS.len() || object.keys().any(|key| !FIELDS.contains(&key.as_str())) {
            return Err(NativeSessionMetadataError::Malformed);
        }
        let workspace = optional_string(object, "workspace_hex")?
            .map(decode_workspace)
            .transpose()?;
        let created_at_ms = optional_time(object, "created_at_ms")?;
        let updated_at_ms = optional_time(object, "updated_at_ms")?;
        if matches!((created_at_ms, updated_at_ms), (Some(created), Some(updated)) if updated < created)
        {
            return Err(NativeSessionMetadataError::TimeRegression);
        }
        let title = optional_string(object, "title")?
            .map(|title| {
                let validated = validate_title(title)?;
                if validated != title {
                    return Err(NativeSessionMetadataError::InvalidTitle);
                }
                Ok(title.to_owned())
            })
            .transpose()?;
        let language = optional_string(object, "language")?
            .map(|language| {
                if validate_language(language)? != language {
                    return Err(NativeSessionMetadataError::InvalidLanguage);
                }
                Ok(language.to_owned())
            })
            .transpose()?;
        let origin = match optional_string(object, "origin")? {
            None => None,
            Some("cli") => Some(NativeSessionOrigin::Cli),
            Some("recovered") => Some(NativeSessionOrigin::Recovered),
            Some("imported") => Some(NativeSessionOrigin::Imported),
            Some(_) => return Err(NativeSessionMetadataError::Malformed),
        };
        Ok(Self {
            workspace,
            created_at_ms,
            updated_at_ms,
            title,
            language,
            origin,
        })
    }

    /// Produces a bounded entry value, without altering a record or other keys.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "schema_version": 1,
            "workspace_hex": self.workspace.as_deref().map(encode_workspace),
            "created_at_ms": self.created_at_ms,
            "updated_at_ms": self.updated_at_ms,
            "title": self.title,
            "language": self.language,
            "origin": self.origin.map(NativeSessionOrigin::as_str),
        })
    }

    #[must_use]
    pub fn workspace(&self) -> Option<&Path> {
        self.workspace.as_deref()
    }

    #[must_use]
    pub const fn created_at_ms(&self) -> Option<i64> {
        self.created_at_ms
    }

    #[must_use]
    pub const fn updated_at_ms(&self) -> Option<i64> {
        self.updated_at_ms
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    #[must_use]
    pub const fn origin(&self) -> Option<NativeSessionOrigin> {
        self.origin
    }

    /// Stages a title and explicit update time together. Does not persist them.
    ///
    /// # Errors
    /// Rejects empty, over-240-byte, C0/DEL-containing titles and regressing time;
    /// no field changes on error. Only SP/TAB/CR/LF are trimmed at the edges.
    pub fn rename(&mut self, title: &str, now_ms: i64) -> Result<(), NativeSessionMetadataError> {
        let title = validate_title(title)?;
        self.validate_update_time(now_ms)?;
        self.title = Some(title.to_owned());
        self.updated_at_ms = Some(now_ms);
        Ok(())
    }

    /// Stages a bounded language tag and explicit update time without persistence.
    ///
    /// # Errors
    /// Rejects invalid tags or regressing time without changing any field.
    pub fn set_language(
        &mut self,
        language: &str,
        now_ms: i64,
    ) -> Result<(), NativeSessionMetadataError> {
        let language = validate_language(language)?;
        self.validate_update_time(now_ms)?;
        self.language = Some(language.to_owned());
        self.updated_at_ms = Some(now_ms);
        Ok(())
    }

    /// Stages an observed update time, never manufacturing a creation time.
    ///
    /// # Errors
    /// Rejects a value earlier than either known creation or update time.
    pub fn touch(&mut self, now_ms: i64) -> Result<(), NativeSessionMetadataError> {
        self.validate_update_time(now_ms)?;
        self.updated_at_ms = Some(now_ms);
        Ok(())
    }

    fn validate_update_time(&self, now_ms: i64) -> Result<(), NativeSessionMetadataError> {
        if self.created_at_ms.is_some_and(|created| now_ms < created)
            || self.updated_at_ms.is_some_and(|updated| now_ms < updated)
        {
            Err(NativeSessionMetadataError::TimeRegression)
        } else {
            Ok(())
        }
    }
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, NativeSessionMetadataError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(NativeSessionMetadataError::Malformed),
    }
}

fn optional_time(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<i64>, NativeSessionMetadataError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or(NativeSessionMetadataError::Malformed),
    }
}

fn validate_title(title: &str) -> Result<&str, NativeSessionMetadataError> {
    let title = title.trim_matches([' ', '\t', '\r', '\n']);
    if title.is_empty()
        || title.len() > MAX_NATIVE_SESSION_TITLE_BYTES
        || title.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
    {
        Err(NativeSessionMetadataError::InvalidTitle)
    } else {
        Ok(title)
    }
}

fn validate_language(language: &str) -> Result<&str, NativeSessionMetadataError> {
    let language = language.trim_matches([' ', '\t', '\r', '\n']);
    if language.is_empty() || language.len() > MAX_NATIVE_SESSION_LANGUAGE_BYTES {
        Err(NativeSessionMetadataError::InvalidLanguage)
    } else {
        Ok(language)
    }
}

fn validate_workspace(path: &Path) -> Result<(), NativeSessionMetadataError> {
    let bytes = path.as_os_str().as_bytes();
    if !path.is_absolute()
        || bytes.len() > MAX_NATIVE_SESSION_WORKSPACE_BYTES
        || bytes.contains(&0)
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
        || path
            .components()
            .collect::<PathBuf>()
            .as_os_str()
            .as_bytes()
            != bytes
    {
        Err(NativeSessionMetadataError::InvalidWorkspace)
    } else {
        Ok(())
    }
}

fn encode_workspace(path: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = path.as_os_str().as_bytes();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 15)]));
    }
    encoded
}

fn decode_workspace(encoded: &str) -> Result<PathBuf, NativeSessionMetadataError> {
    if encoded.len() > MAX_NATIVE_SESSION_WORKSPACE_BYTES * 2 || !encoded.len().is_multiple_of(2) {
        return Err(NativeSessionMetadataError::InvalidWorkspace);
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().chunks_exact(2) {
        bytes.push((decode_hex(pair[0])? << 4) | decode_hex(pair[1])?);
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    validate_workspace(&path)?;
    Ok(path)
}

fn decode_hex(byte: u8) -> Result<u8, NativeSessionMetadataError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(NativeSessionMetadataError::InvalidWorkspace),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> NativeSessionMetadata {
        NativeSessionMetadata::new(Path::new("/work"), 100, NativeSessionOrigin::Cli).unwrap()
    }

    #[test]
    fn legacy_metadata_does_not_invent_historical_facts() {
        let unrelated = BTreeMap::from([("other".to_owned(), json!({"workspace": "/pretend"}))]);
        let mut decoded = NativeSessionMetadata::from_metadata(&unrelated).unwrap();
        assert_eq!(decoded, NativeSessionMetadata::default());
        decoded.rename("new title", 300).unwrap();
        assert_eq!(decoded.created_at_ms(), None);
        assert_eq!(decoded.workspace(), None);
        assert_eq!(decoded.origin(), None);
        assert_eq!(decoded.updated_at_ms(), Some(300));
        assert_eq!(unrelated.len(), 1);
    }

    #[test]
    fn new_metadata_roundtrips_non_utf8_workspace_without_loss() {
        let path = PathBuf::from(OsString::from_vec(b"/work/\xff".to_vec()));
        let mut original =
            NativeSessionMetadata::new(&path, -20, NativeSessionOrigin::Cli).unwrap();
        original.rename("  title\t\r\n", -10).unwrap();
        original.set_language("zh-Hant", -5).unwrap();
        let decoded = NativeSessionMetadata::from_value(&original.to_value()).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.workspace(), Some(path.as_path()));
        assert_eq!(decoded.title(), Some("title"));
        assert_eq!(decoded.language(), Some("zh-Hant"));
        assert_eq!(decoded.created_at_ms(), Some(-20));
    }

    #[test]
    fn title_uses_utf8_bytes_and_exact_edge_trim_rules() {
        let mut value = metadata();
        value.rename(&"é".repeat(120), 101).unwrap();
        let previous = value.clone();
        for invalid in [
            "é".repeat(121),
            String::new(),
            " \t\r\n".to_owned(),
            "a\nb".to_owned(),
            "a\tb".to_owned(),
            "x\0".to_owned(),
            "x\x7f".to_owned(),
        ] {
            assert_eq!(
                value.rename(&invalid, 102),
                Err(NativeSessionMetadataError::InvalidTitle)
            );
            assert_eq!(value, previous);
        }
        value.rename("\u{a0}title\u{a0}", 102).unwrap();
        assert_eq!(value.title(), Some("\u{a0}title\u{a0}"));
    }

    #[test]
    fn failed_time_or_language_update_is_atomic() {
        let mut value = metadata();
        let previous = value.clone();
        assert_eq!(
            value.rename("valid", 99),
            Err(NativeSessionMetadataError::TimeRegression)
        );
        assert_eq!(value, previous);
        for language in ["", " \t\r\n", "this-language-is-too-long!"] {
            assert_eq!(
                value.set_language(language, 101),
                Err(NativeSessionMetadataError::InvalidLanguage)
            );
            assert_eq!(value, previous);
        }
        value.touch(120).unwrap();
        assert_eq!(
            value.touch(119),
            Err(NativeSessionMetadataError::TimeRegression)
        );
        assert_eq!(value.updated_at_ms(), Some(120));
    }

    #[test]
    fn language_uses_pinned_trimmed_byte_limit_without_inventing_tag_grammar() {
        let mut value = metadata();
        value.set_language(" \ten_US\r\n", 101).unwrap();
        assert_eq!(value.language(), Some("en_US"));
        value.set_language(&"é".repeat(12), 102).unwrap();
        let previous = value.clone();
        assert_eq!(
            value.set_language(&"é".repeat(13), 103),
            Err(NativeSessionMetadataError::InvalidLanguage)
        );
        assert_eq!(value, previous);
        value.set_language("un\0known", 103).unwrap();
        assert_eq!(value.language(), Some("un\0known"));
        assert_eq!(
            NativeSessionMetadata::from_value(&value.to_value()).unwrap(),
            value
        );
    }

    #[test]
    fn stored_fields_are_strict_without_recursive_decoding() {
        for invalid in [
            Value::Null,
            json!([]),
            json!({}),
            json!({"schema_version": "1"}),
            json!({"schema_version": 1, "unexpected": {"deep": []}}),
            json!({"schema_version": 1, "title": {"nested": []}}),
            json!({"schema_version": 1, "created_at_ms": 0.5}),
            json!({"schema_version": 1, "updated_at_ms": u64::MAX}),
            json!({"schema_version": 1, "origin": "guessed"}),
            json!({"schema_version": 1, "title": " unnormalized "}),
            json!({"schema_version": 1, "created_at_ms": 10, "updated_at_ms": 9}),
        ] {
            assert!(NativeSessionMetadata::from_value(&invalid).is_err());
        }
        assert_eq!(
            NativeSessionMetadata::from_value(&json!({"schema_version": 2})),
            Err(NativeSessionMetadataError::UnsupportedVersion)
        );
        assert_eq!(
            NativeSessionMetadata::from_value(&json!({"schema_version": 1})).unwrap(),
            NativeSessionMetadata::default()
        );
    }

    #[test]
    fn workspace_shape_is_normalized_and_bounded_not_canonicalized() {
        for path in [
            "relative",
            "",
            "/work/../other",
            "/work/./other",
            "/work/",
            "//work",
            "/nul\0",
        ] {
            assert_eq!(
                NativeSessionMetadata::new(Path::new(path), 0, NativeSessionOrigin::Cli),
                Err(NativeSessionMetadataError::InvalidWorkspace)
            );
        }
        assert!(
            NativeSessionMetadata::new(
                Path::new("/does-not-need-to-exist"),
                0,
                NativeSessionOrigin::Cli
            )
            .is_ok()
        );
        assert!(NativeSessionMetadata::new(Path::new("/"), 0, NativeSessionOrigin::Cli).is_ok());
        let boundary = format!("/{}", "x".repeat(MAX_NATIVE_SESSION_WORKSPACE_BYTES - 1));
        assert!(
            NativeSessionMetadata::new(Path::new(&boundary), 0, NativeSessionOrigin::Cli).is_ok()
        );
        assert!(
            NativeSessionMetadata::new(Path::new(&(boundary + "x")), 0, NativeSessionOrigin::Cli)
                .is_err()
        );
    }

    #[test]
    fn workspace_encoding_rejects_noncanonical_and_invalid_forms() {
        for encoded in ["2F", "2", "zz", "", "2f00", "61", "2f2f61"] {
            assert!(decode_workspace(encoded).is_err());
        }
        assert_eq!(decode_workspace("2f776f726b").unwrap(), Path::new("/work"));
    }

    #[test]
    fn debug_and_errors_do_not_reflect_user_fields() {
        let mut value = metadata();
        value.rename("SECRET-TITLE", 100).unwrap();
        let debug = format!("{value:?}");
        assert!(!debug.contains("SECRET"));
        assert!(!debug.contains("/work"));
        assert_eq!(
            NativeSessionMetadataError::InvalidTitle.to_string(),
            "native session title is invalid"
        );
    }
}
