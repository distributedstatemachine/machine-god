// SPDX-License-Identifier: Apache-2.0
//
// Safe, bounded Rust transliteration of vercel-labs/fx at revision
// b1774fbf6c7602b503026f96f6e960e946c692ef:
// src/core/terminal/engine.zig. Replay remains observational; live callers
// explicitly request and own bounded protocol replies.

use super::terminal_display_width::{decode_next_rune, display_unit_at, utf8_sequence_len};
use machine_god_core::{
    TerminalCell, TerminalCellKind, TerminalCellStyle, TerminalColor, TerminalCursorShape,
    TerminalDimensions, TerminalHyperlink, TerminalModes, TerminalScreen, TerminalScreenCursor,
};
use std::collections::{BTreeSet, HashMap};
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
// Complete UTF-8 cell text, including its base scalar; matches TerminalCell.
const MAX_CELL_TEXT_BYTES: usize = 64;
const FEED_CANCELLATION_CHECKPOINT_BYTES: usize = 16 * 1024;
const SYNC_RESET: &[u8] = b"\x1b[?2026l";
const MAX_REPLY_COUNT: usize = 16;
const MAX_REPLY_BYTES: usize = 256;
const MAX_REPLY_TOTAL_BYTES: usize = 4096;
const MAX_HYPERLINK_POOL_BYTES: usize = 4 * 1024 * 1024;
const MAX_CHECKPOINT_BYTES: usize = 32 * 1024 * 1024;
// Fixed-width fields emitted by CheckpointWriter, not Rust struct sizes or the
// pinned engine's separate render-allocation estimate above.
const CHECKPOINT_STYLE_BYTES: usize = 4 + 4 + 1;
const CHECKPOINT_CELL_BYTES: usize = 4 + 1 + 4 + CHECKPOINT_STYLE_BYTES + 4;

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
    HyperlinkPoolCapacityExceeded,
    ReplyCapacityExceeded,
    InvalidCheckpoint,
    CheckpointTooLarge,
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
            Self::HyperlinkPoolCapacityExceeded => "terminal hyperlink pool limit exceeded",
            Self::ReplyCapacityExceeded => "terminal reply capacity exceeded",
            Self::InvalidCheckpoint => "invalid terminal checkpoint",
            Self::CheckpointTooLarge => "terminal checkpoint limit exceeded",
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

#[derive(Clone, Copy, Eq, PartialEq)]
struct Cell {
    codepoint: u32,
    width: u8,
    suffix_id: u32,
    style: TerminalCellStyle,
    hyperlink_id: u32,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            codepoint: u32::from(' '),
            width: 1,
            suffix_id: 0,
            style: TerminalCellStyle::default(),
            hyperlink_id: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct SavedCursor {
    row: u16,
    col: u16,
    pending_wrap: bool,
    origin_mode: bool,
    style: TerminalCellStyle,
    hyperlink_id: u32,
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
    current_style: TerminalCellStyle,
    hyperlink_id: u32,
    hyperlink_params: Vec<u8>,
    cursor_shape: TerminalCursorShape,
    cursor_blinking: bool,
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
    current_style: TerminalCellStyle,
    hyperlink_id: u32,
    hyperlink_params: Vec<u8>,
    hyperlink_pool: Vec<Arc<[u8]>>,
    hyperlink_index: HashMap<Arc<[u8]>, u32>,
    hyperlink_pool_bytes: usize,
    cursor_shape: TerminalCursorShape,
    cursor_blinking: bool,
    bracketed_paste: bool,
    mouse_modes: u16,
    focus_tracking: bool,
    application_cursor_keys: bool,
    application_keypad: bool,
    keyboard_protocol: bool,
    live_feed: bool,
    replies: Vec<Vec<u8>>,
    reply_bytes: usize,
}

// Active and saved screens deliberately have the same persisted field names.
macro_rules! encode_screen {
    ($writer:ident, $screen:expr) => {{
        let screen = $screen;
        $writer.cells(&screen.cells)?;
        for value in [
            screen.row_origin,
            screen.cursor_row,
            screen.cursor_col,
            screen.scroll_top,
            screen.scroll_bottom,
        ] {
            $writer.u16(value)?;
        }
        for value in [
            screen.autowrap,
            screen.pending_wrap,
            screen.origin_mode,
            screen.insert_mode,
        ] {
            $writer.boolean(value)?;
        }
        $writer.style(screen.current_style)?;
        $writer.u32(screen.hyperlink_id as usize)?;
        $writer.sized(&screen.hyperlink_params)?;
        $writer.u8(shape_code(screen.cursor_shape))?;
        $writer.boolean(screen.cursor_blinking)?;
        $writer.boolean(screen.saved_cursor.is_some())?;
        if let Some(saved) = screen.saved_cursor {
            $writer.u16(saved.row)?;
            $writer.u16(saved.col)?;
            $writer.boolean(saved.pending_wrap)?;
            $writer.boolean(saved.origin_mode)?;
            $writer.style(saved.style)?;
            $writer.u32(saved.hyperlink_id as usize)?;
        }
        $writer.boolean(screen.last_printable_idx.is_some())?;
        if let Some(index) = screen.last_printable_idx {
            $writer.u32(index)?;
        }
    }};
}

macro_rules! validate_screen {
    ($grid:expr, $screen:expr) => {{
        let grid = $grid;
        let screen = $screen;
        validate_cells(
            &screen.cells,
            grid.cols,
            grid.rows,
            &grid.suffix_pool,
            grid.hyperlink_pool.len(),
        )?;
        if screen.row_origin >= grid.rows
            || screen.cursor_row == 0
            || screen.cursor_row > grid.rows
            || screen.cursor_col == 0
            || screen.cursor_col > grid.cols
            || screen.scroll_top == 0
            || screen.scroll_top > screen.scroll_bottom
            || screen.scroll_bottom > grid.rows
            || screen.hyperlink_id as usize > grid.hyperlink_pool.len()
            || screen.hyperlink_params.len() > MAX_CONTROL_STRING_BYTES
            || screen
                .last_printable_idx
                .is_some_and(|index| screen.cells.get(index).is_none_or(|cell| cell.width == 0))
        {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
        if let Some(saved) = screen.saved_cursor {
            // Resize clamps saved cursor only when it is restored, matching fx.
            if saved.row == 0
                || saved.row > MAX_DIMENSION
                || saved.col == 0
                || saved.col > MAX_DIMENSION
                || saved.hyperlink_id as usize > grid.hyperlink_pool.len()
            {
                return Err(TerminalGridError::InvalidCheckpoint);
            }
        }
    }};
}

impl TerminalGrid {
    /// Upper bound for this encoder at fixed dimensions, including a saved
    /// normal screen and every independently bounded parser/pool field. Pools
    /// retain unused entries, so even a tiny grid needs their full allowance.
    /// The result is not clamped: every valid checkpoint state fits the result,
    /// and all supported dimensions fit the existing global checkpoint cap.
    pub(crate) fn checkpoint_bound(cols: u16, rows: u16) -> Result<usize, TerminalGridError> {
        let cells = checked_cell_count(cols, rows)?
            .checked_mul(CHECKPOINT_CELL_BYTES)
            .ok_or(TerminalGridError::CheckpointTooLarge)?;
        // encode_screen!: cell count/payload, five coordinates, four modes,
        // style/link/params, shape/blink, maximal saved cursor and last index.
        let screen = checkpoint_size_sum(&[
            4,
            cells,
            5 * 2,
            4,
            CHECKPOINT_STYLE_BYTES,
            4,
            4 + MAX_CONTROL_STRING_BYTES,
            1 + 1,
            1 + 2 * 2 + 2 + CHECKPOINT_STYLE_BYTES + 4,
            1 + 4,
        ])?;
        let both_screens = screen
            .checked_mul(2)
            .ok_or(TerminalGridError::CheckpointTooLarge)?;
        let bound = checkpoint_size_sum(&[
            6 + 2 + 2, // Magic/version and dimensions.
            both_screens,
            1 + 5 + 2,         // Cursor visibility, global modes and mouse mask.
            usize::from(cols), // One byte per tab stop, not a packed bitset.
            1 + 4 + MAX_SYNC_BYTES,
            1 + MAX_CSI_PARAMS * 2 + 4 + 1 + 1 + MAX_CSI_INTERMEDIATES + 4,
            2 * (1 + 4 + MAX_CONTROL_STRING_BYTES), // OSC and DCS.
            4 + 4 + 4, // UTF-8 pending bytes, length and expected length.
            checkpoint_pool_bound(MAX_CELL_TEXT_BYTES, MAX_SUFFIX_POOL_BYTES)?,
            checkpoint_pool_bound(MAX_CONTROL_STRING_BYTES, MAX_HYPERLINK_POOL_BYTES)?,
            1, // Saved-screen presence flag; its full payload is included above.
        ])?;
        if bound > MAX_CHECKPOINT_BYTES {
            return Err(TerminalGridError::CheckpointTooLarge);
        }
        Ok(bound)
    }

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
            current_style: TerminalCellStyle::default(),
            hyperlink_id: 0,
            hyperlink_params: Vec::new(),
            hyperlink_pool: Vec::new(),
            hyperlink_index: HashMap::new(),
            hyperlink_pool_bytes: 0,
            cursor_shape: TerminalCursorShape::Block,
            cursor_blinking: true,
            bracketed_paste: false,
            mouse_modes: 0,
            focus_tracking: false,
            application_cursor_keys: false,
            application_keypad: false,
            keyboard_protocol: false,
            live_feed: false,
            replies: Vec::new(),
            reply_bytes: 0,
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

    /// Explicitly permits bounded protocol replies. No I/O is performed here.
    pub(crate) fn feed_live(&mut self, bytes: &[u8]) -> Result<(), TerminalGridError> {
        self.live_feed = true;
        let result = self.feed(bytes);
        self.live_feed = false;
        result
    }

    pub(crate) fn take_replies(&mut self) -> Vec<Vec<u8>> {
        self.reply_bytes = 0;
        std::mem::take(&mut self.replies)
    }

    pub(crate) fn modes(&self) -> TerminalModes {
        TerminalModes {
            alternate_screen: self.saved_normal_screen.is_some(),
            origin: self.origin_mode,
            autowrap: self.autowrap,
            insert: self.insert_mode,
            bracketed_paste: self.bracketed_paste,
            mouse_tracking: self.mouse_modes != 0,
            focus_tracking: self.focus_tracking,
            application_cursor_keys: self.application_cursor_keys,
            application_keypad: self.application_keypad,
            keyboard_protocol: self.keyboard_protocol,
            synchronized_updates: self.sync_active,
        }
    }

    pub(crate) fn structured_screen(&self) -> Result<TerminalScreen, TerminalGridError> {
        let mut cells = Vec::with_capacity(self.cells.len());
        let mut used_links = BTreeSet::new();
        for row in 1..=self.rows {
            let base = self.row_base(row);
            for cell in &self.cells[base..base + usize::from(self.cols)] {
                let kind = match cell.width {
                    0 => TerminalCellKind::Continuation,
                    2 => TerminalCellKind::Wide,
                    _ if cell.codepoint == u32::from(' ') && cell.suffix_id == 0 => {
                        TerminalCellKind::Blank
                    }
                    _ => TerminalCellKind::Single,
                };
                let mut text = String::new();
                if cell.width != 0 && kind != TerminalCellKind::Blank {
                    text.push(
                        char::from_u32(cell.codepoint)
                            .ok_or(TerminalGridError::InvalidCheckpoint)?,
                    );
                    if let Some(suffix) = self.suffix(cell.suffix_id) {
                        text.push_str(
                            std::str::from_utf8(suffix)
                                .map_err(|_| TerminalGridError::InvalidCheckpoint)?,
                        );
                    }
                }
                let hyperlink_id = (cell.hyperlink_id != 0).then_some(cell.hyperlink_id);
                if let Some(id) = hyperlink_id {
                    used_links.insert(id);
                }
                cells.push(TerminalCell {
                    kind,
                    text,
                    style: cell.style,
                    hyperlink_id,
                });
            }
        }
        let screen = TerminalScreen {
            dimensions: TerminalDimensions::new(self.rows, self.cols)
                .map_err(|_| TerminalGridError::InvalidGridSize)?,
            cursor: TerminalScreenCursor {
                row: self.cursor_row - 1,
                column: self.cursor_col - 1,
                visible: self.cursor_visible,
                shape: self.cursor_shape,
                blinking: self.cursor_blinking,
            },
            modes: self.modes(),
            cells,
            hyperlinks: used_links
                .into_iter()
                .map(|id| TerminalHyperlink {
                    id,
                    uri: self.hyperlink_pool[(id - 1) as usize].to_vec(),
                })
                .collect(),
        };
        screen
            .validate()
            .map_err(|_| TerminalGridError::SnapshotTooLarge)?;
        Ok(screen)
    }

    pub(crate) fn hyperlink_at(&self, row: u16, column: u16) -> Option<&[u8]> {
        if row >= self.rows || column >= self.cols {
            return None;
        }
        let id = self.cells[self.cell_index(row + 1, column + 1)].hyperlink_id;
        self.hyperlink_pool
            .get(usize::try_from(id.checked_sub(1)?).ok()?)
            .map(AsRef::as_ref)
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

    /// Versioned, bounded state bytes. Reply effects are deliberately excluded:
    /// restoring a checkpoint cannot repeat an already-issued protocol reply.
    pub(crate) fn checkpoint(&self) -> Result<Vec<u8>, TerminalGridError> {
        self.validate_checkpoint()?;
        let mut writer = CheckpointWriter(Vec::new());
        writer.bytes(b"MGTE\x01\0")?;
        writer.u16(self.cols)?;
        writer.u16(self.rows)?;
        encode_screen!(writer, self);
        writer.boolean(self.cursor_visible)?;
        for mode in [
            self.bracketed_paste,
            self.focus_tracking,
            self.application_cursor_keys,
            self.application_keypad,
            self.keyboard_protocol,
        ] {
            writer.boolean(mode)?;
        }
        writer.u16(self.mouse_modes)?;
        for stop in &self.tab_stops {
            writer.boolean(*stop)?;
        }
        writer.boolean(self.sync_active)?;
        writer.sized(&self.sync_buffer)?;
        writer.u8(match self.state {
            ParserState::Normal => 0,
            ParserState::Escape => 1,
            ParserState::Csi => 2,
            ParserState::Osc => 3,
            ParserState::Dcs => 4,
        })?;
        for parameter in self.csi_params {
            writer.u16(parameter)?;
        }
        writer.u32(self.csi_param_count)?;
        writer.boolean(self.csi_has_digit)?;
        writer.u8(self.csi_private)?;
        writer.bytes(&self.csi_intermediates)?;
        writer.u32(self.csi_intermediate_count)?;
        writer.boolean(self.osc_saw_esc)?;
        writer.sized(&self.osc_buffer)?;
        writer.boolean(self.dcs_saw_esc)?;
        writer.sized(&self.dcs_buffer)?;
        writer.bytes(&self.utf8_buffer)?;
        writer.u32(self.utf8_len)?;
        writer.u32(self.utf8_expected)?;
        writer.pool(&self.suffix_pool)?;
        writer.pool(&self.hyperlink_pool)?;
        writer.boolean(self.saved_normal_screen.is_some())?;
        if let Some(saved) = &self.saved_normal_screen {
            encode_screen!(writer, saved);
        }
        Ok(writer.0)
    }

    pub(crate) fn restore(bytes: &[u8]) -> Result<Self, TerminalGridError> {
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(TerminalGridError::CheckpointTooLarge);
        }
        let mut reader = CheckpointReader { bytes, offset: 0 };
        if reader.bytes(6)? != b"MGTE\x01\0" {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
        let cols = reader.u16()?;
        let rows = reader.u16()?;
        let count = checked_cell_count(cols, rows)?;
        let active = reader.screen(count)?;
        let mut grid = Self::new(cols, rows)?;
        grid.saved_normal_screen = Some(active);
        grid.leave_alternate_screen();
        grid.cursor_visible = reader.boolean()?;
        grid.bracketed_paste = reader.boolean()?;
        grid.focus_tracking = reader.boolean()?;
        grid.application_cursor_keys = reader.boolean()?;
        grid.application_keypad = reader.boolean()?;
        grid.keyboard_protocol = reader.boolean()?;
        grid.mouse_modes = reader.u16()?;
        for stop in &mut grid.tab_stops {
            *stop = reader.boolean()?;
        }
        grid.sync_active = reader.boolean()?;
        grid.sync_buffer = reader.sized(MAX_SYNC_BYTES)?.to_vec();
        grid.state = match reader.u8()? {
            0 => ParserState::Normal,
            1 => ParserState::Escape,
            2 => ParserState::Csi,
            3 => ParserState::Osc,
            4 => ParserState::Dcs,
            _ => return Err(TerminalGridError::InvalidCheckpoint),
        };
        for parameter in &mut grid.csi_params {
            *parameter = reader.u16()?;
        }
        grid.csi_param_count = reader.u32()?;
        grid.csi_has_digit = reader.boolean()?;
        grid.csi_private = reader.u8()?;
        grid.csi_intermediates
            .copy_from_slice(reader.bytes(MAX_CSI_INTERMEDIATES)?);
        grid.csi_intermediate_count = reader.u32()?;
        grid.osc_saw_esc = reader.boolean()?;
        grid.osc_buffer = reader.sized(MAX_CONTROL_STRING_BYTES)?.to_vec();
        grid.dcs_saw_esc = reader.boolean()?;
        grid.dcs_buffer = reader.sized(MAX_CONTROL_STRING_BYTES)?.to_vec();
        grid.utf8_buffer.copy_from_slice(reader.bytes(4)?);
        grid.utf8_len = reader.u32()?;
        grid.utf8_expected = reader.u32()?;
        grid.suffix_pool = reader.pool(MAX_CELL_TEXT_BYTES, MAX_SUFFIX_POOL_BYTES)?;
        grid.hyperlink_pool = reader.pool(MAX_CONTROL_STRING_BYTES, MAX_HYPERLINK_POOL_BYTES)?;
        grid.suffix_pool_bytes = grid.suffix_pool.iter().map(|item| item.len()).sum();
        grid.hyperlink_pool_bytes = grid.hyperlink_pool.iter().map(|item| item.len()).sum();
        grid.suffix_index = index_pool(&grid.suffix_pool)?;
        grid.hyperlink_index = index_pool(&grid.hyperlink_pool)?;
        if reader.boolean()? {
            grid.saved_normal_screen = Some(reader.screen(count)?);
        }
        if reader.offset != bytes.len() {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
        grid.validate_checkpoint()?;
        Ok(grid)
    }

    fn validate_checkpoint(&self) -> Result<(), TerminalGridError> {
        let invalid = TerminalGridError::InvalidCheckpoint;
        if checked_cell_count(self.cols, self.rows)? != self.cells.len()
            || self.tab_stops.len() != usize::from(self.cols)
            || self.csi_param_count > MAX_CSI_PARAMS
            || (self.state == ParserState::Csi && self.csi_param_count >= MAX_CSI_PARAMS)
            || self.csi_intermediate_count > MAX_CSI_INTERMEDIATES
            || self.osc_buffer.len() > MAX_CONTROL_STRING_BYTES
            || self.dcs_buffer.len() > MAX_CONTROL_STRING_BYTES
            || self.sync_buffer.len() > MAX_SYNC_BYTES
            || (!self.sync_active && !self.sync_buffer.is_empty())
            || (self.osc_saw_esc && self.state != ParserState::Osc)
            || (self.dcs_saw_esc && self.state != ParserState::Dcs)
            || self.mouse_modes & !0x3f != 0
            || self.utf8_expected > 4
            || (self.utf8_len == 0 && self.utf8_expected != 0)
            || (self.utf8_len > 0
                && (self.state != ParserState::Normal
                    || self.utf8_len >= self.utf8_expected
                    || utf8_sequence_len(self.utf8_buffer[0]) != Some(self.utf8_expected)))
            || !matches!(self.csi_private, 0 | b'?' | b'>' | b'<' | b'=')
        {
            return Err(invalid);
        }
        validate_pool(
            &self.suffix_pool,
            MAX_CELL_TEXT_BYTES,
            MAX_SUFFIX_POOL_BYTES,
        )?;
        for suffix in &self.suffix_pool {
            // Native parsing cannot store C0 bytes as printable suffixes.
            // Restoring them would turn an observational snapshot into an
            // injected terminal control sequence.
            if std::str::from_utf8(suffix).is_err() || suffix.iter().any(|byte| *byte < 0x20) {
                return Err(invalid);
            }
        }
        validate_pool(
            &self.hyperlink_pool,
            MAX_CONTROL_STRING_BYTES,
            MAX_HYPERLINK_POOL_BYTES,
        )?;
        validate_screen!(self, self);
        if let Some(saved) = &self.saved_normal_screen {
            validate_screen!(self, saved);
        }
        Ok(())
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
                    self.dispatch_csi(byte)?;
                    self.state = ParserState::Normal;
                    index += 1;
                    checkpoint.consume(1, is_cancelled)?;
                    if stop_on_sync_start && self.sync_active {
                        return Ok(index);
                    }
                }
                ParserState::Osc => {
                    if byte == 0x07 {
                        self.dispatch_osc()?;
                        self.cancel_control_sequence();
                    } else if byte == 0x1b {
                        self.osc_saw_esc = true;
                    } else if self.osc_saw_esc && byte == b'\\' {
                        self.dispatch_osc()?;
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
                        self.dispatch_dcs()?;
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
            b'=' | b'>' => {
                self.application_keypad = byte == b'=';
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
            style: self.current_style,
            hyperlink_id: self.hyperlink_id,
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
                style: self.current_style,
                hyperlink_id: self.hyperlink_id,
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

    fn dispatch_csi(&mut self, final_byte: u8) -> Result<(), TerminalGridError> {
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
            b'u' => self.keyboard_protocol = self.csi_private != b'<' && self.param_raw(0, 0) != 0,
            b'm' if self.csi_private == 0 => self.apply_sgr(),
            b'n' | b'c' | b't' => self.dispatch_query(final_byte)?,
            b'q' if self.csi_intermediate_count == 1 && self.csi_intermediates[0] == b' ' => {
                let value = self.param_raw(0, 0);
                if value <= 6 {
                    self.cursor_shape = match value {
                        3 | 4 => TerminalCursorShape::Underline,
                        5 | 6 => TerminalCursorShape::Bar,
                        _ => TerminalCursorShape::Block,
                    };
                    self.cursor_blinking = value == 0 || value % 2 == 1;
                }
            }
            _ => {}
        }
        Ok(())
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
                1 => self.application_cursor_keys = set,
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
                1004 => self.focus_tracking = set,
                2004 => self.bracketed_paste = set,
                1000 | 1002 | 1003 | 1005 | 1006 | 1015 => {
                    let bit = match param {
                        1000 => 0,
                        1002 => 1,
                        1003 => 2,
                        1005 => 3,
                        1006 => 4,
                        _ => 5,
                    };
                    if set {
                        self.mouse_modes |= 1 << bit;
                    } else {
                        self.mouse_modes &= !(1 << bit);
                    }
                }
                _ => {}
            }
        }
    }

    fn append_reply(&mut self, bytes: &[u8]) -> Result<(), TerminalGridError> {
        if !self.live_feed {
            return Ok(());
        }
        if bytes.is_empty()
            || bytes.len() > MAX_REPLY_BYTES
            || self.replies.len() >= MAX_REPLY_COUNT
            || bytes.len() > MAX_REPLY_TOTAL_BYTES.saturating_sub(self.reply_bytes)
        {
            return Err(TerminalGridError::ReplyCapacityExceeded);
        }
        self.reply_bytes += bytes.len();
        self.replies.push(bytes.to_vec());
        Ok(())
    }

    fn dispatch_query(&mut self, final_byte: u8) -> Result<(), TerminalGridError> {
        // Replay consumes queries without even allocating a reply.
        if !self.live_feed {
            return Ok(());
        }
        let reply = match (final_byte, self.param_raw(0, 0)) {
            (b'n', 5) if self.csi_private == 0 => "\x1b[0n".to_owned(),
            (b'n', 6) => {
                let row = if self.origin_mode {
                    self.cursor_row.saturating_sub(self.scroll_top) + 1
                } else {
                    self.cursor_row
                };
                format!(
                    "\x1b[{}{row};{}R",
                    if self.csi_private == b'?' { "?" } else { "" },
                    self.cursor_col
                )
            }
            (b'c', _) if self.csi_private == b'>' => "\x1b[>0;0;0c".to_owned(),
            (b'c', _) if self.csi_private == 0 => "\x1b[?1;2c".to_owned(),
            (b't', 14) => "\x1b[4;0;0t".to_owned(),
            (b't', 16) => "\x1b[6;0;0t".to_owned(),
            (b't', 18) => format!("\x1b[8;{};{}t", self.rows, self.cols),
            (b't', 19) => format!("\x1b[9;{};{}t", self.rows, self.cols),
            _ => return Ok(()),
        };
        self.append_reply(reply.as_bytes())
    }

    fn dispatch_dcs(&mut self) -> Result<(), TerminalGridError> {
        if !self.live_feed {
            return Ok(());
        }
        match self.dcs_buffer.as_slice() {
            b"$qm" => self.append_reply(b"\x1bP1$r0m\x1b\\"),
            b"$qr" => self.append_reply(
                format!("\x1bP1$r{};{}r\x1b\\", self.scroll_top, self.scroll_bottom).as_bytes(),
            ),
            bytes if bytes.starts_with(b"$q") => self.append_reply(b"\x1bP0$r\x1b\\"),
            _ => Ok(()),
        }
    }

    fn dispatch_osc(&mut self) -> Result<(), TerminalGridError> {
        let Some(payload) = self.osc_buffer.strip_prefix(b"8;") else {
            return Ok(());
        };
        let Some(split) = payload.iter().position(|byte| *byte == b';') else {
            return Ok(());
        };
        let (params, uri) = (&payload[..split], &payload[split + 1..]);
        if uri.is_empty() {
            self.hyperlink_id = 0;
            self.hyperlink_params.clear();
            return Ok(());
        }
        let id = if let Some(id) = self.hyperlink_index.get(uri) {
            *id
        } else {
            if self.hyperlink_pool.len() >= MAX_SUFFIX_ENTRIES
                || uri.len() > MAX_HYPERLINK_POOL_BYTES.saturating_sub(self.hyperlink_pool_bytes)
            {
                return Err(TerminalGridError::HyperlinkPoolCapacityExceeded);
            }
            let uri: Arc<[u8]> = uri.into();
            let id = u32::try_from(self.hyperlink_pool.len() + 1)
                .map_err(|_| TerminalGridError::HyperlinkPoolCapacityExceeded)?;
            self.hyperlink_pool_bytes += uri.len();
            self.hyperlink_pool.push(Arc::clone(&uri));
            self.hyperlink_index.insert(uri, id);
            id
        };
        self.hyperlink_params = params.to_vec();
        self.hyperlink_id = id;
        Ok(())
    }

    fn apply_sgr(&mut self) {
        if self.csi_param_count == 0 {
            self.current_style = TerminalCellStyle::default();
            return;
        }
        let mut index = 0;
        while index < self.csi_param_count {
            match self.csi_params[index] {
                0 => self.current_style = TerminalCellStyle::default(),
                1 => self.current_style.bold = true,
                2 => self.current_style.faint = true,
                3 => self.current_style.italic = true,
                4 => self.current_style.underline = true,
                7 => self.current_style.inverse = true,
                9 => self.current_style.strikethrough = true,
                22 => {
                    self.current_style.bold = false;
                    self.current_style.faint = false;
                }
                23 => self.current_style.italic = false,
                24 => self.current_style.underline = false,
                27 => self.current_style.inverse = false,
                29 => self.current_style.strikethrough = false,
                value @ (30..=37 | 90..=97) => {
                    self.current_style.foreground = TerminalColor::Indexed {
                        index: u8::try_from(if value >= 90 {
                            value - 90 + 8
                        } else {
                            value - 30
                        })
                        .expect("palette index is below sixteen"),
                    }
                }
                value @ (40..=47 | 100..=107) => {
                    self.current_style.background = TerminalColor::Indexed {
                        index: u8::try_from(if value >= 100 {
                            value - 100 + 8
                        } else {
                            value - 40
                        })
                        .expect("palette index is below sixteen"),
                    }
                }
                39 => self.current_style.foreground = TerminalColor::Default,
                49 => self.current_style.background = TerminalColor::Default,
                value @ (38 | 48) => {
                    if let Some((color, consumed)) = self.extended_color(index) {
                        if value == 38 {
                            self.current_style.foreground = color;
                        } else {
                            self.current_style.background = color;
                        }
                        index += consumed;
                        continue;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn extended_color(&self, index: usize) -> Option<(TerminalColor, usize)> {
        let params = &self.csi_params[..self.csi_param_count];
        match params.get(index + 1)? {
            5 => Some((
                TerminalColor::Indexed {
                    index: (*params.get(index + 2)?).min(255) as u8,
                },
                3,
            )),
            2 => Some((
                TerminalColor::Rgb {
                    red: (*params.get(index + 2)?).min(255) as u8,
                    green: (*params.get(index + 3)?).min(255) as u8,
                    blue: (*params.get(index + 4)?).min(255) as u8,
                },
                5,
            )),
            _ => None,
        }
    }

    fn blank_cell(&self) -> Cell {
        Cell {
            style: TerminalCellStyle {
                background: self.current_style.background,
                ..TerminalCellStyle::default()
            },
            ..Cell::default()
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
            style: self.current_style,
            hyperlink_id: self.hyperlink_id,
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
        self.current_style = saved.style;
        self.hyperlink_id = saved.hyperlink_id;
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
            current_style: self.current_style,
            hyperlink_id: self.hyperlink_id,
            hyperlink_params: std::mem::take(&mut self.hyperlink_params),
            cursor_shape: self.cursor_shape,
            cursor_blinking: self.cursor_blinking,
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
        self.current_style = TerminalCellStyle::default();
        self.hyperlink_id = 0;
        self.cursor_shape = TerminalCursorShape::Block;
        self.cursor_blinking = true;
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
        self.current_style = saved.current_style;
        self.hyperlink_id = saved.hyperlink_id;
        self.hyperlink_params = saved.hyperlink_params;
        self.cursor_shape = saved.cursor_shape;
        self.cursor_blinking = saved.cursor_blinking;
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
        self.cursor_shape = TerminalCursorShape::Block;
        self.cursor_blinking = true;
        self.current_style = TerminalCellStyle::default();
        self.hyperlink_id = 0;
        self.hyperlink_params.clear();
        self.hyperlink_pool.clear();
        self.hyperlink_index.clear();
        self.hyperlink_pool_bytes = 0;
        self.bracketed_paste = false;
        self.mouse_modes = 0;
        self.focus_tracking = false;
        self.application_cursor_keys = false;
        self.application_keypad = false;
        self.keyboard_protocol = false;
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
        let blank = self.blank_cell();
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
            self.cells[base..base + usize::from(self.cols)].fill(blank);
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
            self.cells[base..base + usize::from(self.cols)].fill(blank);
        }
    }

    fn scroll_down(&mut self, top: u16, bottom: u16, requested: u16) {
        let blank = self.blank_cell();
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
            self.cells[base..base + usize::from(self.cols)].fill(blank);
        }
    }

    fn insert_cells(&mut self, requested: u16) {
        let blank = self.blank_cell();
        let count = requested.min(self.cols - self.cursor_col + 1);
        if count == 0 {
            return;
        }
        let base = self.row_base(self.cursor_row);
        let start = base + usize::from(self.cursor_col - 1);
        let end = base + usize::from(self.cols);
        self.cells
            .copy_within(start..end - usize::from(count), start + usize::from(count));
        self.cells[start..start + usize::from(count)].fill(blank);
        repair_wide_cells(&mut self.cells[base..end], self.cols, 1);
        self.pending_wrap = false;
        self.last_printable_idx = None;
    }

    fn delete_cells(&mut self, requested: u16) {
        let blank = self.blank_cell();
        let count = requested.min(self.cols - self.cursor_col + 1);
        if count == 0 {
            return;
        }
        let base = self.row_base(self.cursor_row);
        let start = base + usize::from(self.cursor_col - 1);
        let end = base + usize::from(self.cols);
        self.cells
            .copy_within(start + usize::from(count)..end, start);
        self.cells[end - usize::from(count)..end].fill(blank);
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
        let blank = self.blank_cell();
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
            self.cells[physical..physical + chunk].fill(blank);
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
                    self.cells[index - 1] = self.blank_cell();
                }
                self.cells[index] = self.blank_cell();
            }
            2 => {
                self.cells[index] = self.blank_cell();
                if col < self.cols && self.cells[index + 1].width == 0 {
                    self.cells[index + 1] = self.blank_cell();
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
        let base_bytes = char::from_u32(self.cells[cell_index].codepoint)
            .ok_or(TerminalGridError::InvalidCheckpoint)?
            .len_utf8();
        let available = MAX_CELL_TEXT_BYTES.saturating_sub(base_bytes + existing.len());
        let text = std::str::from_utf8(bytes).map_err(|_| TerminalGridError::InvalidCheckpoint)?;
        // This is only the bounded projection. The history owner has already
        // retained every raw byte; never split a scalar or invalidate the whole
        // screen just because one cell has more combining text than it can hold.
        let bytes = &bytes[..text.floor_char_boundary(available.min(bytes.len()))];
        if bytes.is_empty() {
            return Ok(());
        }
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

struct CheckpointWriter(Vec<u8>);

fn checkpoint_size_sum(parts: &[usize]) -> Result<usize, TerminalGridError> {
    parts
        .iter()
        .try_fold(0usize, |total, part| total.checked_add(*part))
        .ok_or(TerminalGridError::CheckpointTooLarge)
}

fn checkpoint_pool_bound(item_max: usize, total_max: usize) -> Result<usize, TerminalGridError> {
    // Every entry is nonempty and has its own u32 prefix, in addition to the
    // pool's u32 entry count. The suffix entry limit is tighter than its 4 MiB
    // payload cap: 65,535 x 64 bytes, not 4 MiB of payload plus free prefixes.
    let entries = MAX_SUFFIX_ENTRIES.min(total_max);
    let payload = entries
        .checked_mul(item_max)
        .ok_or(TerminalGridError::CheckpointTooLarge)?
        .min(total_max);
    let prefixes = entries
        .checked_mul(4)
        .ok_or(TerminalGridError::CheckpointTooLarge)?;
    checkpoint_size_sum(&[4, prefixes, payload])
}

impl CheckpointWriter {
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), TerminalGridError> {
        if bytes.len() > MAX_CHECKPOINT_BYTES.saturating_sub(self.0.len()) {
            return Err(TerminalGridError::CheckpointTooLarge);
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
    fn u8(&mut self, value: u8) -> Result<(), TerminalGridError> {
        self.bytes(&[value])
    }
    fn u16(&mut self, value: u16) -> Result<(), TerminalGridError> {
        self.bytes(&value.to_le_bytes())
    }
    fn u32(&mut self, value: usize) -> Result<(), TerminalGridError> {
        self.bytes(
            &u32::try_from(value)
                .map_err(|_| TerminalGridError::CheckpointTooLarge)?
                .to_le_bytes(),
        )
    }
    fn boolean(&mut self, value: bool) -> Result<(), TerminalGridError> {
        self.u8(u8::from(value))
    }
    fn sized(&mut self, value: &[u8]) -> Result<(), TerminalGridError> {
        self.u32(value.len())?;
        self.bytes(value)
    }
    fn color(&mut self, value: TerminalColor) -> Result<(), TerminalGridError> {
        match value {
            TerminalColor::Default => self.bytes(&[0, 0, 0, 0]),
            TerminalColor::Indexed { index } => self.bytes(&[1, index, 0, 0]),
            TerminalColor::Rgb { red, green, blue } => self.bytes(&[2, red, green, blue]),
        }
    }
    fn style(&mut self, style: TerminalCellStyle) -> Result<(), TerminalGridError> {
        self.color(style.foreground)?;
        self.color(style.background)?;
        let mut flags = 0;
        for (bit, enabled) in [
            style.bold,
            style.faint,
            style.italic,
            style.underline,
            style.inverse,
            style.strikethrough,
        ]
        .into_iter()
        .enumerate()
        {
            if enabled {
                flags |= 1 << bit;
            }
        }
        self.u8(flags)
    }
    fn cells(&mut self, cells: &[Cell]) -> Result<(), TerminalGridError> {
        self.u32(cells.len())?;
        for cell in cells {
            self.u32(cell.codepoint as usize)?;
            self.u8(cell.width)?;
            self.u32(cell.suffix_id as usize)?;
            self.style(cell.style)?;
            self.u32(cell.hyperlink_id as usize)?;
        }
        Ok(())
    }
    fn pool(&mut self, pool: &[Arc<[u8]>]) -> Result<(), TerminalGridError> {
        self.u32(pool.len())?;
        for item in pool {
            self.sized(item)?;
        }
        Ok(())
    }
}

struct CheckpointReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CheckpointReader<'a> {
    fn bytes(&mut self, length: usize) -> Result<&'a [u8], TerminalGridError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(TerminalGridError::InvalidCheckpoint)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(TerminalGridError::InvalidCheckpoint)?;
        self.offset = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8, TerminalGridError> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, TerminalGridError> {
        Ok(u16::from_le_bytes(
            self.bytes(2)?
                .try_into()
                .map_err(|_| TerminalGridError::InvalidCheckpoint)?,
        ))
    }
    fn u32(&mut self) -> Result<usize, TerminalGridError> {
        usize::try_from(u32::from_le_bytes(
            self.bytes(4)?
                .try_into()
                .map_err(|_| TerminalGridError::InvalidCheckpoint)?,
        ))
        .map_err(|_| TerminalGridError::InvalidCheckpoint)
    }
    fn boolean(&mut self) -> Result<bool, TerminalGridError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(TerminalGridError::InvalidCheckpoint),
        }
    }
    fn sized(&mut self, maximum: usize) -> Result<&'a [u8], TerminalGridError> {
        let size = self.u32()?;
        if size > maximum {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
        self.bytes(size)
    }
    fn color(&mut self) -> Result<TerminalColor, TerminalGridError> {
        match self.bytes(4)? {
            [0, 0, 0, 0] => Ok(TerminalColor::Default),
            [1, index, 0, 0] => Ok(TerminalColor::Indexed { index: *index }),
            [2, red, green, blue] => Ok(TerminalColor::Rgb {
                red: *red,
                green: *green,
                blue: *blue,
            }),
            _ => Err(TerminalGridError::InvalidCheckpoint),
        }
    }
    fn style(&mut self) -> Result<TerminalCellStyle, TerminalGridError> {
        let foreground = self.color()?;
        let background = self.color()?;
        let flags = self.u8()?;
        if flags & !0x3f != 0 {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
        Ok(TerminalCellStyle {
            foreground,
            background,
            bold: flags & 1 != 0,
            faint: flags & 2 != 0,
            italic: flags & 4 != 0,
            underline: flags & 8 != 0,
            inverse: flags & 16 != 0,
            strikethrough: flags & 32 != 0,
        })
    }
    fn cells(&mut self, count: usize) -> Result<Vec<Cell>, TerminalGridError> {
        if self.u32()? != count || count > self.bytes.len().saturating_sub(self.offset) / 22 {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
        let mut cells = Vec::with_capacity(count);
        for _ in 0..count {
            let codepoint =
                u32::try_from(self.u32()?).map_err(|_| TerminalGridError::InvalidCheckpoint)?;
            let width = self.u8()?;
            let suffix_id =
                u32::try_from(self.u32()?).map_err(|_| TerminalGridError::InvalidCheckpoint)?;
            let style = self.style()?;
            let hyperlink_id =
                u32::try_from(self.u32()?).map_err(|_| TerminalGridError::InvalidCheckpoint)?;
            cells.push(Cell {
                codepoint,
                width,
                suffix_id,
                style,
                hyperlink_id,
            });
        }
        Ok(cells)
    }
    fn pool(
        &mut self,
        item_max: usize,
        total_max: usize,
    ) -> Result<Vec<Arc<[u8]>>, TerminalGridError> {
        let count = self.u32()?;
        if count > MAX_SUFFIX_ENTRIES || count > self.bytes.len().saturating_sub(self.offset) / 5 {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
        let mut pool = Vec::with_capacity(count);
        let mut total = 0usize;
        for _ in 0..count {
            let item = self.sized(item_max)?;
            if item.is_empty() || item.len() > total_max.saturating_sub(total) {
                return Err(TerminalGridError::InvalidCheckpoint);
            }
            total += item.len();
            pool.push(Arc::from(item));
        }
        Ok(pool)
    }
    fn screen(&mut self, count: usize) -> Result<SavedScreen, TerminalGridError> {
        let cells = self.cells(count)?;
        let row_origin = self.u16()?;
        let cursor_row = self.u16()?;
        let cursor_col = self.u16()?;
        let scroll_top = self.u16()?;
        let scroll_bottom = self.u16()?;
        let autowrap = self.boolean()?;
        let pending_wrap = self.boolean()?;
        let origin_mode = self.boolean()?;
        let insert_mode = self.boolean()?;
        let current_style = self.style()?;
        let hyperlink_id =
            u32::try_from(self.u32()?).map_err(|_| TerminalGridError::InvalidCheckpoint)?;
        let hyperlink_params = self.sized(MAX_CONTROL_STRING_BYTES)?.to_vec();
        let cursor_shape = match self.u8()? {
            0 => TerminalCursorShape::Block,
            1 => TerminalCursorShape::Underline,
            2 => TerminalCursorShape::Bar,
            _ => return Err(TerminalGridError::InvalidCheckpoint),
        };
        let cursor_blinking = self.boolean()?;
        let saved_cursor = if self.boolean()? {
            Some(SavedCursor {
                row: self.u16()?,
                col: self.u16()?,
                pending_wrap: self.boolean()?,
                origin_mode: self.boolean()?,
                style: self.style()?,
                hyperlink_id: u32::try_from(self.u32()?)
                    .map_err(|_| TerminalGridError::InvalidCheckpoint)?,
            })
        } else {
            None
        };
        let last_printable_idx = if self.boolean()? {
            Some(self.u32()?)
        } else {
            None
        };
        Ok(SavedScreen {
            cells,
            row_origin,
            cursor_row,
            cursor_col,
            autowrap,
            pending_wrap,
            scroll_top,
            scroll_bottom,
            origin_mode,
            insert_mode,
            saved_cursor,
            last_printable_idx,
            current_style,
            hyperlink_id,
            hyperlink_params,
            cursor_shape,
            cursor_blinking,
        })
    }
}

fn shape_code(shape: TerminalCursorShape) -> u8 {
    match shape {
        TerminalCursorShape::Block => 0,
        TerminalCursorShape::Underline => 1,
        TerminalCursorShape::Bar => 2,
    }
}

fn validate_pool(
    pool: &[Arc<[u8]>],
    item_max: usize,
    total_max: usize,
) -> Result<(), TerminalGridError> {
    if pool.len() > MAX_SUFFIX_ENTRIES {
        return Err(TerminalGridError::InvalidCheckpoint);
    }
    let mut total = 0usize;
    for item in pool {
        if item.is_empty() || item.len() > item_max || item.len() > total_max.saturating_sub(total)
        {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
        total += item.len();
    }
    Ok(())
}

fn index_pool(pool: &[Arc<[u8]>]) -> Result<HashMap<Arc<[u8]>, u32>, TerminalGridError> {
    let mut index = HashMap::with_capacity(pool.len());
    for (position, item) in pool.iter().enumerate() {
        if index
            .insert(
                Arc::clone(item),
                u32::try_from(position + 1).map_err(|_| TerminalGridError::InvalidCheckpoint)?,
            )
            .is_some()
        {
            return Err(TerminalGridError::InvalidCheckpoint);
        }
    }
    Ok(index)
}

fn validate_cells(
    cells: &[Cell],
    cols: u16,
    rows: u16,
    suffixes: &[Arc<[u8]>],
    hyperlink_count: usize,
) -> Result<(), TerminalGridError> {
    let invalid = TerminalGridError::InvalidCheckpoint;
    if cells.len() != usize::from(cols) * usize::from(rows) {
        return Err(invalid);
    }
    for (index, cell) in cells.iter().enumerate() {
        if cell.suffix_id as usize > suffixes.len() || cell.hyperlink_id as usize > hyperlink_count
        {
            return Err(invalid);
        }
        if cell.codepoint != 0
            && (cell.codepoint < 0x20 || char::from_u32(cell.codepoint).is_none())
        {
            return Err(invalid);
        }
        if let Some(base) = char::from_u32(cell.codepoint) {
            let suffix_bytes = cell
                .suffix_id
                .checked_sub(1)
                .map_or(0, |index| suffixes[index as usize].len());
            if base.len_utf8() + suffix_bytes > MAX_CELL_TEXT_BYTES {
                return Err(invalid);
            }
        }
        match cell.width {
            0 => {
                if index % usize::from(cols) == 0
                    || cell.codepoint != 0
                    || cell.suffix_id != 0
                    || cells[index - 1].width != 2
                    || cells[index - 1].style != cell.style
                    || cells[index - 1].hyperlink_id != cell.hyperlink_id
                {
                    return Err(invalid);
                }
            }
            1 => {
                if cell.codepoint == 0 {
                    return Err(invalid);
                }
            }
            2 => {
                if index % usize::from(cols) + 1 >= usize::from(cols)
                    || cell.codepoint == 0
                    || cells[index + 1].width != 0
                    || cells[index + 1].style != cell.style
                    || cells[index + 1].hyperlink_id != cell.hyperlink_id
                {
                    return Err(invalid);
                }
            }
            _ => return Err(invalid),
        }
    }
    Ok(())
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

    fn maximal_checkpoint_grid() -> TerminalGrid {
        let mut grid = test_grid(1, 1);
        grid.feed(b"x\x1b7\x1b[?1049hx\x1b7").unwrap();
        grid.hyperlink_params = vec![b'p'; MAX_CONTROL_STRING_BYTES];
        grid.saved_normal_screen.as_mut().unwrap().hyperlink_params =
            vec![b'q'; MAX_CONTROL_STRING_BYTES];
        grid.osc_buffer = vec![b'o'; MAX_CONTROL_STRING_BYTES];
        grid.dcs_buffer = vec![b'd'; MAX_CONTROL_STRING_BYTES];
        grid.sync_active = true;
        grid.sync_buffer = vec![b's'; MAX_SYNC_BYTES];
        grid.csi_params = [u16::MAX; MAX_CSI_PARAMS];
        grid.csi_param_count = MAX_CSI_PARAMS;
        grid.csi_intermediates = [b' '; MAX_CSI_INTERMEDIATES];
        grid.csi_intermediate_count = MAX_CSI_INTERMEDIATES;
        grid.utf8_buffer = [0xf0, 0x90, 0x80, 0];
        grid.utf8_len = 3;
        grid.utf8_expected = 4;
        // Distinct, valid restore inputs reach both entry-count limits. Suffix
        // payloads reach their per-entry cap; links use the remaining 64 bytes
        // of their independent pool cap in their first entry.
        grid.suffix_pool = (0..MAX_SUFFIX_ENTRIES)
            .map(|index| Arc::from(format!("{index:064}").into_bytes()))
            .collect();
        grid.suffix_pool_bytes = MAX_SUFFIX_ENTRIES * MAX_CELL_TEXT_BYTES;
        grid.hyperlink_pool.clone_from(&grid.suffix_pool);
        grid.hyperlink_pool[0] = Arc::from(vec![b'l'; MAX_CELL_TEXT_BYTES + 64]);
        grid.hyperlink_pool_bytes = MAX_HYPERLINK_POOL_BYTES;
        grid
    }

    #[test]
    fn checkpoint_bound_covers_actual_maxima_at_tiny_normal_and_max_dimensions() {
        let mut grid = maximal_checkpoint_grid();
        for (cols, rows) in [(1, 1), (80, 24), (MAX_DIMENSION, 64)] {
            grid.resize(cols, rows).unwrap();
            grid.last_printable_idx = Some(0);
            grid.saved_normal_screen
                .as_mut()
                .unwrap()
                .last_printable_idx = Some(0);
            let bound = TerminalGrid::checkpoint_bound(cols, rows).unwrap();
            assert_eq!(
                bound,
                9_978_007 + 44 * usize::from(cols) * usize::from(rows) + usize::from(cols)
            );
            assert!(bound < MAX_CHECKPOINT_BYTES);
            let encoded = grid.checkpoint().unwrap();
            // Equality locks the formula to every actual encoder field/prefix,
            // not merely a loose estimate based on in-memory struct layouts.
            assert_eq!(encoded.len(), bound);
            let restored = TerminalGrid::restore(&encoded).unwrap();
            assert_eq!(restored.checkpoint().unwrap(), encoded);
        }
    }

    #[test]
    fn checkpoint_wire_widths_and_pool_prefixes_match_the_bound() {
        let mut writer = CheckpointWriter(Vec::new());
        let style = TerminalCellStyle {
            foreground: TerminalColor::Rgb {
                red: 1,
                green: 2,
                blue: 3,
            },
            background: TerminalColor::Indexed { index: 255 },
            bold: true,
            faint: true,
            italic: true,
            underline: true,
            inverse: true,
            strikethrough: true,
        };
        writer.style(style).unwrap();
        assert_eq!(writer.0.len(), CHECKPOINT_STYLE_BYTES);
        writer.0.clear();
        writer
            .cells(&[Cell {
                style,
                ..Cell::default()
            }])
            .unwrap();
        assert_eq!(writer.0.len(), 4 + CHECKPOINT_CELL_BYTES);
        writer.0.clear();
        writer
            .pool(&[Arc::from(&b"one"[..]), Arc::from(&b"two"[..])])
            .unwrap();
        assert_eq!(writer.0.len(), 4 + 2 * 4 + 6);
        assert_eq!(
            checkpoint_pool_bound(MAX_CELL_TEXT_BYTES, MAX_SUFFIX_POOL_BYTES).unwrap(),
            4 + MAX_SUFFIX_ENTRIES * (4 + MAX_CELL_TEXT_BYTES)
        );
        assert_eq!(
            checkpoint_pool_bound(MAX_CONTROL_STRING_BYTES, MAX_HYPERLINK_POOL_BYTES).unwrap(),
            4 + MAX_SUFFIX_ENTRIES * 4 + MAX_HYPERLINK_POOL_BYTES
        );
    }

    #[test]
    fn checkpoint_bound_rejects_invalid_dimensions_and_checked_overflow() {
        for (cols, rows) in [(0, 1), (1, 0), (4097, 1), (1, 4097), (4096, 65)] {
            assert!(TerminalGrid::checkpoint_bound(cols, rows).is_err());
        }
        assert_eq!(
            checkpoint_size_sum(&[usize::MAX, 1]),
            Err(TerminalGridError::CheckpointTooLarge)
        );
        assert_eq!(
            checkpoint_pool_bound(usize::MAX, usize::MAX),
            Err(TerminalGridError::CheckpointTooLarge)
        );
        assert_eq!(
            TerminalGrid::checkpoint_bound(4096, 64).unwrap(),
            21_516_439
        );
        assert!(TerminalGrid::checkpoint_bound(64, 4096).unwrap() < 21_516_439);
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
    fn complete_cell_bound_rejects_oversized_live_and_restored_cells() {
        let mut grid = test_grid(8, 2);
        let text = format!("a{}\u{20d0}", "\u{0301}".repeat(30));
        assert_eq!(text.len(), MAX_CELL_TEXT_BYTES);
        grid.feed(text.as_bytes()).unwrap();
        let mut checkpoint = grid.checkpoint().unwrap();
        assert_eq!(grid.structured_screen().unwrap().cells[0].text, text);

        // The first cell's scalar follows magic, dimensions and cell count.
        let scalar_offset = 6 + 2 + 2 + 4;
        assert_eq!(
            &checkpoint[scalar_offset..scalar_offset + 4],
            &u32::from('a').to_le_bytes()
        );
        checkpoint[scalar_offset..scalar_offset + 4].copy_from_slice(&u32::from('é').to_le_bytes());
        assert!(matches!(
            TerminalGrid::restore(&checkpoint),
            Err(TerminalGridError::InvalidCheckpoint)
        ));
        grid.cells[0].codepoint = u32::from('é');
        assert!(matches!(
            grid.checkpoint(),
            Err(TerminalGridError::InvalidCheckpoint)
        ));
        // Saved normal screens use the same complete-cell validation.
        grid.enter_alternate_screen();
        assert!(matches!(
            grid.checkpoint(),
            Err(TerminalGridError::InvalidCheckpoint)
        ));

        grid.saved_normal_screen.as_mut().unwrap().cells[0].codepoint = u32::from('a');
        let mut saved_checkpoint = grid.checkpoint().unwrap();
        let saved_scalar = saved_checkpoint
            .windows(4)
            .rposition(|window| window == u32::from('a').to_le_bytes())
            .unwrap();
        saved_checkpoint[saved_scalar..saved_scalar + 4]
            .copy_from_slice(&u32::from('é').to_le_bytes());
        assert!(matches!(
            TerminalGrid::restore(&saved_checkpoint),
            Err(TerminalGridError::InvalidCheckpoint)
        ));
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
    fn structured_styles_palette_rgb_flags_and_erasure_background() {
        let mut grid = test_grid(8, 2);
        grid.feed(
            b"\x1b[1;2;3;4;7;9;38;5;196;48;2;12;34;56mA\x1b[22;23;24;27;29mB\x1b[0mC\x1b[44m\x1b[K",
        )
        .unwrap();
        let screen = grid.structured_screen().unwrap();
        let style = screen.cells[0].style;
        assert_eq!(style.foreground, TerminalColor::Indexed { index: 196 });
        assert_eq!(
            style.background,
            TerminalColor::Rgb {
                red: 12,
                green: 34,
                blue: 56
            }
        );
        assert!(
            style.bold
                && style.faint
                && style.italic
                && style.underline
                && style.inverse
                && style.strikethrough
        );
        assert!(
            !screen.cells[1].style.bold
                && !screen.cells[1].style.faint
                && !screen.cells[1].style.italic
                && !screen.cells[1].style.underline
                && !screen.cells[1].style.inverse
                && !screen.cells[1].style.strikethrough
        );
        assert_eq!(screen.cells[2].style, TerminalCellStyle::default());
        assert_eq!(screen.cells[3].kind, TerminalCellKind::Blank);
        assert_eq!(
            screen.cells[3].style.background,
            TerminalColor::Indexed { index: 4 }
        );
        assert!(screen.cells[3].text.is_empty());
        assert_eq!(snapshot(&grid), "|ABC     |\n|        |\n");
    }

    #[test]
    fn structured_modes_cursor_shape_wide_cells_and_hyperlinks() {
        let mut grid = test_grid(8, 2);
        grid.feed(b"\x1b[?1;1000;1006;1004;2004h\x1b=\x1b[>1u\x1b[6 q\x1b]8;id=a;https://example.test\x07").unwrap();
        grid.feed("界\x1b[0m!\x1b]8;;\x1b\\?".as_bytes()).unwrap();
        let screen = grid.structured_screen().unwrap();
        assert!(
            screen.modes.application_cursor_keys
                && screen.modes.application_keypad
                && screen.modes.keyboard_protocol
                && screen.modes.bracketed_paste
                && screen.modes.mouse_tracking
                && screen.modes.focus_tracking
        );
        assert_eq!(screen.cursor.shape, TerminalCursorShape::Bar);
        assert!(!screen.cursor.blinking);
        assert_eq!(screen.cells[0].kind, TerminalCellKind::Wide);
        assert_eq!(screen.cells[1].kind, TerminalCellKind::Continuation);
        assert!(screen.cells[1].text.is_empty());
        assert_eq!(screen.cells[0].hyperlink_id, Some(1));
        assert_eq!(screen.cells[1].hyperlink_id, Some(1));
        assert_eq!(screen.cells[2].hyperlink_id, Some(1));
        assert_eq!(screen.cells[3].hyperlink_id, None);
        assert_eq!(screen.hyperlinks.len(), 1);
        assert_eq!(screen.hyperlinks[0].uri, b"https://example.test");
        assert_eq!(grid.hyperlink_at(0, 0), Some(&b"https://example.test"[..]));
        grid.feed(b"\x1b[?1000l").unwrap();
        assert!(grid.modes().mouse_tracking);
        grid.feed(b"\x1b[?1006l\x1b[<u\x1b>").unwrap();
        assert!(
            !grid.modes().mouse_tracking
                && !grid.modes().keyboard_protocol
                && !grid.modes().application_keypad
        );
    }

    #[test]
    fn live_replies_are_explicit_bounded_and_suppressed_during_replay() {
        let queries = b"\x1b[5n\x1b[6n\x1b[?6n\x1b[c\x1b[>c\x1b[18t\x1bP$qm\x1b\\";
        let mut grid = test_grid(80, 24);
        grid.feed(queries).unwrap();
        assert!(grid.take_replies().is_empty());
        grid.feed_live(queries).unwrap();
        assert_eq!(
            grid.take_replies(),
            vec![
                b"\x1b[0n".to_vec(),
                b"\x1b[1;1R".to_vec(),
                b"\x1b[?1;1R".to_vec(),
                b"\x1b[?1;2c".to_vec(),
                b"\x1b[>0;0;0c".to_vec(),
                b"\x1b[8;24;80t".to_vec(),
                b"\x1bP1$r0m\x1b\\".to_vec()
            ]
        );
        grid.feed_live(&b"\x1b[5n".repeat(16)).unwrap();
        assert_eq!(
            grid.feed_live(b"\x1b[5n"),
            Err(TerminalGridError::ReplyCapacityExceeded)
        );
        assert_eq!(grid.take_replies().len(), 16);
        // Failed live feeds must never leave permission to emit on replay.
        grid.feed(b"\x18\x1b[5n").unwrap();
        assert!(grid.take_replies().is_empty());
    }

    #[test]
    fn checkpoint_restores_every_fragmented_control_and_unicode_boundary() {
        let payload="A\u{301}界\x1b[38;2;12;34;56mB\x1b]8;id=x;https://x\x1b\\C\x1bP$qm\x1b\\\x1b[?2004h\x1b[?2026h\rnew\x1b[?2026l".as_bytes();
        for split in 0..=payload.len() {
            let mut original = test_grid(12, 3);
            original.feed(&payload[..split]).unwrap();
            let checkpoint = original.checkpoint().unwrap();
            assert!(checkpoint.len() <= TerminalGrid::checkpoint_bound(12, 3).unwrap());
            let mut restored = TerminalGrid::restore(&checkpoint).unwrap();
            assert_eq!(restored.checkpoint().unwrap(), checkpoint, "split {split}");
            original.feed(&payload[split..]).unwrap();
            restored.feed(&payload[split..]).unwrap();
            assert_eq!(
                original.snapshot().unwrap(),
                restored.snapshot().unwrap(),
                "split {split}"
            );
            assert_eq!(
                original.structured_screen().unwrap(),
                restored.structured_screen().unwrap(),
                "split {split}"
            );
            assert_eq!(
                original.checkpoint().unwrap(),
                restored.checkpoint().unwrap(),
                "split {split}"
            );
            assert!(restored.take_replies().is_empty());
        }
    }

    #[test]
    fn checkpoint_alternate_screen_preserves_styles_cursor_and_suffixes() {
        let mut grid = test_grid(8, 3);
        grid.feed("\x1b[31mN\u{301}\x1b7\x1b[?1049h\x1b[32malt\x1b[4 q".as_bytes())
            .unwrap();
        let mut restored = TerminalGrid::restore(&grid.checkpoint().unwrap()).unwrap();
        grid.resize(6, 2).unwrap();
        restored.resize(6, 2).unwrap();
        for candidate in [&mut grid, &mut restored] {
            candidate.feed(b"\x1b[?1049l\x1b8!").unwrap();
        }
        assert_eq!(
            grid.structured_screen().unwrap(),
            restored.structured_screen().unwrap()
        );
        assert_eq!(snapshot(&restored), "|N\u{301}!    |\n|      |\n");
        assert_eq!(
            restored.structured_screen().unwrap().cells[1]
                .style
                .foreground,
            TerminalColor::Indexed { index: 1 }
        );
    }

    #[test]
    fn corrupt_checkpoint_rejects_truncation_sizes_versions_and_invalid_cells() {
        let mut grid = test_grid(4, 2);
        grid.feed(b"test").unwrap();
        let bytes = grid.checkpoint().unwrap();
        for end in 0..bytes.len() {
            assert!(
                TerminalGrid::restore(&bytes[..end]).is_err(),
                "truncation {end}"
            );
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(TerminalGrid::restore(&extra).is_err());
        for (offset, value) in [(0, b'X'), (4, 2), (6, 0), (7, 0xff), (10, 0xff), (18, 3)] {
            let mut corrupt = bytes.clone();
            corrupt[offset] = value;
            assert!(TerminalGrid::restore(&corrupt).is_err(), "offset {offset}");
        }
        let mut grid = test_grid(2, 1);
        grid.feed("界".as_bytes()).unwrap();
        grid.cells[1].style.bold = true;
        assert!(grid.checkpoint().is_err());
        let mut grid = test_grid(2, 1);
        grid.feed(b"a").unwrap();
        grid.suffix_pool.push(Arc::from(&b"\x1b"[..]));
        grid.cells[0].suffix_id = 1;
        assert!(grid.checkpoint().is_err());
    }

    #[test]
    fn checkpoints_drop_old_reply_effects_and_restore_partial_live_queries() {
        let mut grid = test_grid(8, 3);
        grid.feed_live(b"\x1b[5n\x1bP$q").unwrap();
        let mut restored = TerminalGrid::restore(&grid.checkpoint().unwrap()).unwrap();
        assert!(restored.take_replies().is_empty());
        restored.feed_live(b"r\x1b\\").unwrap();
        assert_eq!(
            restored.take_replies(),
            vec![b"\x1bP1$r1;3r\x1b\\".to_vec()]
        );
    }

    #[test]
    fn cursor_reports_remain_bounded_after_absolute_position_in_origin_mode() {
        let mut grid = test_grid(8, 4);
        grid.feed_live(b"\x1b[2;4r\x1b[?6h\x1b[1d\x1b[6n").unwrap();
        assert_eq!(grid.take_replies(), vec![b"\x1b[1;1R".to_vec()]);
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
