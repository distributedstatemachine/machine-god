// SPDX-License-Identifier: Apache-2.0
//
// Safe, replay-only Rust transliteration of vercel-labs/fx at revision
// b1774fbf6c7602b503026f96f6e960e946c692ef:
// src/core/terminal/engine.zig. Presentation styles, hyperlinks, protocol
// replies, checkpoints, and diffs are intentionally omitted from this private
// projection; their control sequences are still consumed without leaking.

use super::terminal_display_width::{decode_next_rune, display_unit_at, utf8_sequence_len};
use std::collections::HashMap;
use std::sync::Arc;

const MAX_DIMENSION: u16 = 4096;
const MAX_RENDER_BYTES: usize = 8 * 1024 * 1024;
const PINNED_RENDER_CELL_BYTES: usize = 32;
const MAX_CELLS: usize = MAX_RENDER_BYTES / PINNED_RENDER_CELL_BYTES;
const MAX_CSI_PARAMS: usize = 16;
const MAX_CSI_INTERMEDIATES: usize = 2;
const MAX_CONTROL_STRING_BYTES: usize = 4096;
const MAX_SYNC_BYTES: usize = 1024 * 1024;
const MAX_SUFFIX_POOL_BYTES: usize = 4 * 1024 * 1024;
const MAX_SUFFIX_ENTRIES: usize = 65_535;
const MAX_CELL_TEXT_BYTES: usize = 64;
const FEED_CANCELLATION_CHECKPOINT_BYTES: usize = 16 * 1024;
const SYNC_RESET: &[u8] = b"\x1b[?2026l";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalGridError {
    InvalidGridSize,
    GridTooLarge,
    TooManyCsiParameters,
    TooManyCsiIntermediates,
    ControlStringTooLarge,
    SynchronizedUpdateTooLarge,
    CombiningPoolCapacityExceeded,
    SnapshotTooLarge,
}

impl std::fmt::Display for TerminalGridError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidGridSize => "invalid terminal grid size",
            Self::GridTooLarge => "terminal grid exceeds the cell limit",
            Self::TooManyCsiParameters => "terminal CSI parameter limit exceeded",
            Self::TooManyCsiIntermediates => "terminal CSI intermediate limit exceeded",
            Self::ControlStringTooLarge => "terminal control string limit exceeded",
            Self::SynchronizedUpdateTooLarge => "terminal synchronized update limit exceeded",
            Self::CombiningPoolCapacityExceeded => "terminal combining suffix limit exceeded",
            Self::SnapshotTooLarge => "terminal snapshot limit exceeded",
        })
    }
}

impl std::error::Error for TerminalGridError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalGridFeedError {
    Grid(TerminalGridError),
    Cancelled,
}

impl From<TerminalGridError> for TerminalGridFeedError {
    fn from(error: TerminalGridError) -> Self {
        Self::Grid(error)
    }
}

#[derive(Default)]
struct FeedCheckpoint {
    bytes_since_check: usize,
}

impl FeedCheckpoint {
    fn consume(
        &mut self,
        byte_count: usize,
        is_cancelled: &mut impl FnMut() -> bool,
    ) -> Result<(), TerminalGridFeedError> {
        self.bytes_since_check += byte_count;
        if self.bytes_since_check < FEED_CANCELLATION_CHECKPOINT_BYTES {
            return Ok(());
        }
        self.bytes_since_check %= FEED_CANCELLATION_CHECKPOINT_BYTES;
        if is_cancelled() {
            return Err(TerminalGridFeedError::Cancelled);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Cell {
    codepoint: u32,
    width: u8,
    suffix_id: u32,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            codepoint: u32::from(' '),
            width: 1,
            suffix_id: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct SavedCursor {
    row: u16,
    col: u16,
    pending_wrap: bool,
    origin_mode: bool,
}

// These independent booleans are the terminal modes serialized by the pinned
// state machine; packing them would obscure the one-for-one restore contract.
#[allow(clippy::struct_excessive_bools)]
struct SavedScreen {
    cells: Vec<Cell>,
    row_origin: u16,
    cursor_row: u16,
    cursor_col: u16,
    autowrap: bool,
    pending_wrap: bool,
    scroll_top: u16,
    scroll_bottom: u16,
    origin_mode: bool,
    insert_mode: bool,
    saved_cursor: Option<SavedCursor>,
    last_printable_idx: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParserState {
    Normal,
    Escape,
    Csi,
    Osc,
    Dcs,
}

// Parser flags and DEC modes intentionally remain named independent state, as
// in the pinned engine, so every transition stays explicit and reviewable.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct TerminalGrid {
    cols: u16,
    rows: u16,
    cells: Vec<Cell>,
    row_origin: u16,
    cursor_row: u16,
    cursor_col: u16,
    autowrap: bool,
    pending_wrap: bool,
    cursor_visible: bool,
    scroll_top: u16,
    scroll_bottom: u16,
    origin_mode: bool,
    insert_mode: bool,
    tab_stops: Vec<bool>,
    saved_cursor: Option<SavedCursor>,
    saved_normal_screen: Option<SavedScreen>,
    last_printable_idx: Option<usize>,
    suffix_pool: Vec<Arc<[u8]>>,
    suffix_index: HashMap<Arc<[u8]>, u32>,
    suffix_pool_bytes: usize,
    state: ParserState,
    csi_params: [u16; MAX_CSI_PARAMS],
    csi_param_count: usize,
    csi_has_digit: bool,
    csi_private: u8,
    csi_intermediates: [u8; MAX_CSI_INTERMEDIATES],
    csi_intermediate_count: usize,
    osc_saw_esc: bool,
    osc_buffer: Vec<u8>,
    dcs_saw_esc: bool,
    dcs_buffer: Vec<u8>,
    utf8_buffer: [u8; 4],
    utf8_len: usize,
    utf8_expected: usize,
    sync_active: bool,
    sync_buffer: Vec<u8>,
}

impl TerminalGrid {
    pub(crate) fn new(cols: u16, rows: u16) -> Result<Self, TerminalGridError> {
        let cell_count = checked_cell_count(cols, rows)?;
        let mut tab_stops = vec![false; usize::from(cols)];
        initialize_tab_stops(&mut tab_stops);
        Ok(Self {
            cols,
            rows,
            cells: vec![Cell::default(); cell_count],
            row_origin: 0,
            cursor_row: 1,
            cursor_col: 1,
            autowrap: true,
            pending_wrap: false,
            cursor_visible: true,
            scroll_top: 1,
            scroll_bottom: rows,
            origin_mode: false,
            insert_mode: false,
            tab_stops,
            saved_cursor: None,
            saved_normal_screen: None,
            last_printable_idx: None,
            suffix_pool: Vec::new(),
            suffix_index: HashMap::new(),
            suffix_pool_bytes: 0,
            state: ParserState::Normal,
            csi_params: [0; MAX_CSI_PARAMS],
            csi_param_count: 0,
            csi_has_digit: false,
            csi_private: 0,
            csi_intermediates: [0; MAX_CSI_INTERMEDIATES],
            csi_intermediate_count: 0,
            osc_saw_esc: false,
            osc_buffer: Vec::new(),
            dcs_saw_esc: false,
            dcs_buffer: Vec::new(),
            utf8_buffer: [0; 4],
            utf8_len: 0,
            utf8_expected: 0,
            sync_active: false,
            sync_buffer: Vec::new(),
        })
    }

    pub(crate) fn cols(&self) -> u16 {
        self.cols
    }

    pub(crate) fn rows(&self) -> u16 {
        self.rows
    }

    pub(crate) fn cursor_row(&self) -> u16 {
        self.cursor_row
    }

    pub(crate) fn cursor_col(&self) -> u16 {
        self.cursor_col
    }

    pub(crate) fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub(crate) fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalGridError> {
        let cell_count = checked_cell_count(cols, rows)?;
        if cols == self.cols && rows == self.rows {
            return Ok(());
        }
        let new_cells = resized_cells(
            &self.cells,
            self.cols,
            self.rows,
            self.row_origin,
            cols,
            rows,
            cell_count,
        );
        let saved_cells = self.saved_normal_screen.as_ref().map(|saved| {
            resized_cells(
                &saved.cells,
                self.cols,
                self.rows,
                saved.row_origin,
                cols,
                rows,
                cell_count,
            )
        });
        let old_tab_stops = std::mem::take(&mut self.tab_stops);
        let mut new_tab_stops = vec![false; usize::from(cols)];
        initialize_tab_stops(&mut new_tab_stops);
        let shared = old_tab_stops.len().min(new_tab_stops.len());
        new_tab_stops[..shared].copy_from_slice(&old_tab_stops[..shared]);

        self.cells = new_cells;
        self.cols = cols;
        self.rows = rows;
        self.row_origin = 0;
        self.cursor_row = self.cursor_row.min(rows);
        self.cursor_col = self.cursor_col.min(cols);
        self.last_printable_idx = None;
        self.scroll_top = 1;
        self.scroll_bottom = rows;
        self.origin_mode = false;
        self.tab_stops = new_tab_stops;
        if let Some(saved) = self.saved_normal_screen.as_mut() {
            saved.cells = saved_cells.expect("saved cells exist with saved screen");
            saved.row_origin = 0;
            saved.cursor_row = saved.cursor_row.min(rows);
            saved.cursor_col = saved.cursor_col.min(cols);
            saved.last_printable_idx = None;
            saved.scroll_top = 1;
            saved.scroll_bottom = rows;
            saved.origin_mode = false;
        }
        Ok(())
    }

    // Retained for parser callers and focused parity tests that do not need a
    // cooperative cancellation source.
    #[allow(dead_code)]
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<(), TerminalGridError> {
        match self.feed_with_cancel_check(bytes, || false) {
            Ok(()) => Ok(()),
            Err(TerminalGridFeedError::Grid(error)) => Err(error),
            Err(TerminalGridFeedError::Cancelled) => {
                unreachable!("an inert cancellation check cannot cancel")
            }
        }
    }

    pub(crate) fn feed_with_cancel_check(
        &mut self,
        bytes: &[u8],
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<(), TerminalGridFeedError> {
        let mut index = 0;
        let mut checkpoint = FeedCheckpoint::default();
        while index < bytes.len() {
            if self.sync_active {
                if self.sync_buffer.len() >= MAX_SYNC_BYTES {
                    return Err(TerminalGridError::SynchronizedUpdateTooLarge.into());
                }
                self.sync_buffer.push(bytes[index]);
                index += 1;
                if self.sync_buffer.ends_with(SYNC_RESET) {
                    self.sync_buffer
                        .truncate(self.sync_buffer.len() - SYNC_RESET.len());
                    let buffered = std::mem::take(&mut self.sync_buffer);
                    self.sync_active = false;
                    self.feed_direct(&buffered, false, &mut checkpoint, &mut is_cancelled)?;
                }
                checkpoint.consume(1, &mut is_cancelled)?;
                continue;
            }
            let consumed =
                self.feed_direct(&bytes[index..], true, &mut checkpoint, &mut is_cancelled)?;
            index += consumed;
        }
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Result<Vec<u8>, TerminalGridError> {
        let structural = usize::from(self.rows)
            .checked_mul(3)
            .and_then(|value| value.checked_add(self.cells.len()))
            .ok_or(TerminalGridError::SnapshotTooLarge)?;
        if structural > MAX_RENDER_BYTES {
            return Err(TerminalGridError::SnapshotTooLarge);
        }
        let mut output = Vec::with_capacity(structural);
        for row in 1..=self.rows {
            push_bounded(&mut output, b'|')?;
            let base = self.row_base(row);
            for col in 0..usize::from(self.cols) {
                let cell = self.cells[base + col];
                if cell.width == 0 {
                    continue;
                }
                let codepoint = char::from_u32(cell.codepoint).unwrap_or(' ');
                let mut encoded = [0; 4];
                push_slice_bounded(&mut output, codepoint.encode_utf8(&mut encoded).as_bytes())?;
                if let Some(suffix) = self.suffix(cell.suffix_id) {
                    push_slice_bounded(&mut output, suffix)?;
                }
            }
            push_slice_bounded(&mut output, b"|\n")?;
        }
        Ok(output)
    }

    // Keeping the five parser states in one dispatch loop makes byte
    // consumption and cancellation atomic and directly auditable against fx.
    #[allow(clippy::too_many_lines)]
    fn feed_direct(
        &mut self,
        bytes: &[u8],
        stop_on_sync_start: bool,
        checkpoint: &mut FeedCheckpoint,
        is_cancelled: &mut impl FnMut() -> bool,
    ) -> Result<usize, TerminalGridFeedError> {
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == 0x18 || byte == 0x1a {
                self.cancel_control_sequence();
                index += 1;
                checkpoint.consume(1, is_cancelled)?;
                continue;
            }
            match self.state {
                ParserState::Normal => {
                    if self.utf8_len != 0 {
                        let consumed = self.complete_pending_utf8(&bytes[index..])?;
                        index += consumed;
                        checkpoint.consume(consumed, is_cancelled)?;
                        continue;
                    }
                    if byte == 0x1b {
                        self.last_printable_idx = None;
                        self.state = ParserState::Escape;
                        index += 1;
                        checkpoint.consume(1, is_cancelled)?;
                        continue;
                    }
                    let expected = utf8_sequence_len(byte).unwrap_or(1);
                    if expected > 1 && index + expected > bytes.len() {
                        let tail = &bytes[index..];
                        self.utf8_buffer[..tail.len()].copy_from_slice(tail);
                        self.utf8_len = tail.len();
                        self.utf8_expected = expected;
                        checkpoint.consume(tail.len(), is_cancelled)?;
                        return Ok(bytes.len());
                    }
                    let consumed = self.write_unit(bytes, index)?;
                    index += consumed;
                    checkpoint.consume(consumed, is_cancelled)?;
                }
                ParserState::Escape => {
                    self.dispatch_escape(byte);
                    index += 1;
                    checkpoint.consume(1, is_cancelled)?;
                }
                ParserState::Csi => {
                    if matches!(byte, b'?' | b'>' | b'<' | b'=')
                        && self.csi_param_count == 0
                        && !self.csi_has_digit
                        && self.csi_intermediate_count == 0
                    {
                        if self.csi_private == 0 {
                            self.csi_private = byte;
                        }
                        index += 1;
                        checkpoint.consume(1, is_cancelled)?;
                        continue;
                    }
                    if byte.is_ascii_digit() {
                        let slot = &mut self.csi_params[self.csi_param_count];
                        *slot = slot
                            .saturating_mul(10)
                            .saturating_add(u16::from(byte - b'0'));
                        self.csi_has_digit = true;
                        index += 1;
                        checkpoint.consume(1, is_cancelled)?;
                        continue;
                    }
                    if byte == b';' || byte == b':' {
                        if self.csi_param_count + 1 >= MAX_CSI_PARAMS {
                            return Err(TerminalGridError::TooManyCsiParameters.into());
                        }
                        self.csi_param_count += 1;
                        self.csi_has_digit = false;
                        index += 1;
                        checkpoint.consume(1, is_cancelled)?;
                        continue;
                    }
                    if (0x20..=0x2f).contains(&byte) {
                        if self.csi_intermediate_count >= MAX_CSI_INTERMEDIATES {
                            return Err(TerminalGridError::TooManyCsiIntermediates.into());
                        }
                        self.csi_intermediates[self.csi_intermediate_count] = byte;
                        self.csi_intermediate_count += 1;
                        index += 1;
                        checkpoint.consume(1, is_cancelled)?;
                        continue;
                    }
                    if !(0x40..=0x7e).contains(&byte) {
                        self.cancel_control_sequence();
                        index += 1;
                        checkpoint.consume(1, is_cancelled)?;
                        continue;
                    }
                    if self.csi_has_digit || self.csi_param_count > 0 {
                        self.csi_param_count += 1;
                    }
                    self.dispatch_csi(byte);
                    self.state = ParserState::Normal;
                    index += 1;
                    checkpoint.consume(1, is_cancelled)?;
                    if stop_on_sync_start && self.sync_active {
                        return Ok(index);
                    }
                }
                ParserState::Osc => {
                    if byte == 0x07 {
                        self.cancel_control_sequence();
                    } else if byte == 0x1b {
                        self.osc_saw_esc = true;
                    } else if self.osc_saw_esc && byte == b'\\' {
                        self.cancel_control_sequence();
                    } else {
                        if self.osc_saw_esc {
                            append_control_byte(&mut self.osc_buffer, 0x1b)?;
                            self.osc_saw_esc = false;
                        }
                        append_control_byte(&mut self.osc_buffer, byte)?;
                    }
                    index += 1;
                    checkpoint.consume(1, is_cancelled)?;
                }
                ParserState::Dcs => {
                    if byte == 0x1b {
                        self.dcs_saw_esc = true;
                    } else if self.dcs_saw_esc && byte == b'\\' {
                        self.cancel_control_sequence();
                    } else {
                        if self.dcs_saw_esc {
                            append_control_byte(&mut self.dcs_buffer, 0x1b)?;
                            self.dcs_saw_esc = false;
                        }
                        append_control_byte(&mut self.dcs_buffer, byte)?;
                    }
                    index += 1;
                    checkpoint.consume(1, is_cancelled)?;
                }
            }
        }
        Ok(index)
    }

    fn dispatch_escape(&mut self, byte: u8) {
        match byte {
            b'[' => {
                self.reset_csi();
                self.state = ParserState::Csi;
            }
            b']' => {
                self.osc_saw_esc = false;
                self.osc_buffer.clear();
                self.state = ParserState::Osc;
            }
            b'P' => {
                self.dcs_saw_esc = false;
                self.dcs_buffer.clear();
                self.state = ParserState::Dcs;
            }
            b'7' => {
                self.save_cursor();
                self.state = ParserState::Normal;
            }
            b'8' => {
                self.restore_cursor();
                self.state = ParserState::Normal;
            }
            b'D' => {
                self.pending_wrap = false;
                self.advance_row_or_scroll();
                self.state = ParserState::Normal;
            }
            b'E' => {
                self.pending_wrap = false;
                self.cursor_col = 1;
                self.advance_row_or_scroll();
                self.state = ParserState::Normal;
            }
            b'M' => {
                self.pending_wrap = false;
                self.reverse_index();
                self.state = ParserState::Normal;
            }
            b'H' => {
                self.tab_stops[usize::from(self.cursor_col - 1)] = true;
                self.state = ParserState::Normal;
            }
            b'c' => {
                self.reset_terminal();
                self.state = ParserState::Normal;
            }
            _ => self.state = ParserState::Normal,
        }
    }

    fn complete_pending_utf8(&mut self, bytes: &[u8]) -> Result<usize, TerminalGridError> {
        if bytes[0] & 0xc0 != 0x80 {
            self.utf8_len = 0;
            self.utf8_expected = 0;
            self.write_unit("�".as_bytes(), 0)?;
            return Ok(0);
        }
        let needed = self.utf8_expected - self.utf8_len;
        let count = bytes.len().min(needed);
        self.utf8_buffer[self.utf8_len..self.utf8_len + count].copy_from_slice(&bytes[..count]);
        self.utf8_len += count;
        if self.utf8_len != self.utf8_expected {
            return Ok(count);
        }
        let complete = self.utf8_buffer[..self.utf8_len].to_vec();
        self.utf8_len = 0;
        self.utf8_expected = 0;
        if std::str::from_utf8(&complete).is_ok() {
            self.write_unit(&complete, 0)?;
        } else {
            self.write_unit("�".as_bytes(), 0)?;
        }
        Ok(count)
    }

    fn write_unit(&mut self, bytes: &[u8], start: usize) -> Result<usize, TerminalGridError> {
        let byte = bytes[start];
        match byte {
            b'\n' => {
                self.last_printable_idx = None;
                self.pending_wrap = false;
                self.cursor_col = 1;
                self.advance_row_or_scroll();
                return Ok(1);
            }
            b'\r' => {
                self.last_printable_idx = None;
                self.pending_wrap = false;
                self.cursor_col = 1;
                return Ok(1);
            }
            0x08 => {
                self.last_printable_idx = None;
                self.pending_wrap = false;
                self.cursor_col = self.cursor_col.saturating_sub(1).max(1);
                return Ok(1);
            }
            b'\t' => {
                self.last_printable_idx = None;
                self.pending_wrap = false;
                self.move_tabs_forward(1);
                return Ok(1);
            }
            0x00..=0x1f => {
                self.last_printable_idx = None;
                return Ok(1);
            }
            _ => {}
        }

        let unit = display_unit_at(bytes, start);
        let decoded = decode_next_rune(bytes, start);
        let consumed = unit.byte_len;
        let width = unit.cell_width;
        if width == 0 {
            if let Some(index) = self.last_printable_idx {
                self.append_suffix(index, &bytes[start..start + consumed])?;
            }
            return Ok(consumed);
        }
        self.last_printable_idx = None;
        if self.pending_wrap && self.autowrap {
            self.cursor_col = 1;
            self.advance_row_or_scroll();
        }
        self.pending_wrap = false;
        if u32::from(self.cursor_col) + u32::from(width) - 1 > u32::from(self.cols) {
            if self.autowrap {
                self.cursor_col = 1;
                self.advance_row_or_scroll();
            } else if self.cols >= u16::from(width) {
                self.cursor_col = self.cols - u16::from(width) + 1;
            } else {
                return Ok(consumed);
            }
        }
        if self.insert_mode {
            self.insert_cells(u16::from(width));
        }
        let row = self.cursor_row;
        let col = self.cursor_col;
        self.clear_wide_glyph_at(row, col);
        if width == 2 {
            self.clear_wide_glyph_at(row, col + 1);
        }
        let cell_index = self.cell_index(row, col);
        self.cells[cell_index] = Cell {
            codepoint: decoded.codepoint,
            width,
            suffix_id: 0,
        };
        if decoded.len < consumed {
            self.append_suffix(cell_index, &bytes[start + decoded.len..start + consumed])?;
        }
        self.last_printable_idx = Some(cell_index);
        if width == 2 && col < self.cols {
            self.cells[cell_index + 1] = Cell {
                codepoint: 0,
                width: 0,
                suffix_id: 0,
            };
        }
        if u32::from(col) + u32::from(width) <= u32::from(self.cols) {
            self.cursor_col += u16::from(width);
        } else {
            self.cursor_col = self.cols;
            self.pending_wrap = self.autowrap;
        }
        Ok(consumed)
    }

    fn dispatch_csi(&mut self, final_byte: u8) {
        match final_byte {
            b'H' | b'f' => self.position_cursor(self.param(0, 1), self.param(1, 1)),
            b'A' => {
                self.cursor_row = clamp_sub(self.cursor_row, self.param(0, 1), self.cursor_top());
                self.pending_wrap = false;
            }
            b'B' | b'e' => {
                self.cursor_row = clamp(
                    self.cursor_row.saturating_add(self.param(0, 1)),
                    self.cursor_top(),
                    self.cursor_bottom(),
                );
                self.pending_wrap = false;
            }
            b'C' | b'a' => {
                self.cursor_col = clamp(
                    self.cursor_col.saturating_add(self.param(0, 1)),
                    1,
                    self.cols,
                );
                self.pending_wrap = false;
            }
            b'D' => {
                self.cursor_col = clamp_sub(self.cursor_col, self.param(0, 1), 1);
                self.pending_wrap = false;
            }
            b'E' => {
                self.cursor_row = clamp(
                    self.cursor_row.saturating_add(self.param(0, 1)),
                    self.cursor_top(),
                    self.cursor_bottom(),
                );
                self.cursor_col = 1;
                self.pending_wrap = false;
            }
            b'F' => {
                self.cursor_row = clamp_sub(self.cursor_row, self.param(0, 1), self.cursor_top());
                self.cursor_col = 1;
                self.pending_wrap = false;
            }
            b'G' | b'`' => {
                self.cursor_col = clamp(self.param(0, 1), 1, self.cols);
                self.pending_wrap = false;
            }
            b'd' => {
                self.cursor_row = clamp(self.param(0, 1), 1, self.rows);
                self.pending_wrap = false;
            }
            b'J' => self.erase_display(self.param_raw(0, 0)),
            b'K' => self.erase_line(self.param_raw(0, 0)),
            b'@' => self.insert_cells(self.param(0, 1)),
            b'P' => self.delete_cells(self.param(0, 1)),
            b'X' => self.erase_cells(self.param(0, 1)),
            b'L' => self.insert_lines(self.param(0, 1)),
            b'M' => self.delete_lines(self.param(0, 1)),
            b'S' => self.scroll_up(self.scroll_top, self.scroll_bottom, self.param(0, 1)),
            b'T' => self.scroll_down(self.scroll_top, self.scroll_bottom, self.param(0, 1)),
            b'I' => self.move_tabs_forward(self.param(0, 1)),
            b'Z' => self.move_tabs_backward(self.param(0, 1)),
            b'g' => self.clear_tab_stops(self.param_raw(0, 0)),
            b'h' | b'l' => self.set_reset(final_byte == b'h'),
            b'r' => self.set_scroll_region(),
            b's' => self.save_cursor(),
            b'u' if self.csi_private == 0 => self.restore_cursor(),
            // SGR, protocol queries, cursor style, and unsupported sequences
            // are deliberately consumed with no visible plain-grid effect.
            _ => {}
        }
    }

    fn set_reset(&mut self, set: bool) {
        if self.csi_private == 0 {
            if self.csi_params[..self.csi_param_count].contains(&4) {
                self.insert_mode = set;
            }
            return;
        }
        if self.csi_private != b'?' {
            return;
        }
        let params = self.csi_params[..self.csi_param_count].to_vec();
        for param in params {
            match param {
                6 => {
                    self.origin_mode = set;
                    self.position_cursor(1, 1);
                }
                7 => self.autowrap = set,
                25 => self.cursor_visible = set,
                47 | 1047 | 1049 => {
                    if set {
                        self.enter_alternate_screen();
                    } else {
                        self.leave_alternate_screen();
                    }
                }
                2026 => self.sync_active = set,
                _ => {}
            }
        }
    }

    fn reset_csi(&mut self) {
        self.csi_params = [0; MAX_CSI_PARAMS];
        self.csi_param_count = 0;
        self.csi_has_digit = false;
        self.csi_private = 0;
        self.csi_intermediates = [0; MAX_CSI_INTERMEDIATES];
        self.csi_intermediate_count = 0;
    }

    fn cancel_control_sequence(&mut self) {
        self.state = ParserState::Normal;
        self.reset_csi();
        self.osc_saw_esc = false;
        self.osc_buffer.clear();
        self.dcs_saw_esc = false;
        self.dcs_buffer.clear();
    }

    fn param(&self, index: usize, default: u16) -> u16 {
        if index >= self.csi_param_count {
            return default;
        }
        let value = self.csi_params[index];
        if value == 0 { default } else { value }
    }

    fn param_raw(&self, index: usize, default: u16) -> u16 {
        self.csi_params
            .get(index)
            .copied()
            .filter(|_| index < self.csi_param_count)
            .unwrap_or(default)
    }

    fn cursor_top(&self) -> u16 {
        if self.origin_mode { self.scroll_top } else { 1 }
    }

    fn cursor_bottom(&self) -> u16 {
        if self.origin_mode {
            self.scroll_bottom
        } else {
            self.rows
        }
    }

    fn position_cursor(&mut self, row: u16, col: u16) {
        let top = self.cursor_top();
        let bottom = self.cursor_bottom();
        let absolute_row = if self.origin_mode {
            top.saturating_add(row.saturating_sub(1))
        } else {
            row
        };
        self.cursor_row = clamp(absolute_row, top, bottom);
        self.cursor_col = clamp(col, 1, self.cols);
        self.pending_wrap = false;
    }

    fn save_cursor(&mut self) {
        self.saved_cursor = Some(SavedCursor {
            row: self.cursor_row,
            col: self.cursor_col,
            pending_wrap: self.pending_wrap,
            origin_mode: self.origin_mode,
        });
    }

    fn restore_cursor(&mut self) {
        let Some(saved) = self.saved_cursor else {
            return;
        };
        self.cursor_row = clamp(saved.row, 1, self.rows);
        self.cursor_col = clamp(saved.col, 1, self.cols);
        self.pending_wrap = saved.pending_wrap;
        self.origin_mode = saved.origin_mode;
    }

    fn set_scroll_region(&mut self) {
        if self.csi_private != 0 {
            return;
        }
        let top = clamp(self.param(0, 1), 1, self.rows);
        let bottom = clamp(self.param(1, self.rows), 1, self.rows);
        if top >= bottom {
            return;
        }
        self.scroll_top = top;
        self.scroll_bottom = bottom;
        self.position_cursor(1, 1);
    }

    fn enter_alternate_screen(&mut self) {
        if self.saved_normal_screen.is_some() {
            return;
        }
        let alternate = vec![Cell::default(); self.cells.len()];
        let saved = SavedScreen {
            cells: std::mem::replace(&mut self.cells, alternate),
            row_origin: self.row_origin,
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            autowrap: self.autowrap,
            pending_wrap: self.pending_wrap,
            scroll_top: self.scroll_top,
            scroll_bottom: self.scroll_bottom,
            origin_mode: self.origin_mode,
            insert_mode: self.insert_mode,
            saved_cursor: self.saved_cursor,
            last_printable_idx: self.last_printable_idx,
        };
        self.saved_normal_screen = Some(saved);
        self.row_origin = 0;
        self.cursor_row = 1;
        self.cursor_col = 1;
        self.autowrap = true;
        self.pending_wrap = false;
        self.scroll_top = 1;
        self.scroll_bottom = self.rows;
        self.origin_mode = false;
        self.insert_mode = false;
        self.last_printable_idx = None;
        self.saved_cursor = None;
    }

    fn leave_alternate_screen(&mut self) {
        let Some(saved) = self.saved_normal_screen.take() else {
            return;
        };
        self.cells = saved.cells;
        self.row_origin = saved.row_origin;
        self.cursor_row = saved.cursor_row;
        self.cursor_col = saved.cursor_col;
        self.autowrap = saved.autowrap;
        self.pending_wrap = saved.pending_wrap;
        self.scroll_top = saved.scroll_top;
        self.scroll_bottom = saved.scroll_bottom;
        self.origin_mode = saved.origin_mode;
        self.insert_mode = saved.insert_mode;
        self.saved_cursor = saved.saved_cursor;
        self.last_printable_idx = saved.last_printable_idx;
    }

    fn reset_terminal(&mut self) {
        self.leave_alternate_screen();
        self.cells.fill(Cell::default());
        self.row_origin = 0;
        self.cursor_row = 1;
        self.cursor_col = 1;
        self.autowrap = true;
        self.pending_wrap = false;
        self.cursor_visible = true;
        self.scroll_top = 1;
        self.scroll_bottom = self.rows;
        self.origin_mode = false;
        self.insert_mode = false;
        self.sync_active = false;
        self.sync_buffer.clear();
        self.saved_cursor = None;
        self.last_printable_idx = None;
        self.utf8_len = 0;
        self.utf8_expected = 0;
        self.suffix_pool.clear();
        self.suffix_index.clear();
        self.suffix_pool_bytes = 0;
        initialize_tab_stops(&mut self.tab_stops);
        self.cancel_control_sequence();
    }

    fn advance_row_or_scroll(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            self.scroll_up(self.scroll_top, self.scroll_bottom, 1);
        } else if self.cursor_row < self.rows {
            self.cursor_row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.cursor_row == self.scroll_top {
            self.scroll_down(self.scroll_top, self.scroll_bottom, 1);
        } else if self.cursor_row > 1 {
            self.cursor_row -= 1;
        }
    }

    fn scroll_up(&mut self, top: u16, bottom: u16, requested: u16) {
        if top == 0 || bottom < top || bottom > self.rows {
            return;
        }
        let count = requested.min(bottom - top + 1);
        if count == 0 {
            return;
        }
        if top == 1 && bottom == self.rows && count == 1 {
            self.row_origin = (self.row_origin + 1) % self.rows;
            let base = self.row_base(self.rows);
            self.cells[base..base + usize::from(self.cols)].fill(Cell::default());
            return;
        }
        for row in top..=bottom - count {
            let destination = self.row_base(row);
            let source = self.row_base(row + count);
            self.cells
                .copy_within(source..source + usize::from(self.cols), destination);
        }
        for row in bottom - count + 1..=bottom {
            let base = self.row_base(row);
            self.cells[base..base + usize::from(self.cols)].fill(Cell::default());
        }
    }

    fn scroll_down(&mut self, top: u16, bottom: u16, requested: u16) {
        if top == 0 || bottom < top || bottom > self.rows {
            return;
        }
        let count = requested.min(bottom - top + 1);
        if count == 0 {
            return;
        }
        for row in (top + count..=bottom).rev() {
            let destination = self.row_base(row);
            let source = self.row_base(row - count);
            self.cells
                .copy_within(source..source + usize::from(self.cols), destination);
        }
        for row in top..top + count {
            let base = self.row_base(row);
            self.cells[base..base + usize::from(self.cols)].fill(Cell::default());
        }
    }

    fn insert_cells(&mut self, requested: u16) {
        let count = requested.min(self.cols - self.cursor_col + 1);
        if count == 0 {
            return;
        }
        let base = self.row_base(self.cursor_row);
        let start = base + usize::from(self.cursor_col - 1);
        let end = base + usize::from(self.cols);
        self.cells
            .copy_within(start..end - usize::from(count), start + usize::from(count));
        self.cells[start..start + usize::from(count)].fill(Cell::default());
        repair_wide_cells(&mut self.cells[base..end], self.cols, 1);
        self.pending_wrap = false;
        self.last_printable_idx = None;
    }

    fn delete_cells(&mut self, requested: u16) {
        let count = requested.min(self.cols - self.cursor_col + 1);
        if count == 0 {
            return;
        }
        let base = self.row_base(self.cursor_row);
        let start = base + usize::from(self.cursor_col - 1);
        let end = base + usize::from(self.cols);
        self.cells
            .copy_within(start + usize::from(count)..end, start);
        self.cells[end - usize::from(count)..end].fill(Cell::default());
        repair_wide_cells(&mut self.cells[base..end], self.cols, 1);
        self.pending_wrap = false;
        self.last_printable_idx = None;
    }

    fn erase_cells(&mut self, requested: u16) {
        let count = requested.min(self.cols - self.cursor_col + 1);
        let start = (usize::from(self.cursor_row) - 1) * usize::from(self.cols)
            + usize::from(self.cursor_col - 1);
        self.erase_range(start, start + usize::from(count));
        self.pending_wrap = false;
        self.last_printable_idx = None;
    }

    fn insert_lines(&mut self, requested: u16) {
        if (self.scroll_top..=self.scroll_bottom).contains(&self.cursor_row) {
            self.scroll_down(self.cursor_row, self.scroll_bottom, requested);
            self.pending_wrap = false;
            self.last_printable_idx = None;
        }
    }

    fn delete_lines(&mut self, requested: u16) {
        if (self.scroll_top..=self.scroll_bottom).contains(&self.cursor_row) {
            self.scroll_up(self.cursor_row, self.scroll_bottom, requested);
            self.pending_wrap = false;
            self.last_printable_idx = None;
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let total = self.cells.len();
        match mode {
            0 => {
                let start = (usize::from(self.cursor_row) - 1) * usize::from(self.cols)
                    + usize::from(self.cursor_col - 1);
                self.erase_range(start, total);
            }
            1 => {
                let end = (usize::from(self.cursor_row) - 1) * usize::from(self.cols)
                    + usize::from(self.cursor_col);
                self.erase_range(0, end);
            }
            2 => self.erase_range(0, total),
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let row_base = (usize::from(self.cursor_row) - 1) * usize::from(self.cols);
        match mode {
            0 => self.erase_range(
                row_base + usize::from(self.cursor_col - 1),
                row_base + usize::from(self.cols),
            ),
            1 => self.erase_range(row_base, row_base + usize::from(self.cursor_col)),
            2 => self.erase_range(row_base, row_base + usize::from(self.cols)),
            _ => {}
        }
    }

    fn erase_range(&mut self, start: usize, end: usize) {
        let cols = usize::from(self.cols);
        let mut expanded_start = start;
        let mut expanded_end = end;
        if expanded_start < expanded_end
            && !expanded_start.is_multiple_of(cols)
            && self.cells[self.physical_index_for_logical_offset(expanded_start)].width == 0
        {
            expanded_start -= 1;
        }
        if expanded_end > expanded_start
            && expanded_end < self.cells.len()
            && !expanded_end.is_multiple_of(cols)
            && self.cells[self.physical_index_for_logical_offset(expanded_end - 1)].width == 2
        {
            expanded_end += 1;
        }
        let mut logical = expanded_start;
        while logical < expanded_end {
            let column = logical % cols;
            let chunk = (cols - column).min(expanded_end - logical);
            let physical = self.physical_index_for_logical_offset(logical);
            self.cells[physical..physical + chunk].fill(Cell::default());
            logical += chunk;
        }
    }

    fn clear_wide_glyph_at(&mut self, row: u16, col: u16) {
        if row == 0 || row > self.rows || col == 0 || col > self.cols {
            return;
        }
        let index = self.cell_index(row, col);
        match self.cells[index].width {
            0 => {
                if col > 1 && self.cells[index - 1].width == 2 {
                    self.cells[index - 1] = Cell::default();
                }
                self.cells[index] = Cell::default();
            }
            2 => {
                self.cells[index] = Cell::default();
                if col < self.cols && self.cells[index + 1].width == 0 {
                    self.cells[index + 1] = Cell::default();
                }
            }
            _ => {}
        }
    }

    fn move_tabs_forward(&mut self, requested: u16) {
        for _ in 0..requested {
            let mut column = self.cursor_col.saturating_add(1);
            while column < self.cols && !self.tab_stops[usize::from(column - 1)] {
                column += 1;
            }
            self.cursor_col = column.min(self.cols);
        }
        self.pending_wrap = false;
    }

    fn move_tabs_backward(&mut self, requested: u16) {
        for _ in 0..requested {
            if self.cursor_col <= 1 {
                break;
            }
            let mut column = self.cursor_col - 1;
            while column > 1 && !self.tab_stops[usize::from(column - 1)] {
                column -= 1;
            }
            self.cursor_col = column;
        }
        self.pending_wrap = false;
    }

    fn clear_tab_stops(&mut self, mode: u16) {
        match mode {
            0 => self.tab_stops[usize::from(self.cursor_col - 1)] = false,
            3 => self.tab_stops.fill(false),
            _ => {}
        }
    }

    fn append_suffix(&mut self, cell_index: usize, bytes: &[u8]) -> Result<(), TerminalGridError> {
        let existing = self
            .suffix(self.cells[cell_index].suffix_id)
            .unwrap_or_default();
        let combined_len = existing
            .len()
            .checked_add(bytes.len())
            .ok_or(TerminalGridError::CombiningPoolCapacityExceeded)?;
        let mut combined = Vec::with_capacity(combined_len);
        combined.extend_from_slice(existing);
        combined.extend_from_slice(bytes);
        if let Some(&id) = self.suffix_index.get(combined.as_slice()) {
            self.cells[cell_index].suffix_id = id;
            return Ok(());
        }
        if combined_len > MAX_CELL_TEXT_BYTES
            || self.suffix_pool.len() >= MAX_SUFFIX_ENTRIES
            || self.suffix_pool_bytes > MAX_SUFFIX_POOL_BYTES.saturating_sub(combined_len)
        {
            return Err(TerminalGridError::CombiningPoolCapacityExceeded);
        }
        let combined: Arc<[u8]> = combined.into();
        let id = u32::try_from(self.suffix_pool.len() + 1)
            .map_err(|_| TerminalGridError::CombiningPoolCapacityExceeded)?;
        self.suffix_pool_bytes += combined.len();
        self.suffix_pool.push(Arc::clone(&combined));
        self.suffix_index.insert(combined, id);
        self.cells[cell_index].suffix_id = id;
        Ok(())
    }

    fn suffix(&self, id: u32) -> Option<&[u8]> {
        if id == 0 {
            None
        } else {
            self.suffix_pool
                .get(usize::try_from(id - 1).ok()?)
                .map(AsRef::as_ref)
        }
    }

    fn row_base(&self, row: u16) -> usize {
        physical_row_index(self.row_origin, row - 1, self.rows) * usize::from(self.cols)
    }

    fn cell_index(&self, row: u16, col: u16) -> usize {
        self.row_base(row) + usize::from(col - 1)
    }

    fn physical_index_for_logical_offset(&self, offset: usize) -> usize {
        let cols = usize::from(self.cols);
        let logical_row = u16::try_from(offset / cols).expect("logical row fits grid dimensions");
        physical_row_index(self.row_origin, logical_row, self.rows) * cols + offset % cols
    }
}

fn checked_cell_count(cols: u16, rows: u16) -> Result<usize, TerminalGridError> {
    if cols == 0 || rows == 0 || cols > MAX_DIMENSION || rows > MAX_DIMENSION {
        return Err(TerminalGridError::InvalidGridSize);
    }
    let count = usize::from(cols)
        .checked_mul(usize::from(rows))
        .ok_or(TerminalGridError::GridTooLarge)?;
    if count > MAX_CELLS {
        return Err(TerminalGridError::GridTooLarge);
    }
    Ok(count)
}

fn initialize_tab_stops(stops: &mut [bool]) {
    stops.fill(false);
    for index in (8..stops.len()).step_by(8) {
        stops[index] = true;
    }
}

fn resized_cells(
    source: &[Cell],
    source_cols: u16,
    source_rows: u16,
    source_origin: u16,
    cols: u16,
    rows: u16,
    count: usize,
) -> Vec<Cell> {
    let mut cells = vec![Cell::default(); count];
    for row in 0..source_rows.min(rows) {
        let source_row = physical_row_index(source_origin, row, source_rows);
        let source_base = source_row * usize::from(source_cols);
        let destination_base = usize::from(row) * usize::from(cols);
        let copy_cols = usize::from(source_cols.min(cols));
        cells[destination_base..destination_base + copy_cols]
            .copy_from_slice(&source[source_base..source_base + copy_cols]);
    }
    repair_wide_cells(&mut cells, cols, rows);
    cells
}

fn repair_wide_cells(cells: &mut [Cell], cols: u16, rows: u16) {
    for row in 0..rows {
        let base = usize::from(row) * usize::from(cols);
        for col in 0..cols {
            let index = base + usize::from(col);
            let valid = match cells[index].width {
                0 => {
                    col != 0
                        && cells[index].codepoint == 0
                        && cells[index].suffix_id == 0
                        && cells[index - 1].width == 2
                }
                1 => true,
                2 => {
                    col + 1 < cols
                        && cells[index + 1].width == 0
                        && cells[index + 1].codepoint == 0
                        && cells[index + 1].suffix_id == 0
                }
                _ => false,
            };
            if !valid {
                cells[index] = Cell::default();
            }
        }
    }
}

fn physical_row_index(origin: u16, logical_row: u16, rows: u16) -> usize {
    usize::from((origin + logical_row) % rows)
}

fn append_control_byte(buffer: &mut Vec<u8>, byte: u8) -> Result<(), TerminalGridError> {
    if buffer.len() >= MAX_CONTROL_STRING_BYTES {
        return Err(TerminalGridError::ControlStringTooLarge);
    }
    buffer.push(byte);
    Ok(())
}

fn push_bounded(output: &mut Vec<u8>, byte: u8) -> Result<(), TerminalGridError> {
    if output.len() >= MAX_RENDER_BYTES {
        return Err(TerminalGridError::SnapshotTooLarge);
    }
    output.push(byte);
    Ok(())
}

fn push_slice_bounded(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), TerminalGridError> {
    if bytes.len() > MAX_RENDER_BYTES.saturating_sub(output.len()) {
        return Err(TerminalGridError::SnapshotTooLarge);
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn clamp(value: u16, low: u16, high: u16) -> u16 {
    value.max(low).min(high)
}

fn clamp_sub(value: u16, amount: u16, low: u16) -> u16 {
    if amount >= value {
        low
    } else {
        (value - amount).max(low)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_grid(cols: u16, rows: u16) -> TerminalGrid {
        TerminalGrid::new(cols, rows).expect("valid test dimensions")
    }

    fn snapshot(grid: &TerminalGrid) -> String {
        String::from_utf8(grid.snapshot().expect("bounded snapshot")).expect("valid UTF-8")
    }

    #[test]
    fn writes_cursor_controls_wrap_and_scroll_match_fx() {
        let mut grid = test_grid(5, 3);
        grid.feed(b"abc\rXY\n12345Z").unwrap();
        assert_eq!(snapshot(&grid), "|XYc  |\n|12345|\n|Z    |\n");
        assert_eq!((grid.cursor_row(), grid.cursor_col()), (3, 2));
    }

    #[test]
    fn csi_cursor_edit_erase_tabs_and_scroll_region() {
        let mut grid = test_grid(10, 4);
        grid.feed(b"one\nabcdef\nthree\nfour").unwrap();
        grid.feed(b"\x1b[2;3H\x1b[2@XY\x1b[1P\x1b[2X").unwrap();
        grid.feed(b"\x1b[2;4r\x1b[4;1H\n").unwrap();
        grid.feed(b"\x1b[1;1H\tT").unwrap();
        assert_eq!(
            snapshot(&grid),
            "|one     T |\n|three     |\n|four      |\n|          |\n"
        );
    }

    #[test]
    fn modes_save_restore_alternate_and_cursor_visibility() {
        let mut grid = test_grid(8, 2);
        grid.feed(b"normal\x1b7\x1b[?1049halt\x1b[?25l").unwrap();
        assert_eq!(snapshot(&grid), "|alt     |\n|        |\n");
        assert!(!grid.cursor_visible());
        grid.feed(b"\x1b[?1049l\x1b8!").unwrap();
        assert_eq!(snapshot(&grid), "|normal! |\n|        |\n");
        // Pinned fx keeps the active cursor-visibility presentation mode
        // across this normal-screen restore.
        assert!(!grid.cursor_visible());
    }

    #[test]
    fn del_is_a_zero_width_suffix_and_reuses_indexed_storage() {
        let mut grid = test_grid(6, 1);
        grid.feed(b"a\x7fb\x7f").unwrap();

        assert_eq!(grid.snapshot().unwrap(), b"|a\x7fb\x7f    |\n");
        assert_eq!(grid.suffix_pool.len(), 1);
        assert_eq!(grid.suffix_index.len(), 1);
        assert_eq!(grid.cells[0].suffix_id, 1);
        assert_eq!(grid.cells[1].suffix_id, 1);
    }

    #[test]
    fn fragmented_parser_utf8_and_suppressed_control_strings() {
        let mut grid = test_grid(12, 2);
        for part in [
            &b"A\xe7"[..],
            &b"\x95"[..],
            &b"\x8c\x1b[2;"[..],
            &b"3HZ\x1b]title"[..],
            &b" ignored\x1b"[..],
            &b"\\\x1bP$qm\x1b"[..],
            &b"\\Q"[..],
        ] {
            grid.feed(part).unwrap();
        }
        assert_eq!(snapshot(&grid), "|A界         |\n|  ZQ        |\n");
    }

    #[test]
    fn invalid_utf8_is_replaced_without_losing_following_bytes() {
        let mut grid = test_grid(6, 1);
        grid.feed(&[0xf0]).unwrap();
        grid.feed(b"(x").unwrap();
        assert_eq!(snapshot(&grid), "|�(x   |\n");
    }

    #[test]
    fn non_scalar_four_byte_prefixes_buffer_until_complete() {
        for prefix in 0xf5..=0xf7 {
            let mut grid = test_grid(4, 1);
            grid.feed(&[prefix]).unwrap();
            assert_eq!(snapshot(&grid), "|    |\n");
            grid.feed(&[0x80, 0x80]).unwrap();
            assert_eq!(snapshot(&grid), "|    |\n");
            grid.feed(&[0x80]).unwrap();
            assert_eq!(snapshot(&grid), "|�   |\n");
        }
    }

    #[test]
    fn c0_and_c1_prefixes_match_pinned_fragmentation_behavior() {
        for prefix in 0xc0..=0xc1 {
            let mut pending = test_grid(4, 1);
            pending.feed(&[prefix]).unwrap();
            assert_eq!(snapshot(&pending), "|    |\n", "lone prefix {prefix:#x}");

            for continuation in 0x80..=0xbf {
                let mut complete = test_grid(4, 1);
                complete.feed(&[prefix, continuation]).unwrap();
                assert_eq!(
                    snapshot(&complete),
                    "|��  |\n",
                    "complete pair {prefix:#x} {continuation:#x}"
                );

                let mut fragmented = test_grid(4, 1);
                fragmented.feed(&[prefix]).unwrap();
                fragmented.feed(&[continuation]).unwrap();
                assert_eq!(
                    snapshot(&fragmented),
                    "|�   |\n",
                    "fragmented pair {prefix:#x} {continuation:#x}"
                );
            }
        }
    }

    #[test]
    fn cancellation_checkpoint_follows_the_complete_display_unit() {
        let mut grid = test_grid(4096, 5);
        let mut payload = vec![b'a'; FEED_CANCELLATION_CHECKPOINT_BYTES - 4];
        payload.extend_from_slice("👩‍💻".as_bytes());
        let mut checks = 0;

        let error = grid
            .feed_with_cancel_check(&payload, || {
                checks += 1;
                true
            })
            .expect_err("the first bounded checkpoint cancels");

        assert_eq!(error, TerminalGridFeedError::Cancelled);
        assert_eq!(checks, 1);
        assert!(
            grid.snapshot()
                .unwrap()
                .windows("👩‍💻".len())
                .any(|window| window == "👩‍💻".as_bytes()),
            "the checkpoint does not split the ZWJ display unit"
        );
    }

    #[test]
    fn terminal_grid_remains_send_with_indexed_suffix_storage() {
        fn assert_send<T: Send>() {}
        assert_send::<TerminalGrid>();
    }

    #[test]
    fn unicode_display_units_combining_variants_and_emoji_are_exact() {
        let mut grid = test_grid(14, 1);
        grid.feed("a\u{301}界☀\u{fe0e}👍🏽🇺🇸👩‍💻".as_bytes()).unwrap();
        assert_eq!(snapshot(&grid), "|a\u{301}界☀\u{fe0e}👍🏽🇺🇸👩‍💻    |\n");
        assert_eq!(grid.cursor_col(), 11);
    }

    #[test]
    fn synchronized_updates_apply_only_after_fragmented_reset() {
        let mut grid = test_grid(8, 1);
        grid.feed(b"old\x1b[?2026h\rnew").unwrap();
        assert_eq!(snapshot(&grid), "|old     |\n");
        grid.feed(b" text\x1b[?20").unwrap();
        assert_eq!(snapshot(&grid), "|old     |\n");
        grid.feed(b"26l").unwrap();
        assert_eq!(snapshot(&grid), "|new text|\n");
    }

    #[test]
    fn resize_keeps_top_left_and_repairs_clipped_wide_cells() {
        let mut grid = test_grid(6, 2);
        grid.feed("abcd界\nsecond".as_bytes()).unwrap();
        grid.resize(5, 3).unwrap();
        assert_eq!(snapshot(&grid), "|abcd |\n|secon|\n|     |\n");
        grid.resize(7, 2).unwrap();
        assert_eq!(snapshot(&grid), "|abcd   |\n|secon  |\n");
    }

    #[test]
    fn wide_resize_then_csi_k_regression_clears_complete_glyph() {
        let mut grid = test_grid(8, 2);
        grid.feed("abc界xyz".as_bytes()).unwrap();
        grid.resize(7, 2).unwrap();
        grid.feed(b"\x1b[1;5H\x1b[K").unwrap();
        assert_eq!(snapshot(&grid), "|abc    |\n|       |\n");
    }

    #[test]
    fn fixed_resource_limits_reject_excess() {
        assert!(matches!(
            TerminalGrid::new(4096, 4096),
            Err(TerminalGridError::GridTooLarge)
        ));
        let mut grid = test_grid(2, 1);
        let mut params = b"\x1b[".to_vec();
        params.extend_from_slice(b"1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;1;");
        assert_eq!(
            grid.feed(&params),
            Err(TerminalGridError::TooManyCsiParameters)
        );
        let mut grid = test_grid(2, 1);
        let oversized = vec![b'x'; MAX_CONTROL_STRING_BYTES + 1];
        grid.feed(b"\x1b]").unwrap();
        assert_eq!(
            grid.feed(&oversized),
            Err(TerminalGridError::ControlStringTooLarge)
        );
    }

    #[test]
    fn getters_and_zero_or_over_dimension_rejection_are_stable() {
        let grid = test_grid(80, 24);
        assert_eq!((grid.cols(), grid.rows()), (80, 24));
        assert_eq!((grid.cursor_row(), grid.cursor_col()), (1, 1));
        assert!(grid.cursor_visible());
        assert!(TerminalGrid::new(0, 1).is_err());
        assert!(TerminalGrid::new(1, 0).is_err());
        assert!(TerminalGrid::new(MAX_DIMENSION + 1, 1).is_err());
    }
}
