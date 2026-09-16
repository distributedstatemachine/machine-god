use super::Lines;
use machine_god_native::{NativeManagedModelsView, NativeModelCatalogCacheState as State};

pub(super) fn render(
    lines: &mut Lines,
    models: &NativeManagedModelsView<'_>,
    limit: usize,
) -> Result<bool, ()> {
    lines.push(match models.state {
        State::Loading => "Loading models… · Ctrl-X returns while loading",
        State::Ready => "Child model selection (does not change the parent)",
        State::Failed => "Model catalog failed · Ctrl-R retries",
        State::Idle => "Model catalog unavailable",
    })?;
    let picker = &models.picker;
    if let Some(entry) = picker.selected.and_then(|index| picker.rows().nth(index))
        && !selected_identity(lines, entry.model().id(), limit)?
    {
        return Ok(false);
    }
    let capacity = limit.saturating_sub(lines.count);
    let count = picker.rows().len();
    if count == 0 && capacity > 0 {
        lines.push("No models match the query")?;
    }
    let start = picker
        .selected
        .unwrap_or(0)
        .saturating_sub(capacity / 2)
        .min(count.saturating_sub(capacity));
    for (index, entry) in picker.rows().enumerate().skip(start).take(capacity) {
        lines.push(&format!(
            "{} {}",
            if picker.selected == Some(index) {
                ">"
            } else {
                " "
            },
            entry.model().id()
        ))?;
    }
    Ok(true)
}

fn selected_identity(lines: &mut Lines, id: &str, limit: usize) -> Result<bool, ()> {
    // Model IDs are opaque UTF-8. A complete escaped ASCII spelling avoids
    // splitting Unicode or hiding an identity suffix in a clipped preview.
    let text = format!("model (escaped): \"{}\"", id.escape_default());
    let width = usize::from(lines.columns - 1);
    if lines.count + text.len().div_ceil(width) > limit {
        lines.push("Resize to display the complete model identity")?;
        return Ok(false);
    }
    for chunk in text.as_bytes().chunks(width) {
        lines.push_rendered(chunk)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_escaped_unicode_identity_must_fit_before_selection_is_acknowledged() {
        let id = format!("vendor/{}suffix", "α🙂".repeat(12));
        let escaped = format!("model (escaped): \"{}\"", id.escape_default());
        let mut lines = Lines {
            bytes: Vec::new(),
            columns: 40,
            limit: 64,
            count: 0,
        };
        assert!(!selected_identity(&mut lines, &id, 2).unwrap());
        assert!(!String::from_utf8_lossy(&lines.bytes).contains("suffix"));
        lines.bytes.clear();
        lines.count = 0;
        assert!(selected_identity(&mut lines, &id, 64).unwrap());
        let text = String::from_utf8(lines.bytes).unwrap();
        for chunk in escaped.as_bytes().chunks(39) {
            assert!(text.contains(std::str::from_utf8(chunk).unwrap()));
        }
    }
}
