//! Retained bounded process snapshots. Rendering never resolves or controls a PID.
use super::{Lines, prefix};
use machine_god_native::{NativeManagedNavigationView, NativeTerminalBackgroundSnapshot};

pub(in super::super) fn count(snapshot: Option<&NativeTerminalBackgroundSnapshot>) -> usize {
    snapshot.map_or(0, |snapshot| snapshot.entries().len())
}

pub(super) fn render(
    lines: &mut Lines,
    view: &NativeManagedNavigationView<'_>,
    limit: usize,
    offset: usize,
) -> Result<(), ()> {
    let Some(snapshot) = view.processes else {
        return lines.push(if view.busy {
            "Reading process snapshot…"
        } else {
            "Process snapshot unavailable"
        });
    };
    let capacity = limit.saturating_sub(lines.count);
    let entries = snapshot.entries();
    for entry in entries.iter().skip(offset).take(capacity.saturating_sub(1)) {
        lines.push(&format!(
            "{} {:?} [{}] {}",
            entry.id().as_str(),
            entry.facts().lifecycle,
            if entry.owns_backend() {
                "owned"
            } else {
                "history only"
            },
            prefix(entry.command().unwrap_or("(interactive shell)")),
        ))?;
    }
    if capacity > 0 {
        let end = offset
            .saturating_add(capacity.saturating_sub(1))
            .min(entries.len());
        lines.push(&format!(
            "Processes {}–{} of {} · snapshot, not live status",
            offset.saturating_add(1).min(entries.len()),
            end,
            entries.len(),
        ))?;
    }
    Ok(())
}
