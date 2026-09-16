//! Pure wrapping over one native-owned canonical record. Only one row and the
//! visible output are allocated; source coordinates, not terminal rows, persist.
use super::{Lines, identity_rows};
use crate::ask::production::interactive::composer_view::{Atom, label};
use machine_god_core::{ContentBlock, Role, SessionRecord};
use machine_god_native::{
    NativeManagedHistoryPosition as Position, NativeManagedHistoryView as View,
    NativeManagedNavigationAction as Action, NativeManagedNavigationView,
};

const ROW_BYTES: usize = 768;

pub(super) fn render(lines: &mut Lines, history: Option<View<'_>>, limit: usize) -> Result<(), ()> {
    let Some(history) = history else {
        return lines.push("Canonical conversation unavailable or loading; /refresh retries");
    };
    let capacity = limit.saturating_sub(lines.count).saturating_sub(1);
    let (start, total) = window(history, lines.columns, capacity);
    let mut index = 0;
    let mut result = Ok(());
    visit(history.record, lines.columns, |_, row| {
        if index >= start && index - start < capacity && result.is_ok() {
            result = lines.push_rendered(row);
        }
        index += 1;
    });
    result?;
    lines.push(&format!(
        "History rows {}–{} of {total}{}",
        (start + 1).min(total),
        (start + capacity).min(total),
        if history.position.is_none() {
            " · following tail"
        } else {
            ""
        }
    ))
}

pub(in super::super) fn scroll(
    view: &NativeManagedNavigationView<'_>,
    columns: u16,
    rows: u16,
    previous: bool,
    page: bool,
) -> Option<Action> {
    let history = view.history?;
    let heading = identity_rows(&view.target?.id, columns)?.checked_add(3)?;
    let capacity = usize::from(rows).min(64).saturating_sub(
        heading + 5 + usize::from(view.error.is_some()) + usize::from(view.result.is_some()),
    );
    scroll_from(history, columns, capacity, previous, page)
}

fn scroll_from(
    history: View<'_>,
    columns: u16,
    capacity: usize,
    previous: bool,
    page: bool,
) -> Option<Action> {
    if capacity == 0 {
        return None;
    }
    let (start, total) = window(history, columns, capacity);
    let step = if page { capacity } else { 1 };
    let next = if previous {
        start.saturating_sub(step)
    } else {
        start
            .saturating_add(step)
            .min(total.saturating_sub(capacity))
    };
    if !previous && next == total.saturating_sub(capacity) {
        // A resize may clamp an old anchor to the tail without clearing it.
        // Down still resumes following, even if no screen row needs to move.
        return history
            .position
            .is_some()
            .then_some(Action::SeekHistory(None));
    }
    if next == start {
        return None;
    }
    let mut selected = None;
    let mut index = 0;
    visit(history.record, columns, |position, _| {
        if index == next {
            selected = Some(position);
        }
        index += 1;
    });
    selected.map(|position| Action::SeekHistory(Some(position)))
}

fn window(history: View<'_>, columns: u16, capacity: usize) -> (usize, usize) {
    let mut total: usize = 0;
    let mut anchored = 0;
    visit(history.record, columns, |position, _| {
        if history.position.is_some_and(|anchor| position <= anchor) {
            anchored = total;
        }
        total += 1;
    });
    let tail = total.saturating_sub(capacity);
    (
        if history.position.is_some() {
            anchored.min(tail)
        } else {
            tail
        },
        total,
    )
}

fn visit(record: &SessionRecord, columns: u16, mut row: impl FnMut(Position, &[u8])) {
    for (message_index, message) in record.messages.iter().enumerate() {
        if message.role == Role::System {
            continue;
        }
        let mut position = Position {
            message: message_index,
            block: None,
            byte: 0,
        };
        row(
            position,
            match message.role {
                Role::User => b"[user]",
                Role::Assistant => b"[assistant]",
                Role::Tool => b"[recorded tool evidence]",
                _ => b"[unrecognized history role]",
            },
        );
        for (block_index, block) in message.content.iter().enumerate() {
            position.block = Some(block_index);
            position.byte = 0;
            if let ContentBlock::Text { text } = block
                && matches!(message.role, Role::User | Role::Assistant)
            {
                text_rows(text, columns, |byte, text| {
                    position.byte = byte;
                    row(position, text);
                });
            } else {
                let description = match block {
                    ContentBlock::ToolCall { .. } => "[recorded tool call · /tools for activity]",
                    ContentBlock::ToolResult { .. } => "[recorded tool result · detail collapsed]",
                    ContentBlock::Json { .. } => "[structured detail collapsed]",
                    ContentBlock::Text { .. } => "[non-conversation text collapsed]",
                    _ => "[non-text content collapsed]",
                };
                row(position, &label(description, columns, ROW_BYTES));
            }
        }
    }
}

fn text_rows(text: &str, columns: u16, mut row: impl FnMut(usize, &[u8])) {
    let capacity = usize::from(columns.saturating_sub(1)).max(1);
    let mut bytes = Vec::new();
    let (mut start, mut next, mut cells) = (0, 0, 0);
    while next < text.len() {
        if text.as_bytes()[next] == b'\n' {
            row(start, &bytes);
            bytes.clear();
            cells = 0;
            next += 1;
            start = next;
            continue;
        }
        let atom = Atom::at(text, next, capacity).bounded_bytes(ROW_BYTES);
        if !bytes.is_empty()
            && (cells + atom.cells > capacity || bytes.len() + atom.bytes > ROW_BYTES)
        {
            row(start, &bytes);
            bytes.clear();
            cells = 0;
            start = next;
        }
        atom.append(text, &mut bytes);
        cells += atom.cells;
        next = atom.end;
    }
    row(start, &bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{Message, SessionId, SessionIncarnationId};

    fn record(text: &str) -> SessionRecord {
        let mut record = SessionRecord::empty(
            SessionId::new("history").unwrap(),
            SessionIncarnationId::new("original").unwrap(),
        );
        record.messages = vec![
            Message::text(Role::System, "private instructions"),
            Message::text(Role::Assistant, text),
        ];
        record
    }

    #[test]
    fn wrapping_preserves_complete_unicode_text_beyond_preview_bounds() {
        let text = "α🙂é word ".repeat(4096);
        for columns in [40, 80, 120, u16::MAX] {
            let mut combined = Vec::new();
            let mut previous = 0;
            text_rows(&text, columns, |byte, row| {
                assert!(byte >= previous && text.is_char_boundary(byte));
                previous = byte;
                assert!(row.len() <= ROW_BYTES);
                std::str::from_utf8(row).unwrap();
                combined.extend_from_slice(row);
            });
            assert_eq!(combined, text.as_bytes());
        }
    }

    #[test]
    fn multiline_tail_and_source_anchor_survive_reflow_without_system_text() {
        let text = (1..=90)
            .map(|n| format!("CHILD_POSITION_{n:03} α🙂"))
            .collect::<Vec<_>>()
            .join("\n");
        let record = record(&text);
        let tail = View {
            record: &record,
            position: None,
        };
        assert_eq!(window(tail, 80, 10), (81, 91));
        let byte = text.find("CHILD_POSITION_040").unwrap();
        let anchored = View {
            record: &record,
            position: Some(Position {
                message: 1,
                block: Some(0),
                byte,
            }),
        };
        for columns in [40, 80, 120] {
            let (start, _) = window(anchored, columns, 10);
            assert_eq!(start, 40);
            let mut shown = Vec::new();
            visit(&record, columns, |_, row| shown.extend_from_slice(row));
            assert!(
                !String::from_utf8(shown)
                    .unwrap()
                    .contains("private instructions")
            );
        }
        let (start, _) = window(anchored, 12, 10);
        let mut index = 0;
        visit(&record, 12, |position, _| {
            if index == start {
                assert_eq!(Some(position), anchored.position);
            }
            index += 1;
        });
    }

    #[test]
    fn controls_and_large_combining_clusters_are_bounded_without_terminal_escapes() {
        let text = format!(
            "\x1b]52;c;secret\x07\r\t\u{202e}\na{}tail",
            "\u{301}".repeat(8192)
        );
        let mut shown = Vec::new();
        text_rows(&text, 40, |_, row| {
            assert!(row.len() <= ROW_BYTES);
            assert!(!row.iter().any(|byte| matches!(*byte, 7 | 9 | 13 | 27)));
            shown.extend_from_slice(row);
        });
        let shown = String::from_utf8(shown).unwrap();
        assert!(shown.contains("\\u{1b}"));
        assert!(shown.contains("…tail"));
        assert!(!shown.contains('\u{202e}'));
    }

    #[test]
    fn down_resumes_tail_following_when_resize_already_clamped_the_anchor() {
        let record = record("first\nsecond\nthird");
        let position = Position {
            message: 1,
            block: Some(0),
            byte: 6,
        };
        let anchored = View {
            record: &record,
            position: Some(position),
        };
        assert!(matches!(
            scroll_from(anchored, 80, 20, false, false),
            Some(Action::SeekHistory(None))
        ));
        assert!(matches!(
            scroll_from(anchored, 80, 20, false, true),
            Some(Action::SeekHistory(None))
        ));
        assert!(
            scroll_from(
                View {
                    position: None,
                    ..anchored
                },
                80,
                20,
                false,
                false
            )
            .is_none()
        );
        assert!(scroll_from(anchored, 80, 20, true, false).is_none());
    }
}
