//! Effect-free projection of native picker state into one bounded menu frame.
//!
//! The caller clears the previous menu and owns successful output acknowledgement.
//! Only a frame with `selection_visible` may acknowledge a selectable native row.
//! Resize or any other presentation change must invalidate the native frame.

use crate::bounded_output::BoundedOutput;
use machine_god_native::{
    MAX_NATIVE_SKILL_CANDIDATES, MAX_NATIVE_SKILL_DESCRIPTION_BYTES,
    MAX_NATIVE_SKILL_METADATA_NAME_BYTES, MAX_NATIVE_SKILL_PATH_BYTES,
    MAX_NATIVE_SKILL_QUERY_BYTES, MAX_NATIVE_SKILL_QUERY_ROWS, NativeSkillFrameIdentity,
    NativeSkillPickerView,
};
use std::fmt::{self, Write as _};

const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_PHYSICAL_ROWS: usize = 128;
const ROW_BYTES: usize = 480;
const MIN_SELECT_COLUMNS: u16 = 16;

#[cfg(test)]
#[path = "skills_view/tests.rs"]
mod tests;

pub(super) struct Frame {
    pub bytes: Vec<u8>,
    /// CRLF count below the start anchor; no trailing newline is emitted.
    pub height: u16,
    pub identity: NativeSkillFrameIdentity,
    /// True only when the exact absolute row number and a location preview are
    /// presented. This does not claim that clipped names/paths are lossless.
    /// False for empty results or terminals too small for the normal layout.
    pub selection_visible: bool,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkillsFrame")
            .field("bytes", &self.bytes.len())
            .field("height", &self.height)
            .field("selection_visible", &self.selection_visible)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RenderError {
    InvalidDimensions,
    InvalidView,
    OutputLimit,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("skills view unavailable")
    }
}
impl std::error::Error for RenderError {}

pub(super) fn render(
    view: &NativeSkillPickerView<'_>,
    columns: u16,
    rows: u16,
) -> Result<Frame, RenderError> {
    if columns <= 1 || rows == 0 {
        return Err(RenderError::InvalidDimensions);
    }
    validate(view)?;
    let rows = usize::from(rows).min(MAX_PHYSICAL_ROWS);
    let warning_rows = usize::from(view.discovery_incomplete);
    let mut output = Lines::new(columns);
    if view.discovery_incomplete {
        output.push("! Incomplete discovery; other skills may be unavailable")?;
    }
    if columns < MIN_SELECT_COLUMNS || rows < 5 + warning_rows {
        if output.count < rows {
            output.push("Resize terminal to select skills (Esc closes)")?;
        }
        return output.finish(view, false);
    }

    // Header, query and footer are independent of the two lines per entry.
    // Slide inside the native window; never silently show the first 128 matches
    // while acknowledging a selection somewhere after that window.
    let capacity = (rows - 3 - warning_rows) / 2;
    let selected_local = view.selected.map(|index| index - view.window_start);
    let start = selected_local
        .unwrap_or(0)
        .saturating_sub(capacity / 2)
        .min(view.rows.len().saturating_sub(capacity));
    let end = (start + capacity).min(view.rows.len());
    let ordinal = view.selected.map_or(0, |index| index + 1);
    if view.rows.is_empty() {
        output.push("0/0 Skills")?;
    } else {
        output.push(&format!(
            "{ordinal}/{} Skills · rows {}-{}",
            view.total_matches,
            view.window_start + start + 1,
            view.window_start + end
        ))?;
    }
    output.push(&format!("Search: {}", view.query))?;
    let mut selection_visible = false;
    for (local, entry) in view.rows.iter().enumerate().take(end).skip(start) {
        let position = view.window_start + local;
        output.push(&format!(
            "{} {}. {} — {}",
            if view.selected == Some(position) {
                ">"
            } else {
                " "
            },
            position + 1,
            entry.metadata.name,
            entry.metadata.description
        ))?;
        // A separate preview prevents a long description erasing location.
        // Basename first also exposes distinguishing child names when a common
        // absolute path prefix is wider than the terminal. The row number
        // remains the unique view identity; this is not a lossless path report.
        let basename = entry
            .location()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(RenderError::InvalidView)?;
        let preview_bytes = output.push(&format!(
            "@ {basename} | {}",
            entry.location().to_str().ok_or(RenderError::InvalidView)?
        ))?;
        if view.selected == Some(position) {
            // An indivisible projected cluster may exceed the byte budget even
            // if it fits in cells. Never acknowledge a bare location prefix.
            selection_visible = preview_bytes > b"@ ".len();
        }
    }
    if view.rows.is_empty() {
        output.push("No matching observed skills")?;
    }
    output.push("↑↓ move · Enter select · Esc close · clipped previews")?;
    output.finish(view, selection_visible)
}

fn validate(view: &NativeSkillPickerView<'_>) -> Result<(), RenderError> {
    if view.query.len() > MAX_NATIVE_SKILL_QUERY_BYTES
        || view.rows.len() > MAX_NATIVE_SKILL_QUERY_ROWS
        || view.total_matches > MAX_NATIVE_SKILL_CANDIDATES
    {
        return Err(RenderError::InvalidView);
    }
    let end = view
        .window_start
        .checked_add(view.rows.len())
        .ok_or(RenderError::InvalidView)?;
    if end > view.total_matches {
        return Err(RenderError::InvalidView);
    }
    if view.total_matches == 0 {
        if view.selected.is_some() || view.window_start != 0 {
            return Err(RenderError::InvalidView);
        }
    } else if !view
        .selected
        .is_some_and(|selected| view.window_start <= selected && selected < end)
    {
        return Err(RenderError::InvalidView);
    }
    for entry in &view.rows {
        if entry.metadata.name.len() > MAX_NATIVE_SKILL_METADATA_NAME_BYTES
            || entry.metadata.description.len() > MAX_NATIVE_SKILL_DESCRIPTION_BYTES
            || entry.location().as_os_str().len() > MAX_NATIVE_SKILL_PATH_BYTES
            || entry.location().to_str().is_none()
        {
            return Err(RenderError::InvalidView);
        }
    }
    Ok(())
}

struct Lines {
    output: BoundedOutput,
    columns: u16,
    count: usize,
}

impl Lines {
    fn new(columns: u16) -> Self {
        Self {
            output: BoundedOutput::with_capacity(MAX_OUTPUT_BYTES, 4096),
            columns,
            count: 0,
        }
    }
    fn push(&mut self, label: &str) -> Result<usize, RenderError> {
        let projected = super::composer_view::label(label, self.columns, ROW_BYTES);
        let projected = std::str::from_utf8(&projected).map_err(|_| RenderError::OutputLimit)?;
        if self.count != 0 {
            self.output
                .write_str("\r\n")
                .map_err(|_| RenderError::OutputLimit)?;
        }
        self.output
            .write_str(projected)
            .map_err(|_| RenderError::OutputLimit)?;
        self.count += 1;
        Ok(projected.len())
    }
    fn finish(
        self,
        view: &NativeSkillPickerView<'_>,
        selection_visible: bool,
    ) -> Result<Frame, RenderError> {
        Ok(Frame {
            bytes: self.output.finish().into_bytes(),
            height: u16::try_from(self.count.saturating_sub(1))
                .map_err(|_| RenderError::OutputLimit)?,
            identity: view.identity.clone(),
            selection_visible,
        })
    }
}
