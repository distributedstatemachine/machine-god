//! One owner for committed terminal output and its screen projection.
//!
//! Synchronous descriptor operations belong on a bounded native worker. This
//! component grants no process authority and never dispatches protocol replies.

use std::fmt;

use machine_god_core::{
    TerminalCursor, TerminalDimensions, TerminalModes, TerminalScreen,
    TerminalScreenUnavailableReason as Unavailable, TerminalSessionId,
};

use crate::terminal_journal::{
    TerminalJournal, TerminalJournalCheckpoint, TerminalJournalCheckpointStatus,
    TerminalJournalError, TerminalJournalEviction, TerminalJournalPage,
    TerminalJournalPhysicalUsage,
};
use crate::terminal_screen::{
    MAX_TERMINAL_SCREEN_FEED_BYTES, TerminalScreenEngine, TerminalScreenError, TerminalScreenMode,
};

const MAGIC: &[u8; 8] = b"MGTH\0\0\0\x01";
const GRID: u8 = 0;
const RAW_GAP: u8 = 1;
const RESIZE_PENDING: u8 = 2;
// The journal retains at most 64 MiB of raw/checkpoint output, independently
// of protected state and events. Page reads are at most 64 KiB;
// segment boundaries may add at most 128 short pages.
const MAX_REPLAY_PAGES: usize = 1024 + 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalHistoryError {
    ReadOnly,
    Journal(TerminalJournalError),
    Screen(TerminalScreenError),
    NativeResize,
}
impl fmt::Display for TerminalHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("terminal history operation unavailable")
    }
}
impl std::error::Error for TerminalHistoryError {}
impl From<TerminalJournalError> for TerminalHistoryError {
    fn from(error: TerminalJournalError) -> Self {
        Self::Journal(error)
    }
}
impl From<TerminalScreenError> for TerminalHistoryError {
    fn from(error: TerminalScreenError) -> Self {
        Self::Screen(error)
    }
}
type Result<T> = std::result::Result<T, TerminalHistoryError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalHistoryEviction {
    CompletedOutput,
    CompletedCheckpoint,
    LiveCoveredOutput,
}

/// Raw commitment is independent of projection availability. An unavailable
/// screen must not turn a successful raw append into an ambiguous retry.
pub(crate) struct TerminalHistoryAppend {
    pub(crate) cursor: TerminalCursor,
    pub(crate) replies: Vec<Vec<u8>>,
    pub(crate) screen_unavailable: Option<Unavailable>,
}
impl fmt::Debug for TerminalHistoryAppend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalHistoryAppend")
            .finish_non_exhaustive()
    }
}

pub(crate) struct TerminalHistory {
    journal: TerminalJournal,
    screen: Option<TerminalScreenEngine>,
    unavailable: Unavailable,
    live: bool,
}

/// An unavailable checkpoint is already committed while this guard exists.
/// Dropping it before `finish` preserves that barrier. Native cleanup can keep
/// draining even if a later journal append fails; it must not be interrupted.
pub(crate) struct TerminalHistoryClosing<'a> {
    history: &'a mut TerminalHistory,
    projection: Option<TerminalScreenEngine>,
    failed: bool,
}
impl TerminalHistoryClosing<'_> {
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<TerminalCursor> {
        if self.failed {
            return Err(TerminalJournalError::Unavailable.into());
        }
        let appended = self.history.append(bytes);
        match appended {
            Ok(receipt) => {
                if let Some(screen) = &mut self.projection
                    && screen.feed(bytes).is_err()
                {
                    self.projection = None;
                }
                Ok(receipt.cursor)
            }
            Err(error) => {
                self.failed = true;
                self.projection = None;
                Err(error)
            }
        }
    }

    pub(crate) fn finish(self, complete: bool) -> Result<()> {
        if self.failed {
            return Err(TerminalJournalError::Unavailable.into());
        }
        if complete && let Some(screen) = self.projection {
            let bytes = encode_checkpoint(&screen)?;
            let saved = self
                .history
                .journal
                .publish_checkpoint(self.history.latest(), &bytes);
            self.history.mutation(saved)?;
            self.history.screen = Some(screen);
        }
        Ok(())
    }
}
impl fmt::Debug for TerminalHistory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalHistory").finish_non_exhaustive()
    }
}

impl TerminalHistory {
    /// Takes an exclusively owned, newly created empty journal. The initial
    /// dimensions are checkpointed before any process may release output.
    pub(crate) fn create(
        journal: TerminalJournal,
        dimensions: &TerminalDimensions,
    ) -> Result<Self> {
        if journal.latest() != TerminalCursor::new(1, 0).expect("constant cursor")
            || journal.checkpoint_status() != TerminalJournalCheckpointStatus::Missing
            || journal.usage().payload_bytes != 0
        {
            return Err(TerminalJournalError::Conflict.into());
        }
        let screen = TerminalScreenEngine::new(dimensions, TerminalScreenMode::Live)?;
        let mut history = Self {
            journal,
            screen: Some(screen),
            unavailable: Unavailable::Missing,
            live: true,
        };
        history.checkpoint()?;
        Ok(history)
    }

    /// Reconstructs observation-only history. No API promotes recovered
    /// history to live authority, even when every byte is available.
    pub(crate) fn recover(journal: TerminalJournal) -> Result<Self> {
        let mut history = Self {
            journal,
            screen: None,
            unavailable: Unavailable::Missing,
            live: false,
        };
        let Some(checkpoint) = history.journal.load_checkpoint()? else {
            if history.journal.checkpoint_status()
                == TerminalJournalCheckpointStatus::RetentionEvicted
            {
                history.unavailable = Unavailable::RetentionEvicted;
            }
            return Ok(history);
        };
        match decode_checkpoint(&checkpoint.bytes) {
            Ok(screen) => history.screen = Some(screen),
            Err(reason) => {
                history.unavailable = reason;
                return Ok(history);
            }
        }
        history.replay(checkpoint.source)?;
        Ok(history)
    }

    pub(crate) fn latest(&self) -> TerminalCursor {
        self.journal.latest()
    }

    pub(crate) fn session_id(&self) -> &TerminalSessionId {
        self.journal.session_id()
    }

    pub(crate) fn physical_usage(&self) -> Result<TerminalJournalPhysicalUsage> {
        Ok(self.journal.physical_usage()?)
    }

    /// Session/registry owners establish lifecycle and profile transaction
    /// authority. The history layer alone can validate usable screen coverage.
    fn retention_request(
        &self,
        kind: TerminalHistoryEviction,
    ) -> Result<Option<TerminalJournalEviction>> {
        match kind {
            TerminalHistoryEviction::CompletedOutput => {
                Ok(Some(TerminalJournalEviction::CompletedOutput))
            }
            TerminalHistoryEviction::CompletedCheckpoint => {
                Ok(Some(TerminalJournalEviction::CompletedCheckpoint))
            }
            TerminalHistoryEviction::LiveCoveredOutput => {
                let Some((identity, checkpoint)) = self.journal.load_checkpoint_with_identity()?
                else {
                    return Ok(None);
                };
                match decode_checkpoint(&checkpoint.bytes) {
                    Ok(_) => Ok(Some(TerminalJournalEviction::LiveCoveredOutput {
                        checkpoint: identity,
                    })),
                    Err(Unavailable::RawGap | Unavailable::ResizeUncheckpointed) => Ok(None),
                    Err(reason) => Err(TerminalScreenError::Unavailable(reason).into()),
                }
            }
        }
    }

    pub(crate) fn eviction_bytes(&self, kind: TerminalHistoryEviction) -> Result<usize> {
        let Some(request) = self.retention_request(kind)? else {
            return Ok(0);
        };
        Ok(self.journal.eviction_bytes(&request)?)
    }

    pub(crate) fn evict(&mut self, kind: TerminalHistoryEviction) -> Result<usize> {
        let Some(request) = self.retention_request(kind)? else {
            return Ok(0);
        };
        let result = self.journal.evict(&request);
        let bytes = self.mutation(result)?;
        if kind == TerminalHistoryEviction::CompletedCheckpoint && bytes != 0 {
            self.invalidate(Unavailable::RetentionEvicted);
        }
        Ok(bytes)
    }

    /// Facts are independently durable from evictable screen checkpoints.
    /// Recovery may acknowledge observations without acquiring live authority.
    pub(crate) fn publish_state(&mut self, bytes: &[u8]) -> Result<()> {
        let result = self.journal.publish_state(self.latest(), bytes);
        self.mutation(result)
    }

    pub(crate) fn load_state(&self) -> Result<Option<TerminalJournalCheckpoint>> {
        Ok(self.journal.load_state()?)
    }

    pub(crate) fn read(
        &self,
        cursor: &TerminalCursor,
        maximum: usize,
    ) -> Result<TerminalJournalPage> {
        Ok(self.journal.read(cursor, maximum)?)
    }

    pub(crate) fn screen(&self) -> Result<TerminalScreen> {
        Ok(self.projection()?.screen()?)
    }

    pub(crate) fn modes(&self) -> Result<TerminalModes> {
        Ok(self.projection()?.modes()?)
    }

    /// Commits raw bytes before feeding the screen, so no query reply can be
    /// returned for an uncommitted append. The runtime dispatches each returned
    /// reply once; retrying dispatch must never repeat the append.
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<TerminalHistoryAppend> {
        self.require_live()?;
        let appended = self.journal.append(bytes);
        let cursor = self.mutation(appended)?;
        let replies = if let Some(screen) = &mut self.screen {
            if let Ok(replies) = screen.feed(bytes) {
                replies
            } else {
                self.invalidate(Unavailable::Corrupt);
                Vec::new()
            }
        } else {
            Vec::new()
        };
        Ok(TerminalHistoryAppend {
            cursor,
            replies,
            screen_unavailable: self.screen.is_none().then_some(self.unavailable),
        })
    }

    /// Saves only a projection of the current committed cursor. Callers cannot
    /// pair a screen with an arbitrary earlier or later source position.
    pub(crate) fn checkpoint(&mut self) -> Result<()> {
        self.require_live()?;
        let bytes = encode_checkpoint(self.projection()?)?;
        let saved = self
            .journal
            .publish_checkpoint(self.journal.latest(), &bytes);
        self.mutation(saved)
    }

    /// Must succeed BEFORE an operation which may discard unobserved output
    /// (for example a platform signal that flushes the PTY). Future raw bytes
    /// remain readable but cannot repair the missing screen evidence.
    pub(crate) fn mark_output_gap(&mut self) -> Result<()> {
        self.require_live()?;
        self.save_unavailable(Unavailable::RawGap, RAW_GAP)
    }

    /// Publishes a discontinuity barrier before native close can discard data.
    /// Only a positively complete native drain may restore the final screen.
    /// Closing projection feeds are replay-only: quiesced input cannot emit
    /// terminal replies, even when the final output contains many queries.
    pub(crate) fn begin_close(&mut self) -> Result<TerminalHistoryClosing<'_>> {
        self.require_live()?;
        let projection = self.screen.as_ref().and_then(|screen| {
            let checkpoint = screen.checkpoint().ok()?;
            TerminalScreenEngine::restore(&checkpoint, TerminalScreenMode::Replay).ok()
        });
        self.save_unavailable(Unavailable::RawGap, RAW_GAP)?;
        Ok(TerminalHistoryClosing {
            history: self,
            projection,
            failed: false,
        })
    }

    /// Coordinates a durable invalidation barrier, the actual native resize,
    /// the in-memory resize, and the replacement checkpoint, in that order.
    /// Crashes or ambiguous failures cannot expose a pre-resize checkpoint as
    /// if it described the post-resize terminal. The closure is called once,
    /// and only after all validation and the durable barrier succeed.
    pub(crate) fn resize(
        &mut self,
        dimensions: &TerminalDimensions,
        native_resize: impl FnOnce(&TerminalDimensions) -> std::result::Result<(), ()>,
    ) -> Result<()> {
        self.require_live()?;
        dimensions
            .validate()
            .map_err(|_| TerminalScreenError::InvalidInput)?;
        self.projection()?;
        // Keep the old grid privately while publishing the unavailable marker;
        // no operation may observe it between the barrier and new checkpoint.
        let mut screen = self.screen.take().expect("validated projection");
        self.save_unavailable(Unavailable::ResizeUncheckpointed, RESIZE_PENDING)?;
        native_resize(dimensions).map_err(|()| TerminalHistoryError::NativeResize)?;
        screen.resize(dimensions)?;
        let bytes = encode_checkpoint(&screen)?;
        let saved = self
            .journal
            .publish_checkpoint(self.journal.latest(), &bytes);
        self.mutation(saved)?;
        self.screen = Some(screen);
        Ok(())
    }

    pub(crate) fn require_live(&self) -> Result<()> {
        if self.live {
            Ok(())
        } else {
            Err(TerminalHistoryError::ReadOnly)
        }
    }

    fn projection(&self) -> Result<&TerminalScreenEngine> {
        self.screen
            .as_ref()
            .ok_or_else(|| TerminalScreenError::Unavailable(self.unavailable).into())
    }

    fn invalidate(&mut self, reason: Unavailable) {
        self.screen = None;
        self.unavailable = reason;
    }

    fn save_unavailable(&mut self, reason: Unavailable, tag: u8) -> Result<()> {
        self.invalidate(reason);
        let mut bytes = MAGIC.to_vec();
        bytes.push(tag);
        let saved = self
            .journal
            .publish_checkpoint(self.journal.latest(), &bytes);
        self.mutation(saved)
    }

    fn mutation<T>(&mut self, result: std::result::Result<T, TerminalJournalError>) -> Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                // Invalid requests have no effects. Every other journal
                // failure may leave native state ambiguous: stop live effects.
                if error != TerminalJournalError::Invalid {
                    self.live = false;
                    self.invalidate(Unavailable::Corrupt);
                }
                Err(error.into())
            }
        }
    }

    fn replay(&mut self, mut cursor: TerminalCursor) -> Result<()> {
        for _ in 0..MAX_REPLAY_PAGES {
            if cursor == self.journal.latest() {
                return Ok(());
            }
            let page = self.journal.read(&cursor, MAX_TERMINAL_SCREEN_FEED_BYTES)?;
            if page.gap.is_some() || page.next <= cursor || page.bytes.is_empty() {
                self.invalidate(Unavailable::RawGap);
                return Ok(());
            }
            if self
                .screen
                .as_mut()
                .expect("replay starts with a validated checkpoint")
                .feed(&page.bytes)
                .is_err()
            {
                self.invalidate(Unavailable::Corrupt);
                return Ok(());
            }
            cursor = page.next;
        }
        if cursor != self.journal.latest() {
            self.invalidate(Unavailable::RawGap);
        }
        Ok(())
    }
}

fn encode_checkpoint(screen: &TerminalScreenEngine) -> Result<Vec<u8>> {
    let grid = screen.checkpoint()?;
    let mut bytes = Vec::with_capacity(MAGIC.len() + 1 + grid.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(GRID);
    bytes.extend_from_slice(&grid);
    Ok(bytes)
}

fn decode_checkpoint(bytes: &[u8]) -> std::result::Result<TerminalScreenEngine, Unavailable> {
    let body = bytes
        .strip_prefix(MAGIC)
        .ok_or(Unavailable::UnsupportedSchema)?;
    match body {
        [GRID, grid @ ..] => TerminalScreenEngine::restore(grid, TerminalScreenMode::Replay)
            .map_err(|_| Unavailable::Corrupt),
        [RAW_GAP] => Err(Unavailable::RawGap),
        [RESIZE_PENDING] => Err(Unavailable::ResizeUncheckpointed),
        _ => Err(Unavailable::Corrupt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_journal::TerminalJournalLimits;
    use machine_god_core::TerminalSessionId;
    use rustix::fd::OwnedFd;
    use rustix::fs::{Mode, OFlags};
    use std::fs::DirBuilder;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::PathBuf;

    struct Fixture {
        path: PathBuf,
        limits: TerminalJournalLimits,
    }
    impl Fixture {
        fn new(limits: TerminalJournalLimits) -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "machine-god-terminal-history-{:032x}",
                u128::from_le_bytes(random)
            ));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self { path, limits }
        }
        fn fd(&self) -> OwnedFd {
            rustix::fs::open(
                &self.path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap()
        }
        fn journal(&self) -> TerminalJournal {
            TerminalJournal::create(self.fd(), session(), self.limits).unwrap()
        }
        fn open(&self) -> TerminalJournal {
            // These recovery assertions run beside real process-spawn tests.
            // CLOEXEC does not prevent a transient inherited flock reference
            // between fork and exec. Retry only Busy, after dropping our owner;
            // a retained/leaked lock still fails this bounded expectation.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                match TerminalJournal::open_existing(self.fd(), &session(), self.limits) {
                    Err(TerminalJournalError::Busy) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    result => return result.unwrap(),
                }
            }
        }
        fn history(&self) -> TerminalHistory {
            TerminalHistory::create(self.journal(), &dimensions()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn session() -> TerminalSessionId {
        TerminalSessionId::new("history-test").unwrap()
    }
    fn dimensions() -> TerminalDimensions {
        TerminalDimensions::new(2, 8).unwrap()
    }
    fn origin() -> TerminalCursor {
        TerminalCursor::new(1, 0).unwrap()
    }
    fn unavailable(history: &TerminalHistory, reason: Unavailable) {
        assert_eq!(
            history.screen(),
            Err(TerminalHistoryError::Screen(
                TerminalScreenError::Unavailable(reason)
            ))
        );
        assert_eq!(
            history.modes(),
            Err(TerminalHistoryError::Screen(
                TerminalScreenError::Unavailable(reason)
            ))
        );
    }

    #[test]
    fn protected_records_do_not_displace_output_or_recovered_screen() {
        let fixture = Fixture::new(TerminalJournalLimits {
            segment_bytes: 256,
            session_bytes: 16 * 1024,
        });
        let mut history = fixture.history();
        history.append(b"checkpointed").unwrap();
        history.checkpoint().unwrap();
        history.append(b" replayed").unwrap();
        let screen = history.screen().unwrap();
        let before = history.physical_usage().unwrap();
        let state = vec![b's'; 64 * 1024];
        history.publish_state(&state).unwrap();
        for _ in 0..16 {
            history.journal.append_event(&[b'e'; 4096]).unwrap();
        }
        let after = history.physical_usage().unwrap();
        assert_eq!(after.output_bytes, before.output_bytes);
        assert_eq!(after.state_bytes, 64 * 1024);
        assert_eq!(after.event_bytes, 64 * 1024);
        assert_eq!(history.screen().unwrap(), screen);
        let page = history.read(&origin(), 64).unwrap();
        assert_eq!(page.bytes, b"checkpointed replayed");
        assert!(page.gap.is_none());
        drop(history);

        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        assert_eq!(recovered.screen().unwrap(), screen);
        assert_eq!(recovered.load_state().unwrap().unwrap().bytes, state);
        let events = recovered.journal.read_events(0, 256).unwrap();
        assert_eq!(events.events.len(), 16);
        assert_eq!(events.gap_through, 0);
        assert_eq!(recovered.physical_usage().unwrap(), after);
        assert!(recovered.read(&origin(), 64).unwrap().gap.is_none());
    }

    #[test]
    fn non_output_records_prevent_reclassification_as_new_live_history() {
        for state_only in [true, false] {
            let fixture = Fixture::new(TerminalJournalLimits::default());
            let mut journal = fixture.journal();
            if state_only {
                journal.publish_state(origin(), b"existing state").unwrap();
            } else {
                journal.append_event(b"existing event").unwrap();
            }
            assert_eq!(journal.physical_usage().unwrap().output_bytes, 0);
            drop(journal);
            assert!(matches!(
                TerminalHistory::create(fixture.open(), &dimensions()),
                Err(TerminalHistoryError::Journal(
                    TerminalJournalError::Conflict
                ))
            ));
        }
    }

    #[test]
    fn profile_retention_preserves_covered_replay_and_protected_records() {
        let fixture = Fixture::new(TerminalJournalLimits {
            segment_bytes: 1024,
            session_bytes: 64 * 1024,
        });
        let mut history = fixture.history();
        history.append(&vec![b'a'; 2048]).unwrap();
        history.checkpoint().unwrap();
        let covered = history.latest();
        history.append(b"z").unwrap();
        history.publish_state(b"protected state").unwrap();
        history.journal.append_event(b"protected event").unwrap();
        let latest = history.latest();
        let screen = history.screen().unwrap();
        let before = history.physical_usage().unwrap();
        assert_eq!(before.raw_bytes, 2049);
        assert_eq!(
            history
                .eviction_bytes(TerminalHistoryEviction::LiveCoveredOutput)
                .unwrap(),
            2048
        );
        assert_eq!(
            history
                .evict(TerminalHistoryEviction::LiveCoveredOutput)
                .unwrap(),
            2048
        );
        assert_eq!(history.latest(), latest);
        assert_eq!(history.screen().unwrap(), screen);
        let after = history.physical_usage().unwrap();
        assert_eq!(after.raw_bytes, 1);
        assert_eq!(after.state_bytes, before.state_bytes);
        assert_eq!(after.event_bytes, before.event_bytes);
        let page = history.read(&covered, 64).unwrap();
        assert!(page.gap.is_none());
        assert_eq!(page.bytes, b"z");
        drop(history);
        let mut recovered = TerminalHistory::recover(fixture.open()).unwrap();
        assert_eq!(recovered.screen().unwrap(), screen);
        assert!(
            recovered
                .evict(TerminalHistoryEviction::CompletedCheckpoint)
                .unwrap()
                > 0
        );
        unavailable(&recovered, Unavailable::RetentionEvicted);
        assert_eq!(
            recovered.load_state().unwrap().unwrap().bytes,
            b"protected state"
        );
        let after = recovered.physical_usage().unwrap();
        assert_eq!(after.checkpoint_bytes, 0);
        assert_eq!(after.state_bytes, before.state_bytes);
        assert_eq!(after.event_bytes, before.event_bytes);
        drop(recovered);
        unavailable(
            &TerminalHistory::recover(fixture.open()).unwrap(),
            Unavailable::RetentionEvicted,
        );
    }

    #[test]
    fn profile_live_retention_never_uses_gap_resize_or_malformed_checkpoints() {
        for marker in [RAW_GAP, RESIZE_PENDING, 255] {
            let fixture = Fixture::new(TerminalJournalLimits {
                segment_bytes: 1024,
                session_bytes: 64 * 1024,
            });
            let mut history = fixture.history();
            history.append(&vec![b'a'; 2048]).unwrap();
            history.checkpoint().unwrap();
            history.append(b"z").unwrap();
            let mut bytes = MAGIC.to_vec();
            bytes.push(marker);
            history
                .journal
                .publish_checkpoint(history.latest(), &bytes)
                .unwrap();
            let before = history.physical_usage().unwrap();
            if marker == 255 {
                assert!(
                    history
                        .eviction_bytes(TerminalHistoryEviction::LiveCoveredOutput)
                        .is_err()
                );
                assert!(
                    history
                        .evict(TerminalHistoryEviction::LiveCoveredOutput)
                        .is_err()
                );
            } else {
                assert_eq!(
                    history
                        .eviction_bytes(TerminalHistoryEviction::LiveCoveredOutput)
                        .unwrap(),
                    0
                );
                assert_eq!(
                    history
                        .evict(TerminalHistoryEviction::LiveCoveredOutput)
                        .unwrap(),
                    0
                );
            }
            assert_eq!(history.physical_usage().unwrap(), before);
        }
    }

    #[test]
    fn commits_before_replies_and_recovers_split_parser_state_without_effects() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let mut live = fixture.history();
        let receipt = live.append(b"\x1b[31mred\x1b[0m\xe2").unwrap();
        assert!(receipt.replies.is_empty());
        assert!(receipt.screen_unavailable.is_none());
        assert_eq!(receipt.cursor, live.latest());
        live.checkpoint().unwrap();
        let suffix = b"\x98\x83\x1b[6n";
        let reply = live.append(suffix).unwrap();
        assert_eq!(reply.replies, vec![b"\x1b[1;5R".to_vec()]);
        assert_eq!(live.read(&receipt.cursor, 64).unwrap().bytes, suffix);
        let screen = live.screen().unwrap();
        let modes = live.modes().unwrap();
        let cursor = live.latest();
        drop(live);

        let mut recovered = TerminalHistory::recover(fixture.open()).unwrap();
        assert_eq!(recovered.latest(), cursor);
        assert_eq!(recovered.screen().unwrap(), screen);
        assert_eq!(recovered.modes().unwrap(), modes);
        assert!(matches!(
            recovered.append(b"\x1b[6n"),
            Err(TerminalHistoryError::ReadOnly)
        ));
        assert_eq!(recovered.checkpoint(), Err(TerminalHistoryError::ReadOnly));
        assert_eq!(
            recovered.mark_output_gap(),
            Err(TerminalHistoryError::ReadOnly)
        );
        assert_eq!(
            recovered.resize(&dimensions(), |_| panic!("recovered native effect")),
            Err(TerminalHistoryError::ReadOnly)
        );
    }

    #[test]
    fn invalid_append_preserves_the_committed_projection() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let mut history = fixture.history();
        history.append(b"private").unwrap();
        let screen = history.screen().unwrap();
        let cursor = history.latest();
        for bytes in [Vec::new(), vec![b'x'; MAX_TERMINAL_SCREEN_FEED_BYTES + 1]] {
            assert!(matches!(
                history.append(&bytes),
                Err(TerminalHistoryError::Journal(TerminalJournalError::Invalid))
            ));
            assert_eq!(history.latest(), cursor);
            assert_eq!(history.screen().unwrap(), screen);
        }
        let receipt = history.append(b"\x1b[6n").unwrap();
        assert_eq!(receipt.replies.len(), 1);
        assert_eq!(format!("{history:?}"), "TerminalHistory { .. }");
        assert_eq!(format!("{receipt:?}"), "TerminalHistoryAppend { .. }");
    }

    #[test]
    fn live_reply_capacity_failure_preserves_raw_commit_and_suppresses_partial_replies() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let mut history = fixture.history();
        let bytes = b"\x1b[6n".repeat(17);
        let receipt = history.append(&bytes).unwrap();
        assert!(receipt.replies.is_empty());
        assert_eq!(receipt.screen_unavailable, Some(Unavailable::Corrupt));
        assert_eq!(history.read(&origin(), 1024).unwrap().bytes, bytes);
        assert_eq!(history.latest(), receipt.cursor);
        unavailable(&history, Unavailable::Corrupt);
        // Complete raw evidence can still be reconstructed in replay mode,
        // where queries require no reply budget and never produce effects.
        drop(history);
        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        assert_eq!(recovered.screen().unwrap().dimensions, dimensions());
    }

    #[test]
    fn failed_raw_publication_never_returns_replies_or_live_retry_authority() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let mut history = fixture.history();
        let before = history.screen().unwrap();
        // An unrecognized publication residue makes this transaction fail
        // after raw file writes but before its metadata commit.
        std::fs::write(fixture.path.join("tj-meta.tmp"), b"abandoned").unwrap();
        assert!(history.append(b"\x1b[6nsecret").is_err());
        unavailable(&history, Unavailable::Corrupt);
        assert!(matches!(
            history.append(b"again"),
            Err(TerminalHistoryError::ReadOnly)
        ));
        drop(history);
        // Reconciliation may remove only a private, ordinary temporary file.
        std::fs::set_permissions(
            fixture.path.join("tj-meta.tmp"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        assert_eq!(recovered.latest(), origin());
        assert_eq!(recovered.screen().unwrap(), before);
        assert!(recovered.read(&origin(), 64).unwrap().bytes.is_empty());
    }

    #[test]
    fn resize_barrier_precedes_native_effect_and_new_dimensions_survive_restart() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let mut history = fixture.history();
        history.append(b"\x1b[31mhello").unwrap();
        let source = history.latest();
        let resized = TerminalDimensions::new(4, 20).unwrap();
        let mut calls = 0;
        history
            .resize(&resized, |actual| {
                calls += 1;
                assert_eq!(actual, &resized);
                // Inspect the committed manifest in the native effect seam.
                let metadata: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(fixture.path.join("tj-meta")).unwrap())
                        .unwrap();
                let id = metadata["manifest"]["checkpoint"]["blob"]["id"]
                    .as_u64()
                    .unwrap();
                let marker =
                    std::fs::read(fixture.path.join(format!("tj-checkpoint-{id:020}"))).unwrap();
                assert!(matches!(
                    decode_checkpoint(&marker),
                    Err(Unavailable::ResizeUncheckpointed)
                ));
                Ok(())
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(history.latest(), source);
        assert_eq!(history.screen().unwrap().dimensions, resized);
        let expected = history.screen().unwrap();
        drop(history);
        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        assert_eq!(recovered.screen().unwrap(), expected);
    }

    #[test]
    fn failed_or_interrupted_resize_never_recovers_the_old_screen() {
        for interrupted in [false, true] {
            let fixture = Fixture::new(TerminalJournalLimits::default());
            let mut history = fixture.history();
            history.append(b"before").unwrap();
            if interrupted {
                let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = history.resize(&dimensions(), |_| panic!("simulated interruption"));
                }));
                assert!(unwind.is_err());
            } else {
                assert_eq!(
                    history.resize(&dimensions(), |_| Err(())),
                    Err(TerminalHistoryError::NativeResize)
                );
            }
            unavailable(&history, Unavailable::ResizeUncheckpointed);
            let receipt = history.append(b"after\x1b[6n").unwrap();
            assert!(receipt.replies.is_empty());
            assert_eq!(
                receipt.screen_unavailable,
                Some(Unavailable::ResizeUncheckpointed)
            );
            drop(history);
            let recovered = TerminalHistory::recover(fixture.open()).unwrap();
            unavailable(&recovered, Unavailable::ResizeUncheckpointed);
            assert_eq!(
                recovered.read(&origin(), 64).unwrap().bytes,
                b"beforeafter\x1b[6n"
            );
        }
    }

    #[test]
    fn failed_barrier_never_calls_native_resize() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let mut history = fixture.history();
        std::fs::write(fixture.path.join("tj-meta.tmp"), b"abandoned").unwrap();
        assert!(
            history
                .resize(&dimensions(), |_| panic!("effect before durable barrier"))
                .is_err()
        );
        unavailable(&history, Unavailable::Corrupt);
    }

    #[test]
    fn failed_post_resize_checkpoint_leaves_durable_unavailability() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let mut history = fixture.history();
        assert!(
            history
                .resize(&dimensions(), |_| {
                    use std::os::unix::fs::OpenOptionsExt;
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(fixture.path.join("tj-meta.tmp"))
                        .unwrap();
                    Ok(())
                })
                .is_err()
        );
        drop(history);
        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        unavailable(&recovered, Unavailable::ResizeUncheckpointed);
    }

    #[test]
    fn output_gap_remains_durable_and_cannot_be_repaired_by_later_raw_bytes() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let mut history = fixture.history();
        history.append(b"before").unwrap();
        history.mark_output_gap().unwrap();
        let receipt = history.append(b"after\x1b[6n").unwrap();
        assert!(receipt.replies.is_empty());
        assert_eq!(receipt.screen_unavailable, Some(Unavailable::RawGap));
        assert!(history.checkpoint().is_err());
        drop(history);
        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        unavailable(&recovered, Unavailable::RawGap);
        assert_eq!(
            recovered.read(&origin(), 64).unwrap().bytes,
            b"beforeafter\x1b[6n"
        );
    }

    #[test]
    fn raw_retention_gap_invalidates_replay_but_not_the_live_screen() {
        let fixture = Fixture::new(TerminalJournalLimits {
            segment_bytes: 256,
            session_bytes: 16 * 1024,
        });
        let mut history = fixture.history();
        for _ in 0..32 {
            history.append(&[b'x'; 1024]).unwrap();
        }
        assert!(history.screen().is_ok());
        assert!(history.read(&origin(), 64).unwrap().gap.is_some());
        drop(history);
        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        unavailable(&recovered, Unavailable::RawGap);
    }

    #[test]
    fn checkpoint_at_latest_recovers_even_after_older_raw_was_evicted() {
        let fixture = Fixture::new(TerminalJournalLimits {
            segment_bytes: 256,
            session_bytes: 16 * 1024,
        });
        let mut history = fixture.history();
        for _ in 0..32 {
            history.append(&[b'x'; 1024]).unwrap();
        }
        history.checkpoint().unwrap();
        let screen = history.screen().unwrap();
        drop(history);
        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        assert_eq!(recovered.screen().unwrap(), screen);
        assert!(recovered.read(&origin(), 64).unwrap().gap.is_some());
    }

    #[test]
    fn evicted_checkpoint_is_distinct_from_missing_or_raw_gap_evidence() {
        let fixture = Fixture::new(TerminalJournalLimits {
            segment_bytes: 16 * 1024,
            session_bytes: 16 * 1024,
        });
        let mut history = fixture.history();
        history.append(&[b'x'; 16 * 1024]).unwrap();
        assert!(history.screen().is_ok());
        drop(history);
        let recovered = TerminalHistory::recover(fixture.open()).unwrap();
        unavailable(&recovered, Unavailable::RetentionEvicted);
        assert!(recovered.read(&origin(), 64).unwrap().gap.is_none());
    }

    #[test]
    fn unknown_corrupt_and_missing_checkpoints_preserve_raw_without_guessing_dimensions() {
        for (bytes, expected) in [
            (b"future schema".to_vec(), Unavailable::UnsupportedSchema),
            (
                [MAGIC.as_slice(), &[GRID], b"corrupt"].concat(),
                Unavailable::Corrupt,
            ),
            (
                [MAGIC.as_slice(), &[RAW_GAP, 99]].concat(),
                Unavailable::Corrupt,
            ),
            (Vec::new(), Unavailable::Missing),
        ] {
            let fixture = Fixture::new(TerminalJournalLimits::default());
            let mut journal = fixture.journal();
            journal.append(b"raw").unwrap();
            if !bytes.is_empty() {
                journal
                    .publish_checkpoint(journal.latest(), &bytes)
                    .unwrap();
            }
            let history = TerminalHistory::recover(journal).unwrap();
            unavailable(&history, expected);
            assert_eq!(history.read(&origin(), 64).unwrap().bytes, b"raw");
        }
    }

    #[test]
    fn existing_history_cannot_be_reclassified_as_new_live_output() {
        let fixture = Fixture::new(TerminalJournalLimits::default());
        let history = fixture.history();
        drop(history);
        assert!(matches!(
            TerminalHistory::create(fixture.open(), &dimensions()),
            Err(TerminalHistoryError::Journal(
                TerminalJournalError::Conflict
            ))
        ));
    }
}
