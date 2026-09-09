//! Pure, bounded single-row projection; the original draft remains authoritative.

use machine_god_native::native_terminal_display_unit_at;
use std::collections::VecDeque;

const MAX_INPUT_BYTES: usize = 256 * 1024;
const MAX_OUTPUT_BYTES: usize = 4096;
const PAYLOAD_BYTES: usize = 4000;
const MAX_CLUSTER_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComposerViewError {
    InputLimit,
    InvalidCursor,
    InvalidColumns,
}

impl std::fmt::Display for ComposerViewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("composer view unavailable")
    }
}

impl std::error::Error for ComposerViewError {}

pub(super) const fn clear_row() -> &'static [u8] {
    b"\r\x1b[2K"
}

/// Untrusted menu labels use the same pinned width/control projection as the
/// composer, without its prompt or cursor movement. Byte and cell bounds are
/// independent, including oversized zero-width continuation clusters.
pub(super) fn label(text: &str, columns: u16, max_bytes: usize) -> Vec<u8> {
    let capacity = usize::from(columns.saturating_sub(1));
    let mut output = Vec::new();
    let mut next = 0;
    let mut cells = 0;
    while next < text.len() && cells < capacity {
        let atom = Atom::at(text, next, capacity);
        if cells + atom.cells > capacity || output.len() + atom.bytes > max_bytes {
            break;
        }
        atom.append(text, &mut output);
        cells += atom.cells;
        next = atom.end;
    }
    output
}

/// Render at most `columns - 1` cells, reserving the final column against wrap.
/// The terminal cursor may occupy that reserved blank column. A scalar cursor
/// inside a native indivisible emoji unit maps to its leading cell; a cursor
/// among attached combining marks maps after their base. Neither changes the
/// caller's byte cursor. Tiny terminals clip the static prompt, never wrap it.
/// Oversized clusters or escapes wider than the entire viewport use `…`.
pub(super) fn render(
    text: &str,
    cursor: usize,
    columns: u16,
) -> Result<Vec<u8>, ComposerViewError> {
    if text.len() > MAX_INPUT_BYTES {
        return Err(ComposerViewError::InputLimit);
    }
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return Err(ComposerViewError::InvalidCursor);
    }
    let cells = usize::from(columns)
        .checked_sub(1)
        .ok_or(ComposerViewError::InvalidColumns)?;
    let prompt = &b"> "[..cells.min(2)];
    let capacity = cells.saturating_sub(prompt.len()).min(PAYLOAD_BYTES);
    let mut output = Vec::with_capacity(MAX_OUTPUT_BYTES);
    output.extend_from_slice(clear_row());
    output.extend_from_slice(prompt);
    let (left, mut next) = left_view(text, cursor, capacity);
    let cursor_column = prompt.len() + left.cells + 1;
    let mut used_cells = left.cells;
    let mut used_bytes = left.bytes;
    for atom in left.atoms {
        atom.append(text, &mut output);
    }
    while capacity != 0 && next < text.len() {
        let atom = Atom::at(text, next, capacity);
        if used_cells + atom.cells > capacity || used_bytes + atom.bytes > PAYLOAD_BYTES {
            break;
        }
        atom.append(text, &mut output);
        used_cells += atom.cells;
        used_bytes += atom.bytes;
        next = atom.end;
    }
    // Absolute horizontal positioning does not depend on prior cursor state.
    let position = format!("\x1b[{cursor_column}G");
    output.extend_from_slice(position.as_bytes());
    debug_assert!(output.len() <= MAX_OUTPUT_BYTES);
    debug_assert!(cursor_column <= usize::from(columns));
    Ok(output)
}

#[derive(Default)]
struct LeftView {
    atoms: VecDeque<Atom>,
    cells: usize,
    bytes: usize,
}

impl LeftView {
    fn pop_front(&mut self) {
        if let Some(atom) = self.atoms.pop_front() {
            self.cells -= atom.cells;
            self.bytes -= atom.bytes;
        }
    }

    fn push(&mut self, atom: Atom, cells: usize) {
        self.cells += atom.cells;
        self.bytes += atom.bytes;
        self.atoms.push_back(atom);
        while self.cells > cells || self.bytes > PAYLOAD_BYTES / 2 {
            self.pop_front();
        }
    }
}

fn left_view(text: &str, cursor: usize, capacity: usize) -> (LeftView, usize) {
    let mut left = LeftView::default();
    if capacity == 0 {
        return (left, text.len());
    }
    let left_cells = if cursor == text.len() {
        capacity
    } else {
        capacity / 2
    };
    let mut next = 0;
    while next < cursor {
        let atom = Atom::at(text, next, capacity);
        if cursor < atom.unit_end {
            break;
        }
        next = atom.end;
        left.push(atom, left_cells);
    }
    if next < text.len() {
        let first = Atom::at(text, next, capacity);
        while !left.atoms.is_empty()
            && (left.cells + first.cells > capacity || left.bytes + first.bytes > PAYLOAD_BYTES)
        {
            left.pop_front();
        }
    }
    (left, next)
}

#[derive(Clone, Copy)]
enum Representation {
    Raw,
    Escape(char),
    Marker,
}

#[derive(Clone, Copy)]
struct Atom {
    start: usize,
    end: usize,
    unit_end: usize,
    cells: usize,
    bytes: usize,
    representation: Representation,
}

impl Atom {
    fn at(text: &str, start: usize, capacity: usize) -> Self {
        let character = text[start..].chars().next().expect("nonempty UTF-8 atom");
        let unit = native_terminal_display_unit_at(text, start).expect("valid UTF-8 boundary");
        let mut atom = Self {
            start,
            end: start + unit.byte_len,
            unit_end: start + unit.byte_len,
            cells: usize::from(unit.cell_width),
            bytes: unit.byte_len,
            representation: Representation::Raw,
        };
        if must_escape(character) || unit.cell_width == 0 {
            let escaped = escape(character);
            atom.end = start + character.len_utf8();
            atom.unit_end = atom.end;
            atom.cells = escaped.len;
            atom.bytes = escaped.len;
            atom.representation = Representation::Escape(character);
        } else {
            // Keep zero-width combining continuations attached to their base.
            // Structural formats survive only when already inside the exact
            // multi-scalar unit recognized by the pinned native algorithm.
            while atom.end < text.len() {
                let following = text[atom.end..].chars().next().expect("nonempty suffix");
                let continuation = native_terminal_display_unit_at(text, atom.end)
                    .expect("valid continuation boundary");
                if continuation.cell_width != 0 || must_escape(following) {
                    break;
                }
                atom.end += continuation.byte_len;
            }
            atom.bytes = atom.end - atom.start;
        }
        if atom.cells > capacity || atom.bytes > MAX_CLUSTER_BYTES {
            atom.cells = 1;
            atom.bytes = "…".len();
            atom.representation = Representation::Marker;
        }
        atom
    }

    fn append(self, text: &str, output: &mut Vec<u8>) {
        match self.representation {
            Representation::Raw => output.extend_from_slice(&text.as_bytes()[self.start..self.end]),
            Representation::Escape(character) => {
                output.extend_from_slice(escape(character).as_bytes());
            }
            Representation::Marker => output.extend_from_slice("…".as_bytes()),
        }
    }
}

struct Escaped {
    buffer: [u8; 10],
    len: usize,
}

impl Escaped {
    fn as_bytes(&self) -> &[u8] {
        &self.buffer[..self.len]
    }
}

fn escape(character: char) -> Escaped {
    let mut result = Escaped {
        buffer: [0; 10],
        len: 2,
    };
    result.buffer[0] = b'\\';
    let short = match character {
        '\n' => Some(b'n'),
        '\r' => Some(b'r'),
        '\t' => Some(b't'),
        '\\' => Some(b'\\'),
        _ => None,
    };
    if let Some(short) = short {
        result.buffer[1] = short;
    } else {
        result.buffer[..3].copy_from_slice(b"\\u{");
        let value = u32::from(character);
        let digits = ((32 - value.leading_zeros()).max(1)).div_ceil(4);
        for index in (0..digits).rev() {
            let nibble = usize::try_from((value >> (index * 4)) & 15).expect("hex digit");
            result.buffer[3 + usize::try_from(digits - index - 1).expect("six digits")] =
                b"0123456789abcdef"[nibble];
        }
        result.len = 4 + usize::try_from(digits).expect("six digits");
        result.buffer[result.len - 1] = b'}';
    }
    result
}

fn must_escape(character: char) -> bool {
    character.is_control()
        || matches!(character,
            '\\' | '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}'
            | '\u{70f}' | '\u{890}'..='\u{891}' | '\u{8e2}' | '\u{180e}'
            | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}' | '\u{fe00}'..='\u{fe0f}' | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}'
            | '\u{13430}'..='\u{13455}' | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}'
            | '\u{e0100}'..='\u{e01ef}')
}

#[cfg(test)]
#[path = "composer_view/tests.rs"]
mod tests;
