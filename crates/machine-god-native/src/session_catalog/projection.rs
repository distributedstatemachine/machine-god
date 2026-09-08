use crate::NativeSessionMetadata;
use machine_god_core::{
    ContentBlock, Role, SessionId, SessionIncarnationId, SessionRecord, SessionRevision,
    SessionStoreError, SessionStoreErrorKind,
};
use std::{cmp::Ordering, fmt};

pub const MAX_NATIVE_SESSION_PREVIEW_BYTES: usize = 240;

/// One validated persisted observation. Metadata is authoritative; preview is a
/// separately bounded untrusted excerpt, never a title or proof of user provenance.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeSessionCatalogEntry {
    id: SessionId,
    incarnation: SessionIncarnationId,
    revision: SessionRevision,
    message_count: usize,
    history_len: usize,
    metadata: NativeSessionMetadata,
    preview: Option<String>,
    preview_truncated: bool,
}
impl NativeSessionCatalogEntry {
    pub(super) fn project(record: &SessionRecord) -> Result<Self, SessionStoreError> {
        let metadata = NativeSessionMetadata::from_metadata(&record.metadata).map_err(|_| {
            SessionStoreError::new(
                SessionStoreErrorKind::Corrupt,
                "session_catalog_corrupt",
                "native session record is corrupt",
                false,
            )
        })?;
        let (preview, preview_truncated) = preview(record);
        Ok(Self {
            id: record.id.clone(),
            incarnation: record.incarnation_id.clone(),
            revision: record.revision,
            message_count: record.messages.len(),
            history_len: record
                .messages
                .iter()
                .filter(|message| message.role == Role::User)
                .count(),
            metadata,
            preview,
            preview_truncated,
        })
    }
    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    #[must_use]
    pub fn incarnation_id(&self) -> &SessionIncarnationId {
        &self.incarnation
    }
    #[must_use]
    pub const fn revision(&self) -> SessionRevision {
        self.revision
    }
    #[must_use]
    pub const fn message_count(&self) -> usize {
        self.message_count
    }
    /// Number of canonical user-message groups, excluding assistant/tool rounds.
    #[must_use]
    pub const fn history_len(&self) -> usize {
        self.history_len
    }
    #[must_use]
    pub const fn native_metadata(&self) -> &NativeSessionMetadata {
        &self.metadata
    }
    #[must_use]
    pub fn preview(&self) -> Option<&str> {
        self.preview.as_deref()
    }
    #[must_use]
    pub const fn preview_truncated(&self) -> bool {
        self.preview_truncated
    }
}
impl fmt::Debug for NativeSessionCatalogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSessionCatalogEntry")
            .field("revision", &self.revision)
            .field("message_count", &self.message_count)
            .finish_non_exhaustive()
    }
}
pub(super) fn newest_first(
    a: &NativeSessionCatalogEntry,
    b: &NativeSessionCatalogEntry,
) -> Ordering {
    b.metadata
        .updated_at_ms()
        .cmp(&a.metadata.updated_at_ms())
        .then_with(|| b.id.cmp(&a.id))
}
fn preview(record: &SessionRecord) -> (Option<String>, bool) {
    for message in record
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
    {
        for block in &message.content {
            if let ContentBlock::Text { text } = block {
                let trimmed = text.trim_matches([' ', '\t', '\r', '\n']);
                if !trimmed.is_empty() && (!trimmed.starts_with('/') || trimmed.contains('\n')) {
                    return bounded_preview(text);
                }
            }
        }
    }
    (None, false)
}
fn bounded_preview(text: &str) -> (Option<String>, bool) {
    let mut out = String::new();
    for (lines, line) in text
        .split('\n')
        .map(|line| line.trim_matches([' ', '\t', '\r']))
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        if lines == 2 || out.len() == MAX_NATIVE_SESSION_PREVIEW_BYTES {
            return (Some(out), true);
        }
        if !out.is_empty() {
            out.push('\n');
        }
        let mut take = line.len().min(MAX_NATIVE_SESSION_PREVIEW_BYTES - out.len());
        while !line.is_char_boundary(take) {
            take -= 1;
        }
        out.push_str(&line[..take]);
        if take < line.len() {
            return (Some(out), true);
        }
    }
    ((!out.is_empty()).then_some(out), false)
}
