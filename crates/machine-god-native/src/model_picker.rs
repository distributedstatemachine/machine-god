//! Pure native model-menu state. Catalog entries are observations, not authority.
use crate::{NativeModelCatalog, NativeModelCatalogEntry, model_selection::Query};
use std::{fmt, sync::Arc};

pub const MAX_NATIVE_MODEL_PICKER_QUERY_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeModelPickerError {
    InvalidQuery,
    InvalidCursor,
}
impl fmt::Display for NativeModelPickerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidQuery => "model picker query is invalid or too long",
            Self::InvalidCursor => "model picker cursor is not a UTF-8 boundary",
        })
    }
}
impl std::error::Error for NativeModelPickerError {}

/// Bounded immutable projection borrowed from one native-owned menu.
pub struct NativeModelPickerView<'a> {
    pub query: &'a str,
    pub cursor: usize,
    /// Index in the filtered, ranked rows, not the underlying source catalog.
    pub selected: Option<usize>,
    catalog: Option<&'a NativeModelCatalog>,
    matches: &'a [usize],
}
impl NativeModelPickerView<'_> {
    /// Exact validated catalog spellings and advertised capabilities.
    #[must_use]
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &NativeModelCatalogEntry> {
        // An unloaded picker has no matches; both slices belong to the same
        // immutable projection and cannot be independently replaced by callers.
        let entries = self
            .catalog
            .map(NativeModelCatalog::entries)
            .unwrap_or_default();
        self.matches.iter().map(move |index| &entries[*index])
    }
}
impl fmt::Debug for NativeModelPickerView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeModelPickerView")
            .field("rows", &self.matches.len())
            .finish_non_exhaustive()
    }
}

/// Shares one bounded catalog, retains a byte-bounded query and at most 512
/// source indices. No fetching, timers, model calls or configuration effects.
/// The interactive owner must still check its frame and original child before
/// using a selected model in a durable configure command.
pub struct NativeModelPicker {
    catalog: Option<Arc<NativeModelCatalog>>,
    query: String,
    cursor: usize,
    matches: Vec<usize>,
    selected: Option<usize>,
}
impl fmt::Debug for NativeModelPicker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.view(), f)
    }
}
impl NativeModelPicker {
    #[must_use]
    pub fn new(catalog: Arc<NativeModelCatalog>) -> Self {
        let mut picker = Self::unloaded();
        picker.replace_catalog(catalog);
        picker
    }
    pub(crate) fn unloaded() -> Self {
        Self {
            catalog: None,
            query: String::new(),
            cursor: 0,
            matches: Vec::new(),
            selected: None,
        }
    }
    #[must_use]
    pub fn view(&self) -> NativeModelPickerView<'_> {
        NativeModelPickerView {
            query: &self.query,
            cursor: self.cursor,
            selected: self.selected,
            catalog: self.catalog.as_deref(),
            matches: &self.matches,
        }
    }
    #[must_use]
    pub fn selected(&self) -> Option<&NativeModelCatalogEntry> {
        let catalog = self.catalog.as_ref()?;
        self.matches
            .get(self.selected?)
            .map(|index| &catalog.entries()[*index])
    }
    /// Replaces query/cursor atomically; cursor-only movement preserves selection.
    /// # Errors
    /// Oversized/NUL queries and non-UTF-8-boundary cursors preserve prior state.
    pub fn edit(&mut self, query: &str, cursor: usize) -> Result<(), NativeModelPickerError> {
        if query.len() > MAX_NATIVE_MODEL_PICKER_QUERY_BYTES || query.contains('\0') {
            return Err(NativeModelPickerError::InvalidQuery);
        }
        if !query.is_char_boundary(cursor) {
            return Err(NativeModelPickerError::InvalidCursor);
        }
        if self.query != query {
            self.query.clear();
            self.query.push_str(query);
            self.filter(None);
        }
        self.cursor = cursor;
        Ok(())
    }
    /// Replaces a catalog observation while retaining query and exact selected ID
    /// when it still matches. An absent ID selects the first remaining match.
    pub fn replace_catalog(&mut self, catalog: Arc<NativeModelCatalog>) {
        let selected = self.selected().map(|entry| entry.model().id().to_owned());
        self.catalog = Some(catalog);
        self.filter(selected.as_deref());
    }
    pub fn move_selection(&mut self, previous: bool) {
        let Some(selected) = self.selected else {
            return;
        };
        self.selected = Some(if previous {
            selected.saturating_sub(1)
        } else {
            (selected + 1).min(self.matches.len() - 1)
        });
    }
    fn filter(&mut self, retained: Option<&str>) {
        let Some(catalog) = &self.catalog else {
            self.matches.clear();
            self.selected = None;
            return;
        };
        let query = Query::new(self.query.as_bytes());
        let mut ranked: Vec<_> = catalog
            .entries()
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let id = entry.model().id();
                let score = if self.query.is_empty() {
                    1
                } else if id.eq_ignore_ascii_case(&self.query) {
                    usize::MAX
                } else {
                    query.score(id.as_bytes())
                };
                (score != 0).then_some((index, score))
            })
            .collect();
        // Explicit source-index tie break preserves catalog order without sort
        // scratch allocation, even when equally scored IDs change on refresh.
        ranked.sort_unstable_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        self.matches.clear();
        self.matches
            .extend(ranked.into_iter().map(|(index, _)| index));
        self.selected = retained
            .and_then(|id| {
                self.matches
                    .iter()
                    .position(|index| catalog.entries()[*index].model().id() == id)
            })
            .or_else(|| (!self.matches.is_empty()).then_some(0));
    }
}

#[cfg(test)]
mod tests;
