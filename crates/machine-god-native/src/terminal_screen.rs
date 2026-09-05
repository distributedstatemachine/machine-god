//! Reusable live/recovery screen projection with explicit reply ownership.

use std::fmt;

use machine_god_core::{
    TerminalDimensions, TerminalModes, TerminalScreen, TerminalScreenUnavailableReason,
};

use crate::terminal_grid::TerminalGrid;

/// Maximum raw bytes processed by one screen-feed operation.
pub const MAX_TERMINAL_SCREEN_FEED_BYTES: usize = 64 * 1024;

/// Whether terminal protocol replies may be produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalScreenMode {
    /// Raw history reconstruction never produces protocol effects.
    Replay,
    /// The caller explicitly owns dispatching each returned reply once.
    Live,
}

/// Fixed screen-projection failure with no raw output or checkpoint contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalScreenError {
    /// Input dimensions or feed size violate the bounded contract.
    InvalidInput,
    /// A coherent screen cannot be produced from the available evidence.
    Unavailable(TerminalScreenUnavailableReason),
}

impl fmt::Display for TerminalScreenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal screen projection unavailable")
    }
}

impl std::error::Error for TerminalScreenError {}

/// Stateful screen model shared by live terminal execution and durable recovery.
///
/// This type performs no I/O. Protocol replies are returned only in live mode;
/// native process ownership and exactly-once dispatch remain the caller's job.
pub struct TerminalScreenEngine {
    grid: Option<TerminalGrid>,
    mode: TerminalScreenMode,
    unavailable: TerminalScreenUnavailableReason,
}

impl fmt::Debug for TerminalScreenEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalScreenEngine")
            .finish_non_exhaustive()
    }
}

impl TerminalScreenEngine {
    /// Bounds the serialized grid checkpoint at these dimensions, including
    /// both screens, retained pools and parser buffers. This is a true bound
    /// below the encoder cap, not a guarantee that corrupt/unavailable state
    /// can be checkpointed. It excludes the history owner's outer envelope.
    #[allow(
        dead_code,
        reason = "private profile-reservation integration is staged separately"
    )]
    pub(crate) fn checkpoint_bound(
        dimensions: &TerminalDimensions,
    ) -> Result<usize, TerminalScreenError> {
        dimensions
            .validate()
            .map_err(|_| TerminalScreenError::InvalidInput)?;
        TerminalGrid::checkpoint_bound(dimensions.columns(), dimensions.rows())
            .map_err(|_| TerminalScreenError::InvalidInput)
    }

    /// Creates an empty screen from validated dimensions without native effects.
    ///
    /// # Errors
    /// Rejects invalid or unrepresentable dimensions.
    pub fn new(
        dimensions: &TerminalDimensions,
        mode: TerminalScreenMode,
    ) -> Result<Self, TerminalScreenError> {
        dimensions
            .validate()
            .map_err(|_| TerminalScreenError::InvalidInput)?;
        let grid = TerminalGrid::new(dimensions.columns(), dimensions.rows())
            .map_err(|_| TerminalScreenError::InvalidInput)?;
        Ok(Self {
            grid: Some(grid),
            mode,
            unavailable: TerminalScreenUnavailableReason::Missing,
        })
    }

    /// Restores a validated native checkpoint. Prior replies are never replayed.
    ///
    /// The journal owner must also verify checkpoint integrity and its source
    /// cursor, then feed only contiguous later raw output. A gap invalidates the
    /// reconstruction; this constructor cannot authenticate an external cursor.
    ///
    /// # Errors
    /// Rejects corrupt, unsupported or oversized checkpoint state.
    pub fn restore(
        checkpoint: &[u8],
        mode: TerminalScreenMode,
    ) -> Result<Self, TerminalScreenError> {
        let grid = TerminalGrid::restore(checkpoint).map_err(|_| {
            TerminalScreenError::Unavailable(TerminalScreenUnavailableReason::Corrupt)
        })?;
        Ok(Self {
            grid: Some(grid),
            mode,
            unavailable: TerminalScreenUnavailableReason::Missing,
        })
    }

    /// Processes a bounded raw chunk and returns live replies for explicit dispatch.
    ///
    /// # Errors
    /// Oversized input has no effect. A parser/resource failure invalidates the
    /// whole projection, suppressing partial replies and invented later screens.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, TerminalScreenError> {
        if bytes.len() > MAX_TERMINAL_SCREEN_FEED_BYTES {
            return Err(TerminalScreenError::InvalidInput);
        }
        let error = TerminalScreenError::Unavailable(self.unavailable);
        let grid = self.grid.as_mut().ok_or(error)?;
        let result = match self.mode {
            TerminalScreenMode::Live => grid.feed_live(bytes),
            TerminalScreenMode::Replay => grid.feed(bytes),
        };
        if result.is_err() {
            self.invalidate(TerminalScreenUnavailableReason::Corrupt);
            return Err(TerminalScreenError::Unavailable(self.unavailable));
        }
        Ok(grid.take_replies())
    }

    /// Marks missing/disconnected evidence without inventing a replacement screen.
    pub fn invalidate(&mut self, reason: TerminalScreenUnavailableReason) {
        self.grid = None;
        self.unavailable = reason;
    }

    /// Resizes the projection. The caller must separately resize the actual PTY.
    ///
    /// # Errors
    /// Rejects invalid dimensions or unavailable screen state.
    pub fn resize(&mut self, dimensions: &TerminalDimensions) -> Result<(), TerminalScreenError> {
        dimensions
            .validate()
            .map_err(|_| TerminalScreenError::InvalidInput)?;
        let error = TerminalScreenError::Unavailable(self.unavailable);
        self.grid
            .as_mut()
            .ok_or(error)?
            .resize(dimensions.columns(), dimensions.rows())
            .map_err(|_| TerminalScreenError::InvalidInput)
    }

    /// Returns a structured styled screen without process effects.
    ///
    /// # Errors
    /// Returns explicit unavailability if complete evidence is absent.
    pub fn screen(&self) -> Result<TerminalScreen, TerminalScreenError> {
        self.grid()?
            .structured_screen()
            .map_err(|_| TerminalScreenError::Unavailable(TerminalScreenUnavailableReason::Corrupt))
    }

    /// Returns the modes used by input key/paste encoding.
    ///
    /// # Errors
    /// Returns explicit unavailability when input-mode evidence is missing.
    pub fn modes(&self) -> Result<TerminalModes, TerminalScreenError> {
        Ok(self.grid()?.modes())
    }

    /// Returns data-only hyperlink bytes at a zero-based cell, never opening a URI.
    ///
    /// # Errors
    /// Returns explicit unavailability when the screen is unavailable.
    pub fn hyperlink(&self, row: u16, column: u16) -> Result<Option<&[u8]>, TerminalScreenError> {
        Ok(self.grid()?.hyperlink_at(row, column))
    }

    /// Serializes a bounded native checkpoint for a journal-bound source cursor.
    ///
    /// # Errors
    /// Rejects unavailable or nonrepresentable projection state.
    pub fn checkpoint(&self) -> Result<Vec<u8>, TerminalScreenError> {
        self.grid()?
            .checkpoint()
            .map_err(|_| TerminalScreenError::Unavailable(TerminalScreenUnavailableReason::Corrupt))
    }

    fn grid(&self) -> Result<&TerminalGrid, TerminalScreenError> {
        self.grid
            .as_ref()
            .ok_or(TerminalScreenError::Unavailable(self.unavailable))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_checkpoint_bound_tracks_dimensions_and_fragmented_parser_state() {
        let payload = b"x\x1b7\x1b[?1049halt\x1b7\x1b]8;id=x;https://x\x1b\\\x1b[38;2;1;2;3m\xf0\x90\x80\x80\x1bP$qm\x1b\\\x1b[?2026hbuffered";
        for dimensions in [
            TerminalDimensions::new(1, 1).unwrap(),
            TerminalDimensions::new(24, 80).unwrap(),
            TerminalDimensions::new(64, 4096).unwrap(),
        ] {
            let bound = TerminalScreenEngine::checkpoint_bound(&dimensions).unwrap();
            assert_eq!(
                bound,
                TerminalGrid::checkpoint_bound(dimensions.columns(), dimensions.rows()).unwrap()
            );
            let mut screen =
                TerminalScreenEngine::new(&dimensions, TerminalScreenMode::Replay).unwrap();
            let chunk_size = if dimensions.columns() == 4096 {
                payload.len()
            } else {
                1
            };
            for chunk in payload.chunks(chunk_size) {
                screen.feed(chunk).unwrap();
                assert!(screen.checkpoint().unwrap().len() <= bound);
            }
            let restored = TerminalScreenEngine::restore(
                &screen.checkpoint().unwrap(),
                TerminalScreenMode::Replay,
            )
            .unwrap();
            assert!(restored.checkpoint().unwrap().len() <= bound);
        }
    }

    #[test]
    fn live_and_restored_replay_screens_agree_without_replaying_effects() {
        let mut live = TerminalScreenEngine::new(
            &TerminalDimensions::new(3, 12).unwrap(),
            TerminalScreenMode::Live,
        )
        .unwrap();
        assert_eq!(live.feed(b"\x1b[6n").unwrap(), vec![b"\x1b[1;1R".to_vec()]);
        live.feed(b"\x1b[31mred\x1b[0m \xe2").unwrap();
        let mut replay =
            TerminalScreenEngine::restore(&live.checkpoint().unwrap(), TerminalScreenMode::Replay)
                .unwrap();
        let suffix = b"\x98\x83\x1b[6n";
        assert!(!live.feed(suffix).unwrap().is_empty());
        assert!(replay.feed(suffix).unwrap().is_empty());
        assert_eq!(live.screen().unwrap(), replay.screen().unwrap());
        assert_eq!(live.modes().unwrap(), replay.modes().unwrap());
        live.resize(&TerminalDimensions::new(4, 20).unwrap())
            .unwrap();
        assert_eq!(
            live.screen().unwrap().dimensions,
            TerminalDimensions::new(4, 20).unwrap()
        );
        assert_eq!(live.hyperlink(0, 0).unwrap(), None);
    }

    #[test]
    fn evidence_gaps_and_bad_feeds_never_invent_a_screen() {
        let mut engine = TerminalScreenEngine::new(
            &TerminalDimensions::new(2, 8).unwrap(),
            TerminalScreenMode::Live,
        )
        .unwrap();
        let before = engine.checkpoint().unwrap();
        assert_eq!(
            engine.feed(&vec![b'x'; MAX_TERMINAL_SCREEN_FEED_BYTES + 1]),
            Err(TerminalScreenError::InvalidInput)
        );
        assert_eq!(engine.checkpoint().unwrap(), before);
        engine.invalidate(TerminalScreenUnavailableReason::RawGap);
        assert_eq!(
            engine.screen(),
            Err(TerminalScreenError::Unavailable(
                TerminalScreenUnavailableReason::RawGap
            ))
        );
        assert!(engine.feed(b"later").is_err());
        assert!(engine.checkpoint().is_err());
        assert!(
            TerminalScreenEngine::restore(b"bad checkpoint", TerminalScreenMode::Replay).is_err()
        );
        assert_eq!(format!("{engine:?}"), "TerminalScreenEngine { .. }");
    }
}
