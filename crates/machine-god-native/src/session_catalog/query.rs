use super::{
    MAX_LIST_SESSIONS, NativeSessionCatalogEntry, NativeSessionCatalogError as Error,
    NativeSessionCatalogErrorKind as Kind,
};
use std::{
    fmt,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

pub const MAX_NATIVE_SESSION_CATALOG_QUERY_BYTES: usize = 1024;

/// Pure bounded filtering data. Workspace association never grants filesystem authority.
#[derive(Clone)]
pub struct NativeSessionCatalogQuery {
    limit: usize,
    workspace: Option<PathBuf>,
    search: String,
    updated_since: Option<i64>,
}
impl Default for NativeSessionCatalogQuery {
    fn default() -> Self {
        Self {
            limit: MAX_LIST_SESSIONS,
            workspace: None,
            search: String::new(),
            updated_since: None,
        }
    }
}
impl NativeSessionCatalogQuery {
    /// # Errors
    /// Rejects a zero or greater-than-100 presentation limit.
    pub fn new(limit: usize) -> Result<Self, Error> {
        if !(1..=MAX_LIST_SESSIONS).contains(&limit) {
            return Err(Error::new(Kind::InvalidQuery));
        }
        Ok(Self {
            limit,
            ..Self::default()
        })
    }
    /// # Errors
    /// Rejects non-normalized absolute Unix paths, NULs, or more than 4096 bytes.
    pub fn with_workspace(mut self, workspace: &Path) -> Result<Self, Error> {
        let bytes = workspace.as_os_str().as_bytes();
        if !workspace.is_absolute()
            || bytes.len() > crate::MAX_NATIVE_SESSION_WORKSPACE_BYTES
            || bytes.contains(&0)
            || workspace
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            || workspace
                .components()
                .collect::<PathBuf>()
                .as_os_str()
                .as_bytes()
                != bytes
        {
            return Err(Error::new(Kind::InvalidQuery));
        }
        self.workspace = Some(workspace.to_owned());
        Ok(self)
    }
    /// Case-insensitive ASCII substring search over known title/workspace and
    /// the explicitly bounded canonical-user-text preview, not full transcript search.
    /// # Errors
    /// Rejects more than 1024 raw UTF-8 bytes or an embedded NUL before copying.
    pub fn with_search(mut self, search: &str) -> Result<Self, Error> {
        if search.len() > MAX_NATIVE_SESSION_CATALOG_QUERY_BYTES || search.contains('\0') {
            return Err(Error::new(Kind::InvalidQuery));
        }
        self.search = search
            .trim_matches([' ', '\t', '\r', '\n'])
            .to_ascii_lowercase();
        Ok(self)
    }
    /// Unknown activity never satisfies an explicitly supplied time predicate.
    #[must_use]
    pub const fn with_updated_since(mut self, since: i64) -> Self {
        self.updated_since = Some(since);
        self
    }
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    pub(super) fn matches(&self, entry: &NativeSessionCatalogEntry, scratch: &mut Vec<u8>) -> bool {
        let metadata = entry.native_metadata();
        if self
            .workspace
            .as_deref()
            .is_some_and(|path| metadata.workspace() != Some(path))
            || self
                .updated_since
                .is_some_and(|since| metadata.updated_at_ms().is_none_or(|time| time < since))
        {
            return false;
        }
        if self.search.is_empty() {
            return true;
        }
        let mut contains = |bytes: &[u8]| {
            scratch.clear();
            scratch.extend(bytes.iter().map(u8::to_ascii_lowercase));
            memchr::memmem::find(scratch, self.search.as_bytes()).is_some()
        };
        metadata
            .title()
            .is_some_and(|title| contains(title.as_bytes()))
            || metadata
                .workspace()
                .is_some_and(|path| contains(path.as_os_str().as_bytes()))
            || entry
                .preview()
                .is_some_and(|preview| contains(preview.as_bytes()))
    }
}
impl fmt::Debug for NativeSessionCatalogQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSessionCatalogQuery")
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}
