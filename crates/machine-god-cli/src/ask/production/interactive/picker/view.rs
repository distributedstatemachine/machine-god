use super::{Picker, Row};
use machine_god_native::NativeSessionCatalogScope;

const ROW_BYTES: usize = 512;

pub(in super::super) struct Frame {
    pub bytes: Vec<u8>,
    /// Number of CRLFs below the start anchor; the caller clears this region
    /// before printing other content or a replacement menu.
    pub height: u16,
    pub generation: u64,
    pub revision: u64,
}

impl Picker {
    pub fn render(&mut self, columns: u16, rows: u16, now_ms: i64) -> Option<Frame> {
        let view = self.view.as_mut()?;
        if !view.dirty {
            return None;
        }
        view.dirty = false;
        let mut labels = Vec::new();
        labels.push(match view.scope {
            NativeSessionCatalogScope::CurrentWorkspace => "Resume — current workspace".to_owned(),
            NativeSessionCatalogScope::All => "Resume — all workspaces".to_owned(),
        });
        labels.push(format!("Search: {}", view.query));
        let capacity = usize::from(rows.saturating_sub(3)).clamp(1, 100);
        let start = view.selected.saturating_sub(capacity - 1);
        let items = view.matches.len() + usize::from(view.page.cursor.is_some());
        for position in (start..items).take(capacity) {
            labels.push(if let Some(index) = view.matches.get(position) {
                row_label(&view.page.rows[*index], position == view.selected, now_ms)
            } else if position == view.selected {
                "> Load more".to_owned()
            } else {
                "  Load more".to_owned()
            });
        }
        if items == 0 {
            labels.push(
                if view.loading {
                    "Loading sessions…"
                } else if view.query.trim().is_empty() {
                    "No resumable sessions"
                } else {
                    "No matching loaded sessions"
                }
                .to_owned(),
            );
        }
        labels.push(if let Some(failure) = view.failure {
            failure.to_owned()
        } else if view.selecting {
            "Opening selected session…".to_owned()
        } else if view.loading {
            "Loading sessions…".to_owned()
        } else if view.page.incomplete {
            "Catalog limit reached; listing is incomplete".to_owned()
        } else if view.page.skipped_invalid != 0 {
            format!("Skipped {} invalid records", view.page.skipped_invalid)
        } else {
            "Enter select · ↑/↓ navigate · Tab workspace/all · Esc cancel".to_owned()
        });
        // Tiny terminals still get bounded, non-wrapping output. No guessed
        // terminal size, invisible selectable row, or unbounded escape payload.
        if rows < 4 {
            labels.drain(..2);
        }
        labels.truncate(usize::from(rows).max(1));
        let mut bytes = Vec::new();
        for (index, label) in labels.iter().enumerate() {
            if index != 0 {
                bytes.extend_from_slice(b"\r\n");
            }
            bytes.extend(super::super::composer_view::label(
                label, columns, ROW_BYTES,
            ));
        }
        Some(Frame {
            bytes,
            height: u16::try_from(labels.len().saturating_sub(1)).expect("bounded menu rows"),
            generation: view.generation,
            revision: view.revision,
        })
    }
}

fn row_label(row: &Row, selected: bool, now_ms: i64) -> String {
    format!(
        "{} {} · {} · {} · {} turns",
        if selected { ">" } else { " " },
        row.title,
        row.workspace_name,
        age(row.updated_at_ms, now_ms),
        row.turns
    )
}

fn age(updated_at_ms: Option<i64>, now_ms: i64) -> String {
    let Some(updated) = updated_at_ms else {
        return "unknown activity".into();
    };
    let minutes = now_ms.saturating_sub(updated).max(0) / 60_000;
    if minutes == 0 {
        "just now".into()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if minutes < 1440 {
        format!("{}h ago", minutes / 60)
    } else {
        format!("{}d ago", minutes / 1440)
    }
}
