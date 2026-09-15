//! Bounded selections and opaque pageable journal cursors; neither grants authority.
use super::super::store::{JournalHistoryCursor, JournalTranscript};
use super::{
    Arc, JournalRecord, JournalSnapshot, ManagedJournal, ManagedManager, NativeConversationRuntime,
    Weak, command, fmt,
};
use machine_god_core::{
    ManagedAgentState, ManagedCursor, ManagedFailureCode, ManagedInspect, ManagedInspectSection,
    ManagedInspection, ManagedInspectionSourceError, ManagedQueuedMessage, ManagedRequested,
    ManagedResultStatus, ManagedSubagentResult,
};
use std::sync::Mutex;

pub(super) struct SelectionIdentity;
#[derive(Clone)]
pub(crate) struct ManagedSelection(Weak<SelectionIdentity>);
pub(crate) struct ManagedChildProjection {
    pub id: String,
    pub generation: u64,
    pub name: String,
    pub state: ManagedAgentState,
    pub selection: ManagedSelection,
}
impl fmt::Debug for ManagedSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ManagedSelection { .. }")
    }
}
impl fmt::Debug for ManagedChildProjection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedChildProjection")
            .field("generation", &self.generation)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
impl ManagedManager {
    pub(crate) fn children(&self) -> Vec<super::ManagedChildProjection> {
        self.children
            .iter()
            .map(|child| ManagedChildProjection {
                id: child.snapshot.head.id.clone(),
                generation: child.snapshot.head.generation,
                name: child.snapshot.head.configuration.name.clone(),
                state: child.snapshot.head.status,
                selection: ManagedSelection(Arc::downgrade(&child.selection)),
            })
            .collect()
    }
    /// Selection only exposes observation/navigation; callers must route durable
    /// user messages through native admission rather than enqueue this runtime.
    pub(crate) fn selected_runtime(
        &self,
        selection: &super::ManagedSelection,
    ) -> Option<&Arc<NativeConversationRuntime>> {
        self.children
            .iter()
            .find(|child| selection.0.ptr_eq(&Arc::downgrade(&child.selection)))
            .map(|child| &child.prepared.runtime)
    }
}

#[derive(Default)]
pub(super) struct CursorBook {
    next: u64,
    entries: Vec<CursorEntry>,
}
struct CursorEntry {
    token: u64,
    child: String,
    owner: JournalTranscript,
    revision: u64,
    sections: Vec<ManagedInspectSection>,
    queue: usize,
    history: Option<JournalHistoryCursor>,
}
impl CursorBook {
    fn restore(
        &self,
        snapshot: &JournalSnapshot,
        query: &ManagedInspect,
    ) -> Option<(usize, Option<JournalHistoryCursor>)> {
        let raw = query.cursor.as_ref()?;
        let cursor = ManagedCursor::parse(raw).ok()?;
        self.entries
            .iter()
            .find(|entry| {
                entry.token == cursor.offset
                    && cursor.generation == entry.revision
                    && entry.revision == snapshot.head.revision
                    && entry.child == snapshot.head.id
                    && entry.owner == snapshot.head.transcript
                    && entry.sections == query.sections
            })
            .map(|entry| (entry.queue, entry.history.clone()))
    }
    fn retain(
        &mut self,
        snapshot: &JournalSnapshot,
        query: &ManagedInspect,
        queue: usize,
        history: Option<JournalHistoryCursor>,
    ) -> Option<String> {
        self.next = self.next.checked_add(1)?;
        if self.entries.len() == 64 {
            self.entries.remove(0);
        }
        self.entries.push(CursorEntry {
            token: self.next,
            child: snapshot.head.id.clone(),
            owner: snapshot.head.transcript.clone(),
            revision: snapshot.head.revision,
            sections: query.sections.clone(),
            queue,
            history,
        });
        Some(
            ManagedCursor {
                generation: snapshot.head.revision,
                offset: self.next,
            }
            .encode(),
        )
    }
}

#[allow(clippy::too_many_lines)] // Shared encoded byte budget across all selected projections.
pub(super) async fn inspect(
    journal: &ManagedJournal,
    cursors: &Mutex<CursorBook>,
    snapshot: &JournalSnapshot,
    query: &ManagedInspect,
    operation: &str,
    timeout: bool,
) -> ManagedSubagentResult {
    let selected = |section| query.sections.contains(&section);
    let mut projection = ManagedInspection {
        child_id: snapshot.head.id.clone(),
        generation: snapshot.head.generation,
        status: selected(ManagedInspectSection::Status).then_some(snapshot.head.status),
        configuration: selected(ManagedInspectSection::Configuration)
            .then(|| snapshot.head.configuration.clone()),
        relationship_selected: selected(ManagedInspectSection::Relationship),
        parent_id: selected(ManagedInspectSection::Relationship)
            .then(|| snapshot.head.parent_id.clone())
            .flatten(),
        tool_activity_selected: selected(ManagedInspectSection::ToolActivity),
        failure_work_id: snapshot
            .head
            .failure
            .as_ref()
            .map(|failure| failure.work_id.clone()),
        failure_reason: snapshot
            .head
            .failure
            .as_ref()
            .map(|failure| failure.reason.clone()),
        ..ManagedInspection::default()
    };
    let (mut queue_offset, history_cursor) = if query.cursor.is_some() {
        let restored = cursors.lock().unwrap().restore(snapshot, query);
        let Some(position) = restored else {
            projection.restart_required = true;
            return result(operation, projection, timeout);
        };
        position
    } else {
        (0, None)
    };
    let mut remaining = query.limit;
    let mut serialized = 0usize;
    if selected(ManagedInspectSection::Messages) {
        while remaining > 0 && queue_offset < snapshot.head.queue.len() {
            let reference = &snapshot.head.queue[queue_offset];
            let Ok(work) = journal.read_work(reference.page.clone()).await else {
                projection.history_error = Some(ManagedInspectionSourceError::Unavailable);
                break;
            };
            // Worst-case escaping and the entire structural result share 512 KiB.
            let bytes = work.content.len().saturating_mul(6).saturating_add(4096);
            if serialized + bytes > 448 * 1024 {
                break;
            }
            serialized += bytes;
            projection.messages.push(ManagedQueuedMessage {
                id: work.id,
                source_id: work.source_id,
                content: work.content,
                status: reference.status,
                cancellation_reason: None,
                created_at_ms: work.accepted_at_ms,
            });
            queue_offset += 1;
            remaining -= 1;
        }
    } else {
        queue_offset = snapshot.head.queue.len();
    }
    let mut next_history = history_cursor.clone();
    let mut history_more = false;
    let history_budget = (448 * 1024usize)
        .saturating_sub(serialized)
        .saturating_sub(64 * 1024);
    let text_reservation = if selected(ManagedInspectSection::Messages) {
        32 * 1024 * 6
    } else {
        0
    };
    let mut scan_limit = if history_budget < text_reservation {
        0
    } else {
        remaining
    };
    if selected(ManagedInspectSection::Events) {
        scan_limit = scan_limit.min(history_budget.saturating_sub(text_reservation) / (32 * 1024));
    } else if selected(ManagedInspectSection::ToolActivity) {
        scan_limit = scan_limit.min(history_budget.saturating_sub(text_reservation) / (4 * 1024));
    }
    if queue_offset == snapshot.head.queue.len()
        && scan_limit > 0
        && query.sections.iter().any(|section| {
            matches!(
                section,
                ManagedInspectSection::Messages
                    | ManagedInspectSection::Events
                    | ManagedInspectSection::ToolActivity
            )
        })
    {
        // Every call scans at most 100 raw records, including unselected records.
        // The retained journal cursor permits bounded progress through sparse pages.
        if let Ok(page) = journal
            .history(snapshot.clone(), history_cursor, scan_limit)
            .await
        {
            let mut history_bytes = 0;
            for record in page.records {
                match record {
                    JournalRecord::History(mut item)
                        if selected(ManagedInspectSection::Messages) =>
                    {
                        for (text, truncated) in [
                            (&mut item.user, &mut item.user_truncated),
                            (&mut item.assistant, &mut item.assistant_truncated),
                        ] {
                            if let Some(value) = text {
                                let allowed = (32 * 1024usize).saturating_sub(history_bytes);
                                let (bounded, cut) = prefix(value, allowed);
                                history_bytes += bounded.len();
                                *value = bounded;
                                *truncated |= cut;
                            }
                        }
                        projection.history.push(item);
                    }
                    JournalRecord::Event(item) if selected(ManagedInspectSection::Events) => {
                        projection.events.push(item);
                    }
                    JournalRecord::Tool(item) if selected(ManagedInspectSection::ToolActivity) => {
                        projection.tool_activity.push(item);
                    }
                    _ => {}
                }
            }
            next_history = page.next;
            history_more = next_history.is_some();
        } else {
            projection.history_error = selected(ManagedInspectSection::Messages)
                .then_some(ManagedInspectionSourceError::Unavailable);
            projection.tool_activity_error = selected(ManagedInspectSection::ToolActivity)
                .then_some(ManagedInspectionSourceError::Unavailable);
        }
    } else if queue_offset < snapshot.head.queue.len() || scan_limit == 0 {
        history_more = snapshot.head.history_tail.is_some();
    }
    if queue_offset < snapshot.head.queue.len() || history_more {
        projection.next_cursor =
            cursors
                .lock()
                .unwrap()
                .retain(snapshot, query, queue_offset, next_history);
        projection.restart_required = projection.next_cursor.is_none();
    }
    let mut output = result(operation, projection, timeout);
    if output.validate().is_err() {
        output = command::rejected(operation, ManagedFailureCode::ResultTooLarge);
    }
    output
}
fn result(operation: &str, inspection: ManagedInspection, timeout: bool) -> ManagedSubagentResult {
    ManagedSubagentResult {
        ok: true,
        operation_id: operation.into(),
        child_id: Some(inspection.child_id.clone()),
        status: if timeout {
            ManagedResultStatus::WaitTimedOut
        } else {
            ManagedResultStatus::Inspected
        },
        error_code: None,
        retryable: false,
        cursor: inspection.next_cursor.clone(),
        requested: Some(ManagedRequested::Inspection(Box::new(inspection))),
    }
}
pub(super) fn prefix(text: &str, max: usize) -> (String, bool) {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), end != text.len())
}
