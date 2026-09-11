//! Effect-free skill completion and exact per-draft bindings.
//!
//! The host supplies actual UTF-8 edit receipts, never a diff guessed from two
//! strings. It renders/sanitizes borrowed rows, acknowledges the exact rendered
//! frame, and applies a chosen edit atomically to its matching composer mirror.
//! A synchronization failure requires dropping/resetting this state. No method
//! discovers, opens or materializes a skill, or infers authority from text.

use std::{fmt, ops::Range, sync::Arc};

use crate::{
    skills_catalog::{NativeSkillEntry, NativeSkillSelection, NativeSkillSnapshot},
    skills_invocation::{
        MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES, MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES,
        MAX_NATIVE_SKILL_INVOCATION_SELECTIONS,
    },
};

#[path = "skills_picker/edit.rs"]
mod edit;
#[path = "skills_picker/menu.rs"]
mod menu;
#[path = "skills_picker/query.rs"]
mod query;
#[cfg(test)]
#[path = "skills_picker/tests.rs"]
mod tests;

type Result<T> = std::result::Result<T, NativeSkillPickerError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillPickerError {
    PromptTooLong,
    InvalidEdit,
    StaleDraft,
    InvalidQuery,
    NoInlineQuery,
    NotOpen,
    StaleFrame,
    FrameNotAcknowledged,
    NoSelection,
    TooManySelections,
    SelectionBytesExceeded,
    RevisionExhausted,
}

impl fmt::Display for NativeSkillPickerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skill picker: {self:?}")
    }
}
impl std::error::Error for NativeSkillPickerError {}

/// Opaque owner identity plus revision; even an identical reset is a new owner.
#[derive(Clone)]
pub struct NativeSkillDraftIdentity {
    owner: Arc<()>,
    revision: u64,
}

impl PartialEq for NativeSkillDraftIdentity {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner) && self.revision == other.revision
    }
}
impl Eq for NativeSkillDraftIdentity {}
impl fmt::Debug for NativeSkillDraftIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSkillDraftIdentity(<opaque>)")
    }
}

#[derive(Clone)]
pub struct NativeSkillFrameIdentity {
    draft: NativeSkillDraftIdentity,
    menu: Arc<()>,
    revision: u64,
    catalog_generation: [u8; 32],
}

impl PartialEq for NativeSkillFrameIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.draft == other.draft
            && Arc::ptr_eq(&self.menu, &other.menu)
            && self.revision == other.revision
            && self.catalog_generation == other.catalog_generation
    }
}
impl Eq for NativeSkillFrameIdentity {}
impl fmt::Debug for NativeSkillFrameIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeSkillFrameIdentity(<opaque>)")
    }
}

#[derive(Clone, Debug)]
pub struct NativeSkillBinding {
    span: Range<usize>,
    selection: NativeSkillSelection,
}

impl NativeSkillBinding {
    #[must_use]
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }
    #[must_use]
    pub const fn selection(&self) -> &NativeSkillSelection {
        &self.selection
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillPickerMode {
    Inline,
    Menu,
}

pub struct NativeSkillInlineQuery<'a> {
    pub span: Range<usize>,
    pub query: &'a str,
}

impl fmt::Debug for NativeSkillInlineQuery<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillInlineQuery")
            .field("span", &self.span)
            .finish_non_exhaustive()
    }
}

/// Borrowed, unsanitized display data. `selected` indexes all matches, not rows;
/// `window_start` translates it into the at-most-128 visible rows.
pub struct NativeSkillPickerView<'a> {
    pub identity: NativeSkillFrameIdentity,
    pub mode: NativeSkillPickerMode,
    pub query: &'a str,
    pub rows: Vec<&'a NativeSkillEntry>,
    pub selected: Option<usize>,
    pub window_start: usize,
    pub total_matches: usize,
    pub discovery_incomplete: bool,
}

impl fmt::Debug for NativeSkillPickerView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillPickerView")
            .field("mode", &self.mode)
            .field("total_matches", &self.total_matches)
            .field("discovery_incomplete", &self.discovery_incomplete)
            .finish_non_exhaustive()
    }
}

/// Already applied to native state. Apply this exact edit to the host composer
/// only if its previous draft agrees; do not echo it back through `apply_edit`.
pub struct NativeSkillPickerInsertion {
    pub expected_draft: NativeSkillDraftIdentity,
    pub updated_draft: NativeSkillDraftIdentity,
    pub range: Range<usize>,
    pub inserted: String,
    pub cursor_after: usize,
    pub binding: NativeSkillBinding,
}

impl fmt::Debug for NativeSkillPickerInsertion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillPickerInsertion")
            .field("range", &self.range)
            .field("cursor_after", &self.cursor_after)
            .finish_non_exhaustive()
    }
}

struct Menu {
    owner: Arc<()>,
    revision: u64,
    snapshot: Arc<NativeSkillSnapshot>,
    mode: NativeSkillPickerMode,
    query: String,
    range: Range<usize>,
    matches: Vec<usize>,
    selected: usize,
    acknowledged: Option<NativeSkillFrameIdentity>,
}

pub struct NativeSkillPicker {
    identity: NativeSkillDraftIdentity,
    draft: String,
    cursor: usize,
    bindings: Vec<NativeSkillBinding>,
    menu: Option<Menu>,
}

impl fmt::Debug for NativeSkillPicker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillPicker")
            .field("draft_bytes", &self.draft.len())
            .field("bindings", &self.bindings.len())
            .field("open", &self.menu.is_some())
            .finish_non_exhaustive()
    }
}

impl NativeSkillPicker {
    /// # Errors
    /// Rejects oversized text or a cursor outside UTF-8 boundaries.
    pub fn new(draft: String, cursor: usize) -> Result<Self> {
        edit::validate_draft(&draft, cursor)?;
        Ok(Self {
            identity: NativeSkillDraftIdentity {
                owner: Arc::new(()),
                revision: 0,
            },
            draft,
            cursor,
            bindings: Vec::new(),
            menu: None,
        })
    }

    #[must_use]
    pub fn draft(&self) -> &str {
        &self.draft
    }
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }
    #[must_use]
    pub const fn draft_identity(&self) -> &NativeSkillDraftIdentity {
        &self.identity
    }
    #[must_use]
    pub fn bindings(&self) -> &[NativeSkillBinding] {
        &self.bindings
    }

    /// Fresh prompt/handoff identity, even for identical text. Invalid inputs
    /// leave the old state intact; runtime synchronization failure should drop
    /// this object instead of relying on an invalid reset.
    /// # Errors
    /// Rejects oversized text or an invalid cursor before changing state.
    pub fn reset(&mut self, draft: String, cursor: usize) -> Result<()> {
        *self = Self::new(draft, cursor)?;
        Ok(())
    }

    /// Closing/Escape preserves draft and bindings, but invalidates all frames.
    pub fn close(&mut self) {
        self.menu = None;
    }

    /// Exact bound choices for this prompt. No name/path matching occurs here;
    /// the invocation planner must still validate their catalog revisions.
    /// # Errors
    /// Rejects an earlier owner/revision or any inconsistent bound token.
    pub fn selections(
        &self,
        expected: &NativeSkillDraftIdentity,
    ) -> Result<Vec<NativeSkillSelection>> {
        self.check_draft(expected)?;
        for binding in &self.bindings {
            let token = self
                .draft
                .get(binding.span.clone())
                .ok_or(NativeSkillPickerError::StaleDraft)?;
            if token.strip_prefix('$') != Some(binding.selection.name()) {
                return Err(NativeSkillPickerError::StaleDraft);
            }
        }
        Ok(self
            .bindings
            .iter()
            .map(|binding| binding.selection.clone())
            .collect())
    }

    fn check_draft(&self, expected: &NativeSkillDraftIdentity) -> Result<()> {
        if &self.identity != expected {
            return Err(NativeSkillPickerError::StaleDraft);
        }
        Ok(())
    }

    fn next_revision(&self) -> Result<u64> {
        self.identity
            .revision
            .checked_add(1)
            .ok_or(NativeSkillPickerError::RevisionExhausted)
    }
}
