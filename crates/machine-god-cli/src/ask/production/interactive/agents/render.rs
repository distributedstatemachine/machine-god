//! Bounded sanitized terminal projection. Clipped previews never become authority.
mod detail;
mod forms;
mod history;
mod models;
mod processes;
pub(super) use history::scroll as scroll_history;
pub(super) use processes::count as process_count;
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
    if !matches!(
        view.route,
        Route::Form(machine_god_native::NativeManagedFormKind::Create)
    ) && let Some(target) = view.target
    {
        let required = identity_rows(&target.id, columns)
            .and_then(|id_rows| id_rows.checked_add(9 + usize::from(view.error.is_some())));
        if required.is_none_or(|required| required > lines.limit) {
            lines.push("Resize to display the complete target identity")?;
            return lines.finish(false);
        }
    }
    lines.push(if view.route == Route::Conversation {
        "Agents & processes · canonical conversation"
    } else {
        "Agents & processes · clipped previews"
    })?;
    if let Some(history) = view.history {
        use machine_god_native::NativeManagedHistoryMode;
        lines.push(match history.mode {
            NativeManagedHistoryMode::Conversation => {
                "Conversation · Ctrl-O opens transcript detail"
            }
            NativeManagedHistoryMode::Transcript => "Transcript · ←/→ switch · Ctrl-O close",
            NativeManagedHistoryMode::Full => "Full detail · ←/→ switch · Ctrl-O close",
        })?;
    } else {
        lines.push(&format!(
            "{:?}{}",
            view.route,
            if view.busy { " — pending" } else { "" }
        ))?;
    }
    // Keep target, error and editor visible independently of detail scrolling.
    let content_limit = lines
        .limit
        .saturating_sub(4 + usize::from(view.error.is_some()));
    let mut selectable = true;
    match view.route {
        Route::Models => {
            target_heading(&mut lines, view.target.ok_or(())?)?;
            selectable =
                models::render(&mut lines, view.models.as_ref().ok_or(())?, content_limit)?;
        }
        Route::Conversation => {
            target_heading(&mut lines, view.target.ok_or(())?)?;
            if let Some(result) = view.result {
                lines.push(&format!(
                    "Receipt: {:?} · {:?}",
                    result.status, result.error_code
                ))?;
            }
            history::render(&mut lines, view.history, content_limit)?;
        }
        Route::Catalog(_) => catalog(&mut lines, view, content_limit)?,
        Route::Agent(_) | Route::ConfirmClose => {
            let target = view.target.ok_or(())?;
            target_heading(&mut lines, target)?;
            if view.route == Route::ConfirmClose {
                lines.push("Close and archive this agent? Enter confirms; Esc goes back.")?;
            } else {
                details(&mut lines, view.result, content_limit, detail_offset)?;
            }
        }
        Route::Form(_) => forms::render(&mut lines, view, content_limit)?,
        Route::Processes(_) => {
            if let Some(target) = view.target {
                target_heading(&mut lines, target)?;
            }
            processes::render(&mut lines, view, content_limit, detail_offset)?;
        }
    }
    if let Some(error) = view.error {
        lines.push(&error.to_string())?;
    }
    footer(&mut lines, view)?;
    let mut frame = lines.finish(selectable)?;
    let editor = super::super::composer_view::render(draft.0, draft.1, columns).map_err(|_| ())?;
    if frame.bytes.len() + editor.len() > 64 * 1024 {
        return Err(());
    }
    frame.bytes.extend(editor);
    Ok(frame)
}

fn footer(lines: &mut Lines, view: &NativeManagedNavigationView<'_>) -> Result<(), ()> {
    if view.route == Route::Models {
        lines.push("Type to filter · arrows/Ctrl-J/Ctrl-K select")?;
        lines.push("Enter opens child configuration · Esc returns")?;
        lines.push("Ctrl-R refreshes models · Ctrl-X restores parent")?;
        return lines.push("");
    }
    lines.push(if matches!(view.route, Route::Form(_)) {
        "Tab/arrows fields · Space toggles · Ctrl-R refresh target"
    } else if matches!(view.route, Route::Processes(_)) {
        "/refresh snapshot · /back agents · /parent-processes"
    } else if view.has_next {
        "/next page · /refresh · /current /archived /all"
    } else {
        "/refresh · /current /archived /all"
    })?;
    lines.push(if matches!(view.route, Route::Form(_)) {
        "Enter submits displayed form · Esc discards/back · Ctrl-X closes"
    } else if matches!(view.route, Route::Processes(_)) {
        "Read-only · arrows scroll · Ctrl-X exits"
    } else if view.route == Route::Conversation {
        "Enter sends · arrows/PageUp/PageDown scroll · Ctrl-X exits"
    } else {
        "Enter opens/sends · /status /messages /tools /close · Ctrl-X exits"
    })?;
    lines.push(if matches!(view.route, Route::Form(_)) {
        "Values are intent only; native admission enforces permission policy"
    } else if matches!(view.route, Route::Processes(_)) {
        "/agent-processes uses the selected resident agent"
    } else {
        "Arrows select/scroll · /create /configure /processes"
    })?;
    lines.push("")
}

fn catalog(
    lines: &mut Lines,
    view: &NativeManagedNavigationView<'_>,
    limit: usize,
) -> Result<(), ()> {
    if let Some(target) = view.target {
        target_heading(lines, target)?;
    }
    if let Some(result) = view.result {
        lines.push(&format!(
            "{:?} · /refresh to reload the catalog",
            result.status
        ))?;
    }
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

fn identity_rows(id: &str, columns: u16) -> Option<usize> {
    if id.is_empty()
        || id.len() > 255
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return None;
    }
    let width = usize::from(columns.checked_sub(1)?);
    (width != 0).then(|| (id.len() + 4).div_ceil(width))
}

fn write_identity(lines: &mut Lines, id: &str) -> Result<(), ()> {
    identity_rows(id, lines.columns).ok_or(())?;
    let text = format!("id: {id}");
    for chunk in text.as_bytes().chunks(usize::from(lines.columns - 1)) {
        lines.push(std::str::from_utf8(chunk).map_err(|_| ())?)?;
    }
    Ok(())
}

fn target_heading(
    lines: &mut Lines,
    target: &machine_god_native::NativeManagedCatalogEntry,
) -> Result<(), ()> {
    lines.push(&format!(
        "generation {} · {}",
        target.generation,
        prefix(&target.name)
    ))?;
    write_identity(lines, &target.id)
}
struct Lines {
    bytes: Vec<u8>,
    columns: u16,
    limit: usize,
    count: usize,
}
impl Lines {
    fn push(&mut self, text: &str) -> Result<(), ()> {
        let row = super::super::composer_view::label(text, self.columns, 512);
        self.push_rendered(&row)
    }
    fn push_rendered(&mut self, row: &[u8]) -> Result<(), ()> {
        if self.count == self.limit {
            return Err(());
        }
        if self.bytes.len() + row.len() + 2 > 64 * 1024 {
            return Err(());
        }
        if self.count != 0 {
            self.bytes.extend_from_slice(b"\r\n");
        }
        self.bytes.extend_from_slice(row);
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
