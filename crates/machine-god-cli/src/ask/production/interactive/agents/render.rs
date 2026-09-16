//! Bounded sanitized terminal projection. Clipped previews never become authority.
mod detail;
#[cfg(test)]
mod tests;
use machine_god_core::ManagedSubagentResult;
use machine_god_native::{NativeManagedNavigationRoute as Route, NativeManagedNavigationView};

pub(super) struct Frame {
    pub bytes: Vec<u8>,
    pub height: u16,
    pub selectable: bool,
}

pub(super) fn detail_count(result: Option<&ManagedSubagentResult>) -> usize {
    let mut count = 0;
    detail::visit(result, |_| count += 1);
    count
}

pub(super) fn render(
    view: &NativeManagedNavigationView<'_>,
    draft: (&str, usize),
    columns: u16,
    rows: u16,
    detail_offset: usize,
) -> Result<Frame, ()> {
    if columns <= 1 || rows == 0 {
        return Err(());
    }
    let mut lines = Lines {
        bytes: Vec::new(),
        columns,
        limit: usize::from(rows).min(64),
        count: 0,
    };
    if columns < 40 || rows < 11 {
        lines.push("Resize to navigate agents; Ctrl-X closes")?;
        return lines.finish(false);
    }
    lines.push("Agents & processes · clipped previews")?;
    lines.push(&format!(
        "{:?}{}",
        view.route,
        if view.busy { " — pending" } else { "" }
    ))?;
    // Keep target, error and editor visible independently of detail scrolling.
    let content_limit = lines
        .limit
        .saturating_sub(4 + usize::from(view.error.is_some()));
    match view.route {
        Route::Catalog(_) => catalog(&mut lines, view, content_limit)?,
        Route::Agent(_) | Route::ConfirmClose => {
            let target = view.target.ok_or(())?;
            lines.push(&format!(
                "{} — generation {}",
                prefix(&target.name),
                target.generation
            ))?;
            lines.push(&format!("id: {}", prefix(&target.id)))?;
            if view.route == Route::ConfirmClose {
                lines.push("Close and archive this agent? Enter confirms; Esc goes back.")?;
            } else {
                details(&mut lines, view.result, content_limit, detail_offset)?;
            }
        }
    }
    if let Some(error) = view.error {
        lines.push(&error.to_string())?;
    }
    lines.push(if view.has_next {
        "/next page · /refresh · /current /archived /all"
    } else {
        "/refresh · /current /archived /all"
    })?;
    lines.push("Enter opens/sends · /status /messages /tools /close · Ctrl-X exits")?;
    lines.push("Arrows select/scroll · /create {JSON} /configure {JSON}")?;
    lines.push("")?;
    let mut frame = lines.finish(true)?;
    let editor = super::super::composer_view::render(draft.0, draft.1, columns).map_err(|_| ())?;
    if frame.bytes.len() + editor.len() > 64 * 1024 {
        return Err(());
    }
    frame.bytes.extend(editor);
    Ok(frame)
}

fn catalog(
    lines: &mut Lines,
    view: &NativeManagedNavigationView<'_>,
    limit: usize,
) -> Result<(), ()> {
    let capacity = limit.saturating_sub(lines.count).max(1);
    let start = view
        .selected
        .unwrap_or(0)
        .saturating_sub(capacity / 2)
        .min(view.rows.len().saturating_sub(capacity));
    for (index, row) in view.rows.iter().enumerate().skip(start).take(capacity) {
        lines.push(&format!(
            "{} {}. {} [{:?}, g{}]{}",
            if view.selected == Some(index) {
                ">"
            } else {
                " "
            },
            index + 1,
            prefix(&row.name),
            row.state,
            row.generation,
            if row.recovery_required {
                " recovery required"
            } else {
                ""
            }
        ))?;
    }
    if view.rows.is_empty() {
        lines.push("No matching heads in this bounded page")?;
    }
    Ok(())
}

fn details(
    lines: &mut Lines,
    result: Option<&ManagedSubagentResult>,
    limit: usize,
    offset: usize,
) -> Result<(), ()> {
    let capacity = limit.saturating_sub(lines.count);
    let mut index = 0;
    let mut error = Ok(());
    // Reserve a continuation marker. Only retained data is visited; scrolling
    // does not poll a provider, load history or move a native catalog cursor.
    detail::visit(result, |text| {
        if index >= offset && index - offset < capacity.saturating_sub(1) && error.is_ok() {
            error = lines.push(text);
        }
        index += 1;
    });
    error?;
    if capacity != 0 {
        let end = offset.saturating_add(capacity.saturating_sub(1)).min(index);
        lines.push(&format!(
            "Rows {}–{} of {}{}",
            offset.saturating_add(1).min(index),
            end,
            index,
            if end < index { " · ↓ more" } else { "" }
        ))?;
    }
    Ok(())
}

fn prefix(text: &str) -> &str {
    &text[..text.floor_char_boundary(text.len().min(512))]
}
struct Lines {
    bytes: Vec<u8>,
    columns: u16,
    limit: usize,
    count: usize,
}
impl Lines {
    fn push(&mut self, text: &str) -> Result<(), ()> {
        if self.count == self.limit {
            return Err(());
        }
        let row = super::super::composer_view::label(text, self.columns, 512);
        if self.bytes.len() + row.len() + 2 > 64 * 1024 {
            return Err(());
        }
        if self.count != 0 {
            self.bytes.extend_from_slice(b"\r\n");
        }
        self.bytes.extend_from_slice(&row);
        self.count += 1;
        Ok(())
    }
    fn finish(self, selectable: bool) -> Result<Frame, ()> {
        Ok(Frame {
            bytes: self.bytes,
            height: u16::try_from(self.count.saturating_sub(1)).map_err(|_| ())?,
            selectable,
        })
    }
}
