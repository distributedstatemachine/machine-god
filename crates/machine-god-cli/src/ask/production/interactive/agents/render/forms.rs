use super::{Lines, prefix, target_heading};
use machine_god_native::{NativeManagedFormKind, NativeManagedNavigationView};

pub(super) fn render(
    lines: &mut Lines,
    view: &NativeManagedNavigationView<'_>,
    limit: usize,
) -> Result<(), ()> {
    let Some(form) = &view.form else {
        return lines.push("Loading exact configuration…");
    };
    if form.kind == NativeManagedFormKind::Configure {
        let target = view.target.ok_or(())?;
        target_heading(lines, target)?;
    } else {
        lines.push("Create an independent agent")?;
    }
    if let Some(error) = view.result.and_then(|result| result.error_code) {
        lines.push(&format!("Rejected: {error:?}; Ctrl-R refreshes the target"))?;
    }
    let capacity = limit.saturating_sub(lines.count);
    if capacity == 0 {
        return Err(());
    }
    let start = form
        .selected
        .saturating_sub(capacity / 2)
        .min(form.fields.len().saturating_sub(capacity));
    for (index, field) in form.fields.iter().enumerate().skip(start).take(capacity) {
        lines.push(&format!(
            "{} {}: {}",
            if form.selected == index { ">" } else { " " },
            field.label(),
            prefix(form.values[index])
        ))?;
    }
    Ok(())
}
