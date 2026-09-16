use super::Lines;
use machine_god_native::NativeSkillPickerView;

pub(super) fn render(
    lines: &mut Lines,
    view: &NativeSkillPickerView<'_>,
    limit: usize,
) -> Result<bool, ()> {
    let selected = view
        .selected
        .and_then(|index| index.checked_sub(view.window_start))
        .and_then(|index| view.rows.get(index));
    if let Some(entry) = selected {
        let Some(path) = entry.location().to_str() else {
            lines.push("Skill location cannot be displayed exactly")?;
            return Ok(false);
        };
        let identity = format!(
            "name=\"{}\" location=\"{}\"",
            entry.metadata.name.escape_default(),
            path.escape_default()
        );
        let width = usize::from(lines.columns - 1);
        if lines.count + identity.len().div_ceil(width) > limit {
            lines.push("Resize to display the complete skill identity")?;
            return Ok(false);
        }
        for chunk in identity.as_bytes().chunks(width) {
            lines.push_rendered(chunk)?;
        }
    }
    let capacity = limit.saturating_sub(lines.count);
    if view.total_matches == 0 && capacity > 0 {
        lines.push("No skills match the query")?;
    }
    let focused = view
        .selected
        .unwrap_or(view.window_start)
        .saturating_sub(view.window_start);
    let start = focused
        .saturating_sub(capacity / 2)
        .min(view.rows.len().saturating_sub(capacity));
    for (index, entry) in view.rows.iter().enumerate().skip(start).take(capacity) {
        lines.push(&format!(
            "{} {}",
            if view.selected == Some(view.window_start + index) {
                ">"
            } else {
                " "
            },
            entry.metadata.name
        ))?;
    }
    Ok(true)
}
