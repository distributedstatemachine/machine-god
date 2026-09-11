use super::{
    Menu, NativeSkillBinding, NativeSkillFrameIdentity, NativeSkillInlineQuery, NativeSkillPicker,
    NativeSkillPickerError as Error, NativeSkillPickerInsertion, NativeSkillPickerMode as Mode,
    NativeSkillPickerView, NativeSkillSnapshot, Result, edit,
};
use crate::skills_catalog::{MAX_NATIVE_SKILL_QUERY_BYTES, MAX_NATIVE_SKILL_QUERY_ROWS};
use std::{ops::Range, sync::Arc};

impl NativeSkillPicker {
    /// A whitespace-delimited `$query` prefix ending exactly at the cursor.
    /// The suffix is never consumed. Existing exact bindings are not rematched;
    /// a spaced/Unicode advertised name is inserted intact upon selection.
    #[must_use]
    pub fn inline_query(&self) -> Option<NativeSkillInlineQuery<'_>> {
        if self
            .bindings
            .iter()
            .any(|binding| binding.span.start < self.cursor && self.cursor <= binding.span.end)
        {
            return None;
        }
        let bytes = self.draft.as_bytes();
        let minimum = self.cursor.saturating_sub(MAX_NATIVE_SKILL_QUERY_BYTES + 1);
        for start in (minimum..self.cursor).rev() {
            if bytes[start] == b'$' {
                if start != 0 && !edit::whitespace(bytes[start - 1]) {
                    return None;
                }
                return Some(NativeSkillInlineQuery {
                    span: start..self.cursor,
                    query: &self.draft[start + 1..self.cursor],
                });
            }
            if edit::whitespace(bytes[start]) {
                return None;
            }
        }
        None
    }

    /// # Errors
    /// Rejects drafts without an eligible bounded inline query.
    pub fn open_inline(&mut self, snapshot: Arc<NativeSkillSnapshot>) -> Result<()> {
        let query = self.inline_query().ok_or(Error::NoInlineQuery)?;
        self.menu = Some(Menu::new(snapshot, Mode::Inline, query.query, query.span)?);
        Ok(())
    }

    /// Opens the slash-command menu without clearing or rewriting the draft.
    /// # Errors
    /// Rejects oversized queries without disturbing an existing menu.
    pub fn open_menu(&mut self, snapshot: Arc<NativeSkillSnapshot>, query: &str) -> Result<()> {
        self.menu = Some(Menu::new(
            snapshot,
            Mode::Menu,
            query,
            self.cursor..self.cursor,
        )?);
        Ok(())
    }

    /// Changes only the slash menu query. Inline queries follow actual edits.
    /// # Errors
    /// Rejects a closed/inline menu, oversized queries or exhausted revisions.
    pub fn query_menu(&mut self, query: &str) -> Result<()> {
        if query.len() > MAX_NATIVE_SKILL_QUERY_BYTES {
            return Err(Error::InvalidQuery);
        }
        let menu = self.menu.as_mut().ok_or(Error::NotOpen)?;
        if menu.mode != Mode::Menu {
            return Err(Error::InvalidQuery);
        }
        menu.change()?;
        menu.query.clear();
        menu.query.push_str(query);
        menu.filter();
        Ok(())
    }

    /// Moves through every matching catalog entry, not just the visible window.
    /// # Errors
    /// Rejects a closed menu or exhausted frame revisions.
    pub fn move_selection(&mut self, forward: bool) -> Result<()> {
        let menu = self.menu.as_mut().ok_or(Error::NotOpen)?;
        menu.change()?;
        if !menu.matches.is_empty() {
            menu.selected = if forward {
                (menu.selected + 1) % menu.matches.len()
            } else if menu.selected == 0 {
                menu.matches.len() - 1
            } else {
                menu.selected - 1
            };
        }
        Ok(())
    }

    /// Focuses an exact observed selection, for example a command-service show
    /// result, without matching by name or choosing among duplicate locations.
    /// # Errors
    /// Rejects closed menus, foreign/stale/filtered selections and exhausted
    /// revisions without changing the previous frame or acknowledgement.
    pub fn focus(&mut self, selection: &super::NativeSkillSelection) -> Result<()> {
        let menu = self.menu.as_mut().ok_or(Error::NotOpen)?;
        let selected = menu
            .matches
            .iter()
            .position(|index| menu.snapshot.entries()[*index].selection_ref() == selection)
            .ok_or(Error::NoSelection)?;
        menu.change()?;
        menu.selected = selected;
        Ok(())
    }

    /// Requires a new acknowledged frame after a host presentation change such
    /// as resize, without moving the selected row or changing the query/draft.
    /// # Errors
    /// Rejects a closed menu or exhausted frame revision.
    pub fn invalidate_frame(&mut self) -> Result<()> {
        self.menu.as_mut().ok_or(Error::NotOpen)?.change()
    }

    #[must_use]
    pub fn view(&self) -> Option<NativeSkillPickerView<'_>> {
        let menu = self.menu.as_ref()?;
        let start = menu.selected / MAX_NATIVE_SKILL_QUERY_ROWS * MAX_NATIVE_SKILL_QUERY_ROWS;
        Some(NativeSkillPickerView {
            identity: menu.identity(self),
            mode: menu.mode,
            query: &menu.query,
            rows: menu
                .matches
                .iter()
                .skip(start)
                .take(MAX_NATIVE_SKILL_QUERY_ROWS)
                .map(|index| &menu.snapshot.entries()[*index])
                .collect(),
            selected: (!menu.matches.is_empty()).then_some(menu.selected),
            window_start: start,
            total_matches: menu.matches.len(),
            discovery_incomplete: !menu.snapshot.complete(),
        })
    }

    /// Acknowledge only after the exact frame has been successfully rendered.
    /// # Errors
    /// Rejects closed menus and frames from older revisions/owners/catalogs.
    pub fn acknowledge(&mut self, identity: &NativeSkillFrameIdentity) -> Result<()> {
        self.check_frame(identity)?;
        if let Some(menu) = self.menu.as_mut() {
            menu.acknowledged = Some(identity.clone());
        }
        Ok(())
    }

    /// Applies an acknowledged exact row choice. Duplicate names remain distinct
    /// rows and retain their own catalog authority. No name-based resolution.
    /// # Errors
    /// Rejects stale/unacknowledged frames, empty results and aggregate
    /// prompt/binding limits before changing the draft or cloning a selection.
    pub fn choose(
        &mut self,
        identity: &NativeSkillFrameIdentity,
    ) -> Result<NativeSkillPickerInsertion> {
        self.check_frame(identity)?;
        let menu = self.menu.as_ref().ok_or(Error::NotOpen)?;
        if menu.acknowledged.as_ref() != Some(identity) {
            return Err(Error::FrameNotAcknowledged);
        }
        let index = *menu.matches.get(menu.selected).ok_or(Error::NoSelection)?;
        let selection = menu.snapshot.entries()[index].selection_ref();
        let range = menu.range.clone();
        let leading = menu.mode == Mode::Menu
            && range.start != 0
            && !edit::whitespace(self.draft.as_bytes()[range.start - 1]);
        // Pinned completion behavior: a separator at EOF or before a word-like
        // suffix; existing whitespace and punctuation are left untouched.
        let trailing = self.draft[range.end..]
            .chars()
            .next()
            .is_none_or(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch >= '\u{c0}');
        let mut inserted = String::with_capacity(selection.name().len() + 3);
        if leading {
            inserted.push(' ');
        }
        inserted.push('$');
        inserted.push_str(selection.name());
        let span = range.start + usize::from(leading)..range.start + inserted.len();
        if trailing {
            inserted.push(' ');
        }
        let cursor_after = range.start + inserted.len();
        self.validate_edit(&range, &inserted, cursor_after)?;
        let revision = self.next_revision()?;
        let mut count = 1;
        let mut bytes = selection.retained_bytes();
        for binding in &self.bindings {
            if edit::surviving_span(&self.draft, &binding.span, &range, &inserted).is_some() {
                count += 1;
                bytes = bytes
                    .checked_add(binding.selection.retained_bytes())
                    .ok_or(Error::SelectionBytesExceeded)?;
            }
        }
        if count > super::MAX_NATIVE_SKILL_INVOCATION_SELECTIONS {
            return Err(Error::TooManySelections);
        }
        if bytes > super::MAX_NATIVE_SKILL_INVOCATION_SELECTION_BYTES {
            return Err(Error::SelectionBytesExceeded);
        }
        let binding = NativeSkillBinding {
            span,
            selection: selection.clone(),
        };
        let expected_draft = self.identity.clone();
        self.commit_edit(&range, &inserted, cursor_after);
        self.bindings.push(binding.clone());
        self.identity.revision = revision;
        self.menu = None;
        Ok(NativeSkillPickerInsertion {
            expected_draft,
            updated_draft: self.identity.clone(),
            range,
            inserted,
            cursor_after,
            binding,
        })
    }

    fn check_frame(&self, identity: &NativeSkillFrameIdentity) -> Result<()> {
        let menu = self.menu.as_ref().ok_or(Error::NotOpen)?;
        if menu.identity(self) != *identity {
            return Err(Error::StaleFrame);
        }
        Ok(())
    }

    pub(super) fn refresh_after_edit(&mut self) {
        let inline = self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.mode == Mode::Inline);
        if !inline {
            self.menu = None;
            return;
        }
        let Some(query) = self.inline_query() else {
            self.menu = None;
            return;
        };
        let range = query.span;
        let query = query.query.to_owned();
        if let Some(menu) = self.menu.as_mut() {
            if menu.change().is_err() {
                self.menu = None;
                return;
            }
            menu.range = range;
            menu.query = query;
            menu.filter();
        }
    }
}

impl Menu {
    fn new(
        snapshot: Arc<NativeSkillSnapshot>,
        mode: Mode,
        query: &str,
        range: Range<usize>,
    ) -> Result<Self> {
        if query.len() > MAX_NATIVE_SKILL_QUERY_BYTES {
            return Err(Error::InvalidQuery);
        }
        let mut menu = Self {
            owner: Arc::new(()),
            revision: 0,
            snapshot,
            mode,
            query: query.to_owned(),
            range,
            matches: Vec::new(),
            selected: 0,
            acknowledged: None,
        };
        menu.filter();
        Ok(menu)
    }

    fn identity(&self, picker: &NativeSkillPicker) -> NativeSkillFrameIdentity {
        NativeSkillFrameIdentity {
            draft: picker.identity.clone(),
            menu: self.owner.clone(),
            revision: self.revision,
            catalog_generation: *self.snapshot.generation(),
        }
    }

    fn change(&mut self) -> Result<()> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(Error::RevisionExhausted)?;
        self.acknowledged = None;
        Ok(())
    }

    fn filter(&mut self) {
        self.matches.clear();
        let query = self.query.trim().as_bytes();
        // Discovery caps the complete catalog at 1024 candidates. Keep only
        // indices, never clone per-entry metadata or stop at the first window.
        self.matches
            .extend(
                self.snapshot
                    .entries()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, entry)| {
                        (contains(entry.metadata.name.as_bytes(), query)
                            || contains(entry.metadata.description.as_bytes(), query)
                            || entry
                                .location()
                                .to_str()
                                .is_some_and(|path| contains(path.as_bytes(), query)))
                        .then_some(index)
                    }),
            );
        self.selected = 0;
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}
