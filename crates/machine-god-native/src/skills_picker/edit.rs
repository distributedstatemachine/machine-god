use super::{NativeSkillDraftIdentity, NativeSkillPicker, NativeSkillPickerError as Error, Result};
use std::ops::Range;

pub(super) fn validate_draft(draft: &str, cursor: usize) -> Result<()> {
    if draft.len() > super::MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES {
        return Err(Error::PromptTooLong);
    }
    if !draft.is_char_boundary(cursor) {
        return Err(Error::InvalidEdit);
    }
    Ok(())
}

pub(super) fn whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

impl NativeSkillPicker {
    /// Applies an actual edit receipt, not a text diff. Overlaps invalidate a
    /// binding; disjoint edits shift it. At a changed adjacent boundary, only
    /// ASCII space/tab/CR/LF (or the draft edge) preserves binding authority.
    /// Thus deleting one of two identical tokens cannot transfer its binding.
    /// # Errors
    /// Rejects stale identities, invalid UTF-8 ranges/cursors, prompt limits or
    /// exhausted revisions atomically, retaining the previous state.
    pub fn apply_edit(
        &mut self,
        expected: &NativeSkillDraftIdentity,
        range: Range<usize>,
        inserted: &str,
        cursor_after: usize,
    ) -> Result<()> {
        self.check_draft(expected)?;
        let revision = self.next_revision()?;
        self.validate_edit(&range, inserted, cursor_after)?;
        self.commit_edit(&range, inserted, cursor_after);
        self.identity.revision = revision;
        self.refresh_after_edit();
        Ok(())
    }

    /// # Errors
    /// Rejects stale identities, invalid cursors or exhausted revisions.
    pub fn move_cursor(
        &mut self,
        expected: &NativeSkillDraftIdentity,
        cursor: usize,
    ) -> Result<()> {
        self.check_draft(expected)?;
        validate_draft(&self.draft, cursor)?;
        let revision = self.next_revision()?;
        self.cursor = cursor;
        self.identity.revision = revision;
        self.refresh_after_edit();
        Ok(())
    }

    pub(super) fn validate_edit(
        &self,
        range: &Range<usize>,
        inserted: &str,
        cursor: usize,
    ) -> Result<()> {
        if range.start > range.end
            || !self.draft.is_char_boundary(range.start)
            || !self.draft.is_char_boundary(range.end)
        {
            return Err(Error::InvalidEdit);
        }
        let length = self.draft.len() - range.len();
        let length = length
            .checked_add(inserted.len())
            .filter(|length| *length <= super::MAX_NATIVE_SKILL_INVOCATION_PROMPT_BYTES)
            .ok_or(Error::PromptTooLong)?;
        if cursor > length {
            return Err(Error::InvalidEdit);
        }
        let valid = if cursor <= range.start {
            self.draft.is_char_boundary(cursor)
        } else if cursor <= range.start + inserted.len() {
            inserted.is_char_boundary(cursor - range.start)
        } else {
            self.draft
                .is_char_boundary(cursor - inserted.len() + range.len())
        };
        if !valid {
            return Err(Error::InvalidEdit);
        }
        Ok(())
    }

    pub(super) fn commit_edit(&mut self, range: &Range<usize>, inserted: &str, cursor: usize) {
        self.bindings.retain_mut(|binding| {
            if let Some(span) = surviving_span(&self.draft, &binding.span, range, inserted) {
                binding.span = span;
                true
            } else {
                false
            }
        });
        self.draft.replace_range(range.clone(), inserted);
        self.cursor = cursor;
    }
}

pub(super) fn surviving_span(
    draft: &str,
    span: &Range<usize>,
    edit: &Range<usize>,
    inserted: &str,
) -> Option<Range<usize>> {
    if edit.is_empty() && inserted.is_empty() {
        return Some(span.clone());
    }
    if edit.end <= span.start {
        if edit.end == span.start {
            let previous = inserted
                .as_bytes()
                .last()
                .copied()
                .or_else(|| draft.as_bytes().get(edit.start.wrapping_sub(1)).copied());
            if previous.is_some_and(|byte| !whitespace(byte)) {
                return None;
            }
        }
        Some(span.start - edit.len() + inserted.len()..span.end - edit.len() + inserted.len())
    } else if edit.start >= span.end {
        if edit.start == span.end {
            let next = inserted
                .as_bytes()
                .first()
                .copied()
                .or_else(|| draft.as_bytes().get(edit.end).copied());
            if next.is_some_and(|byte| !whitespace(byte)) {
                return None;
            }
        }
        Some(span.clone())
    } else {
        None
    }
}
