//! Presentation-only session picker. The native reader owns every filesystem
//! operation; rows retain observed identities, never a writable session.

use machine_god_core::{BoxFuture, CancellationToken, SessionId};
use machine_god_native::{
    NativeObservedSession, NativeSessionCatalogCursor, NativeSessionCatalogEntry,
    NativeSessionCatalogPage, NativeSessionCatalogReadError, NativeSessionCatalogReader,
    NativeSessionCatalogScope,
};
use std::{
    sync::Arc,
    task::{Context, Poll},
};

#[cfg(test)]
mod tests;
mod view;

pub(super) const MAX_QUERY_BYTES: usize = 256;
const MAX_ROWS: usize = 1024;

/// Only bounded display fields survive a catalog response. In particular, the
/// potentially much larger native metadata/preferences object is not cached.
struct Row {
    observed: NativeObservedSession,
    title: String,
    workspace: Vec<u8>,
    workspace_name: String,
    preview: String,
    updated_at_ms: Option<i64>,
    turns: usize,
}

impl Row {
    fn from_entry(entry: &NativeSessionCatalogEntry) -> Self {
        let metadata = entry.native_metadata();
        let workspace = metadata.workspace().or_else(|| metadata.origin_workspace());
        Self {
            observed: NativeObservedSession::from_entry(entry),
            title: metadata
                .title()
                .filter(|title| !title.is_empty())
                .unwrap_or("Untitled session")
                .to_owned(),
            workspace: workspace.map_or_else(
                || b"(unknown workspace)".to_vec(),
                |path| path.as_os_str().as_encoded_bytes().to_vec(),
            ),
            workspace_name: workspace.map_or_else(
                || "(unknown workspace)".to_owned(),
                |path| {
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                },
            ),
            preview: entry.preview().unwrap_or_default().to_owned(),
            updated_at_ms: metadata.updated_at_ms(),
            turns: entry.history_len(),
        }
    }

    fn matches(&self, query: &[u8]) -> bool {
        contains_ascii(self.title.as_bytes(), query)
            || contains_ascii(&self.workspace, query)
            || contains_ascii(self.preview.as_bytes(), query)
    }
}

fn contains_ascii(text: &[u8], query: &[u8]) -> bool {
    query.is_empty()
        || text
            .windows(query.len())
            .any(|part| part.eq_ignore_ascii_case(query))
}

#[derive(Clone, Default)]
struct Page {
    rows: Vec<Arc<Row>>,
    cursor: Option<NativeSessionCatalogCursor>,
    incomplete: bool,
    skipped_invalid: usize,
}

impl Page {
    fn project(page: &NativeSessionCatalogPage, exclude: Option<&SessionId>) -> Self {
        Self {
            rows: page
                .entries()
                .iter()
                .filter(|entry| Some(entry.id()) != exclude && entry.history_len() != 0)
                .map(|entry| Arc::new(Row::from_entry(entry)))
                .collect(),
            // The cursor comes from the UNFILTERED native page. Neither search
            // nor current-session exclusion can move the pagination boundary.
            cursor: page.next_cursor(),
            incomplete: !page.scan_complete(),
            skipped_invalid: page.skipped_invalid(),
        }
    }
}

struct View {
    generation: u64,
    revision: u64,
    acknowledged: Option<u64>,
    scope: NativeSessionCatalogScope,
    query: String,
    page: Page,
    matches: Vec<usize>,
    selected: usize,
    failure: Option<&'static str>,
    loading: bool,
    selecting: bool,
    dirty: bool,
}

impl View {
    fn changed(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("bounded picker view revisions");
        self.acknowledged = None;
        self.dirty = true;
    }

    fn filter(&mut self) {
        let query = self.query.trim_matches([' ', '\t', '\r', '\n']).as_bytes();
        self.matches = self
            .page
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.matches(query).then_some(index))
            .collect();
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }
}

#[derive(Clone)]
struct Request {
    generation: u64,
    scope: NativeSessionCatalogScope,
    limit: usize,
    cursor: Option<NativeSessionCatalogCursor>,
}

struct Pending {
    request: Request,
    cancel: CancellationToken,
    future: BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>>,
}

pub(super) enum Selection {
    None,
    LoadMore,
    Session(NativeObservedSession),
}

pub(super) struct Picker {
    reader: NativeSessionCatalogReader,
    exclude: Option<SessionId>,
    generation: u64,
    cache: [Option<Page>; 2],
    view: Option<View>,
    pending: Option<Pending>,
    queued: Option<Request>,
    page_size: usize,
}

impl Picker {
    pub fn new(reader: NativeSessionCatalogReader, exclude: Option<SessionId>, rows: u16) -> Self {
        Self {
            reader,
            exclude,
            generation: 0,
            cache: [None, None],
            view: None,
            pending: None,
            queued: None,
            page_size: page_size(rows),
        }
    }

    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }

    pub fn current_query(&self) -> &str {
        self.view.as_ref().map_or("", |view| view.query.as_str())
    }

    pub fn open(&mut self, scope: NativeSessionCatalogScope) {
        self.open_query(scope, String::new());
    }

    fn open_query(&mut self, scope: NativeSessionCatalogScope, query: String) {
        self.close();
        self.generation = self
            .generation
            .checked_add(1)
            .expect("bounded picker generations");
        let mut view = View {
            generation: self.generation,
            revision: 0,
            acknowledged: None,
            scope,
            query,
            page: self.cache[scope_index(scope)].clone().unwrap_or_default(),
            matches: Vec::new(),
            selected: 0,
            failure: None,
            loading: true,
            selecting: false,
            dirty: true,
        };
        view.filter();
        self.view = Some(view);
        self.queue(None);
    }

    pub fn close(&mut self) {
        self.view = None;
        self.queued = None;
        if let Some(pending) = &self.pending {
            pending.cancel.cancel();
        }
        // Keep the cancelled future until its response is observed. Switching
        // scope never launches unbounded concurrent scans; native scope join is
        // still required independently of response delivery.
    }

    pub fn set_current(&mut self, id: SessionId) {
        self.close();
        self.exclude = Some(id);
        self.cache = [None, None];
    }

    pub fn resize(&mut self, rows: u16) {
        self.page_size = page_size(rows);
        if let Some(view) = &mut self.view {
            view.changed();
        }
    }

    pub fn redraw(&mut self) {
        if let Some(view) = &mut self.view {
            view.changed();
        }
    }

    pub fn toggle_scope(&mut self) {
        let Some(view) = &self.view else { return };
        if view.selecting {
            return;
        }
        let scope = match view.scope {
            NativeSessionCatalogScope::CurrentWorkspace => NativeSessionCatalogScope::All,
            NativeSessionCatalogScope::All => NativeSessionCatalogScope::CurrentWorkspace,
        };
        self.open_query(scope, view.query.clone());
    }

    pub fn query(&mut self, query: &str) -> Result<(), ()> {
        if query.len() > MAX_QUERY_BYTES {
            return Err(());
        }
        if let Some(view) = &mut self.view {
            if view.selecting || view.query == query {
                return Ok(());
            }
            query.clone_into(&mut view.query);
            view.selected = 0;
            view.failure = None;
            view.filter();
            view.changed();
        }
        Ok(())
    }

    pub fn move_selection(&mut self, forward: bool) {
        let Some(view) = &mut self.view else { return };
        if view.selecting {
            return;
        }
        let last = if view.page.cursor.is_some() {
            view.matches.len()
        } else {
            view.matches.len().saturating_sub(1)
        };
        view.selected = if forward {
            view.selected.saturating_add(1).min(last)
        } else {
            view.selected.saturating_sub(1)
        };
        view.failure = None;
        view.changed();
        if forward && view.selected >= view.matches.len().saturating_sub(1) {
            self.load_more();
        }
    }

    pub fn identity(&self) -> Option<(u64, u64)> {
        self.view
            .as_ref()
            .map(|view| (view.generation, view.revision))
    }

    pub fn acknowledge(&mut self, generation: u64, revision: u64) {
        if let Some(view) = &mut self.view
            && (view.generation, view.revision) == (generation, revision)
        {
            view.acknowledged = Some(revision);
        }
    }

    pub fn input_binding(&self) -> Option<super::InputBinding> {
        self.view.as_ref().map(|view| {
            if view.acknowledged == Some(view.revision) {
                super::InputBinding::Picker {
                    generation: view.generation,
                    revision: view.revision,
                }
            } else {
                super::InputBinding::AwaitingPicker {
                    generation: view.generation,
                }
            }
        })
    }

    pub fn select(&mut self, generation: u64, revision: u64) -> Selection {
        let Some(view) = &mut self.view else {
            return Selection::None;
        };
        if (view.generation, view.revision) != (generation, revision)
            || view.acknowledged != Some(revision)
            || view.selecting
        {
            return Selection::None;
        }
        if let Some(index) = view.matches.get(view.selected) {
            let target = view.page.rows[*index].observed.clone();
            view.selecting = true;
            view.changed();
            Selection::Session(target)
        } else if !view.loading && view.page.cursor.is_some() {
            self.load_more();
            Selection::LoadMore
        } else {
            Selection::None
        }
    }

    pub fn selection_failed(&mut self, reason: &'static str) {
        if let Some(view) = &mut self.view {
            view.selecting = false;
            view.failure = Some(reason);
            view.changed();
        }
    }

    fn queue(&mut self, cursor: Option<NativeSessionCatalogCursor>) {
        let Some(view) = &mut self.view else { return };
        view.loading = true;
        self.queued = Some(Request {
            generation: view.generation,
            scope: view.scope,
            limit: self.page_size,
            cursor,
        });
    }

    fn load_more(&mut self) {
        let Some(view) = &self.view else { return };
        if view.loading || view.selecting || view.page.rows.len() >= MAX_ROWS {
            return;
        }
        if let Some(cursor) = view.page.cursor.clone() {
            self.queue(Some(cursor));
        }
    }

    /// At most one worker response and one request admission per poll. Native
    /// scans never run inside this presentation poll.
    pub fn poll(&mut self, cx: &mut Context<'_>) {
        if let Some(pending) = &mut self.pending {
            let Poll::Ready(result) = pending.future.as_mut().poll(cx) else {
                return;
            };
            let request = self.pending.take().expect("ready request").request;
            if self
                .view
                .as_ref()
                .is_some_and(|view| view.generation == request.generation)
            {
                self.accept(&request, result);
            }
            cx.waker().wake_by_ref();
        }
        if let Some(request) = self.queued.take() {
            let cancel = CancellationToken::new();
            let future = self.reader.list(
                request.scope,
                request.limit,
                request.cursor.clone(),
                cancel.clone(),
            );
            self.pending = Some(Pending {
                request,
                cancel,
                future,
            });
            cx.waker().wake_by_ref();
        }
    }

    fn accept(
        &mut self,
        request: &Request,
        result: Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>,
    ) {
        let view = self.view.as_mut().expect("current request");
        view.loading = false;
        match result {
            Err(error) => {
                view.failure = Some(match error {
                    NativeSessionCatalogReadError::Busy => {
                        "Session catalog is busy; reopen to retry"
                    }
                    NativeSessionCatalogReadError::Cancelled => "Session catalog read cancelled",
                    _ => "Session catalog unavailable; reopen to retry",
                });
            }
            Ok(page) => {
                let page = Page::project(&page, self.exclude.as_ref());
                if request.cursor.is_none() {
                    self.cache[scope_index(request.scope)] = Some(page.clone());
                    view.page = page;
                    view.selected = 0;
                    view.filter();
                } else {
                    let previous = view.page.rows.len();
                    for row in page.rows {
                        if view.page.rows.len() == MAX_ROWS {
                            break;
                        }
                        if !view
                            .page
                            .rows
                            .iter()
                            .any(|old| old.observed.id() == row.observed.id())
                        {
                            view.page.rows.push(row);
                        }
                    }
                    view.page.cursor = page.cursor;
                    view.page.incomplete |= page.incomplete;
                    view.page.skipped_invalid = page.skipped_invalid;
                    if view.page.rows.len() == MAX_ROWS && view.page.cursor.is_some() {
                        view.page.cursor = None;
                        view.page.incomplete = true;
                    }
                    view.filter();
                    if let Some(index) = view.matches.iter().position(|index| *index >= previous) {
                        view.selected = index;
                    }
                }
                view.failure = None;
            }
        }
        view.changed();
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        self.close();
    }
}

const fn scope_index(scope: NativeSessionCatalogScope) -> usize {
    match scope {
        NativeSessionCatalogScope::CurrentWorkspace => 0,
        NativeSessionCatalogScope::All => 1,
    }
}

fn page_size(rows: u16) -> usize {
    usize::from(rows.saturating_sub(7)).clamp(10, 100)
}
