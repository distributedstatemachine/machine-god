//! Process-local bounded output retention for managed background commands.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use machine_god_core::BackgroundOutputOwner;

/// Maximum prefix retained for one background command.
pub(crate) const MAX_BACKGROUND_OUTPUT_RETAINED_BYTES: usize = 64 * 1024;
/// Maximum bytes returned by one output read.
///
/// Seven KiB leaves room beneath terminal's 48 KiB serialized-result ceiling
/// even when every retained byte requires a six-byte JSON control escape.
pub(crate) const MAX_BACKGROUND_OUTPUT_READ_BYTES: usize = 7 * 1024;
/// Maximum concurrently registered, non-closed output streams.
pub(crate) const MAX_BACKGROUND_OUTPUT_LIVE_STREAMS: usize = 16;
/// Maximum closed streams retained for later reads.
pub(crate) const MAX_BACKGROUND_OUTPUT_CLOSED_STREAMS: usize = 100;
/// The sole cursor segment in the prefix-retention format.
pub(crate) const BACKGROUND_OUTPUT_SEGMENT: u64 = 1;

/// Stable category for a process-local background-output failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackgroundOutputErrorKind {
    InvalidRequest,
    Capacity,
    Conflict,
    NotFound,
    Closed,
}

/// Fixed, data-free process-local background-output failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct BackgroundOutputError {
    kind: BackgroundOutputErrorKind,
}

impl BackgroundOutputError {
    /// Returns the stable failure category.
    pub(crate) const fn kind(self) -> BackgroundOutputErrorKind {
        self.kind
    }

    const fn new(kind: BackgroundOutputErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Debug for BackgroundOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundOutputError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for BackgroundOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("background output operation failed")
    }
}

impl std::error::Error for BackgroundOutputError {}

/// One immutable bounded page from a registered output stream.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct BackgroundOutputSnapshot {
    bytes: Vec<u8>,
    next_offset: u64,
    produced_bytes: u64,
    retained_bytes: u64,
    pending_utf8_bytes: u8,
    capture_incomplete: bool,
    closed: bool,
}

impl BackgroundOutputSnapshot {
    /// Returns this page's merged output bytes.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the byte offset to use for the next read in segment one.
    pub(crate) const fn next_offset(&self) -> u64 {
        self.next_offset
    }

    /// Returns the saturating count of all bytes appended to the stream.
    pub(crate) const fn produced_bytes(&self) -> u64 {
        self.produced_bytes
    }

    /// Returns the number of prefix bytes still available for reads.
    pub(crate) const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    /// Returns a live trailing byte count withheld until its UTF-8 scalar is complete.
    pub(crate) const fn pending_utf8_bytes(&self) -> u8 {
        self.pending_utf8_bytes
    }

    /// Reports whether produced bytes extend beyond the retained prefix.
    pub(crate) const fn truncated(&self) -> bool {
        self.capture_incomplete || self.produced_bytes > self.retained_bytes
    }

    /// Reports whether the producer has closed this stream.
    pub(crate) const fn closed(&self) -> bool {
        self.closed
    }
}

impl fmt::Debug for BackgroundOutputSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundOutputSnapshot")
            .field("page_bytes", &self.bytes.len())
            .field("next_offset", &self.next_offset)
            .field("produced_bytes", &self.produced_bytes)
            .field("retained_bytes", &self.retained_bytes)
            .field("pending_utf8_bytes", &self.pending_utf8_bytes)
            .field("capture_incomplete", &self.capture_incomplete)
            .field("closed", &self.closed)
            .finish()
    }
}

/// Cloneable process-local registry for bounded managed output.
#[derive(Clone, Default)]
pub(crate) struct BackgroundOutputRegistry {
    state: Arc<Mutex<RegistryState>>,
}

impl BackgroundOutputRegistry {
    /// Constructs one empty registry.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Reserves a hidden stream before process release.
    ///
    /// The stream remains unreadable until [`Self::activate`] succeeds.
    pub(crate) fn register(
        &self,
        id: u64,
        owner: BackgroundOutputOwner,
    ) -> Result<(), BackgroundOutputError> {
        if id == 0 {
            return Err(error(BackgroundOutputErrorKind::InvalidRequest));
        }
        let mut state = self.lock();
        if state.entries.contains_key(&id) {
            return Err(error(BackgroundOutputErrorKind::Conflict));
        }
        if state.live_streams == MAX_BACKGROUND_OUTPUT_LIVE_STREAMS {
            return Err(error(BackgroundOutputErrorKind::Capacity));
        }
        state.entries.insert(
            id,
            OutputEntry {
                owner,
                bytes: Vec::new(),
                produced_bytes: 0,
                capture_incomplete: false,
                active: false,
                closed: false,
            },
        );
        state.live_streams += 1;
        Ok(())
    }

    /// Makes a successfully released stream visible to its owner.
    pub(crate) fn activate(&self, id: u64) -> Result<(), BackgroundOutputError> {
        let mut state = self.lock();
        let entry = state
            .entries
            .get_mut(&id)
            .ok_or_else(|| error(BackgroundOutputErrorKind::NotFound))?;
        if entry.active {
            return Err(error(BackgroundOutputErrorKind::Conflict));
        }
        entry.active = true;
        Ok(())
    }

    /// Appends merged process output while retaining only its bounded prefix.
    pub(crate) fn append(&self, id: u64, bytes: &[u8]) -> Result<(), BackgroundOutputError> {
        let mut state = self.lock();
        let entry = state
            .entries
            .get_mut(&id)
            .ok_or_else(|| error(BackgroundOutputErrorKind::NotFound))?;
        if entry.closed {
            return Err(error(BackgroundOutputErrorKind::Closed));
        }
        entry.produced_bytes = saturating_add_length(entry.produced_bytes, bytes.len());
        let available = MAX_BACKGROUND_OUTPUT_RETAINED_BYTES.saturating_sub(entry.bytes.len());
        let accepted = available.min(bytes.len());
        entry.bytes.extend_from_slice(&bytes[..accepted]);
        Ok(())
    }

    /// Marks a stream closed and evicts the oldest closed stream when needed.
    pub(crate) fn close(&self, id: u64) -> Result<(), BackgroundOutputError> {
        self.close_with_incomplete(id, false)
    }

    /// Marks a stream closed after bounded draining discarded an unread suffix.
    pub(crate) fn close_incomplete(&self, id: u64) -> Result<(), BackgroundOutputError> {
        self.close_with_incomplete(id, true)
    }

    fn close_with_incomplete(
        &self,
        id: u64,
        capture_incomplete: bool,
    ) -> Result<(), BackgroundOutputError> {
        let mut state = self.lock();
        let entry = state
            .entries
            .get_mut(&id)
            .ok_or_else(|| error(BackgroundOutputErrorKind::NotFound))?;
        if entry.closed {
            return Err(error(BackgroundOutputErrorKind::Conflict));
        }
        entry.capture_incomplete = capture_incomplete;
        entry.closed = true;
        state.live_streams -= 1;
        state.closed_order.push_back(id);
        while state.closed_order.len() > MAX_BACKGROUND_OUTPUT_CLOSED_STREAMS {
            let oldest = state
                .closed_order
                .pop_front()
                .expect("closed stream count exceeded its nonzero limit");
            state.entries.remove(&oldest);
        }
        Ok(())
    }

    /// Removes one hidden pre-release stream.
    pub(crate) fn remove(&self, id: u64) -> Result<(), BackgroundOutputError> {
        let mut state = self.lock();
        let entry = state
            .entries
            .get(&id)
            .ok_or_else(|| error(BackgroundOutputErrorKind::NotFound))?;
        if entry.active || entry.closed {
            return Err(error(BackgroundOutputErrorKind::Conflict));
        }
        state.entries.remove(&id);
        state.live_streams -= 1;
        Ok(())
    }

    /// Reads one fixed-size page after exact owner and cursor validation.
    pub(crate) fn read(
        &self,
        id: u64,
        owner: &BackgroundOutputOwner,
        segment: u64,
        offset: u64,
    ) -> Result<BackgroundOutputSnapshot, BackgroundOutputError> {
        let state = self.lock();
        let entry = state
            .entries
            .get(&id)
            .filter(|entry| entry.active && &entry.owner == owner)
            .ok_or_else(|| error(BackgroundOutputErrorKind::NotFound))?;
        if segment != BACKGROUND_OUTPUT_SEGMENT || offset > entry.produced_bytes {
            return Err(error(BackgroundOutputErrorKind::InvalidRequest));
        }

        let retained_bytes = entry.bytes.len() as u64;
        let (bytes, next_offset, pending_utf8_bytes) = if offset >= retained_bytes {
            (Vec::new(), entry.produced_bytes, 0)
        } else {
            let start = usize::try_from(offset)
                .expect("an offset below the retained byte bound always fits usize");
            let candidate_end = start
                .saturating_add(MAX_BACKGROUND_OUTPUT_READ_BYTES)
                .min(entry.bytes.len());
            let preserve_incomplete_suffix = candidate_end < entry.bytes.len()
                || (!entry.closed && entry.produced_bytes == retained_bytes);
            let pending = if preserve_incomplete_suffix {
                incomplete_utf8_suffix_len(&entry.bytes[start..candidate_end])
            } else {
                0
            };
            let end = candidate_end - pending;
            let pending_utf8_bytes = if end == start {
                u8::try_from(pending).expect("a UTF-8 scalar has at most three pending bytes")
            } else {
                0
            };
            (
                entry.bytes[start..end].to_vec(),
                end as u64,
                pending_utf8_bytes,
            )
        };
        Ok(BackgroundOutputSnapshot {
            bytes,
            next_offset,
            produced_bytes: entry.produced_bytes,
            retained_bytes,
            pending_utf8_bytes,
            capture_incomplete: entry.capture_incomplete,
            closed: entry.closed,
        })
    }

    fn lock(&self) -> MutexGuard<'_, RegistryState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl fmt::Debug for BackgroundOutputRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        formatter
            .debug_struct("BackgroundOutputRegistry")
            .field("registered_streams", &state.entries.len())
            .field("live_streams", &state.live_streams)
            .field("closed_streams", &state.closed_order.len())
            .finish()
    }
}

#[derive(Default)]
struct RegistryState {
    entries: BTreeMap<u64, OutputEntry>,
    closed_order: VecDeque<u64>,
    live_streams: usize,
}

struct OutputEntry {
    owner: BackgroundOutputOwner,
    bytes: Vec<u8>,
    produced_bytes: u64,
    capture_incomplete: bool,
    active: bool,
    closed: bool,
}

const fn error(kind: BackgroundOutputErrorKind) -> BackgroundOutputError {
    BackgroundOutputError::new(kind)
}

fn saturating_add_length(produced_bytes: u64, appended_bytes: usize) -> u64 {
    produced_bytes.saturating_add(u64::try_from(appended_bytes).unwrap_or(u64::MAX))
}

pub(crate) fn incomplete_utf8_suffix_len(bytes: &[u8]) -> usize {
    let continuation_bytes = bytes
        .iter()
        .rev()
        .take(3)
        .take_while(|byte| matches!(byte, 0x80..=0xbf))
        .count();
    let Some(lead_index) = bytes.len().checked_sub(continuation_bytes + 1) else {
        return 0;
    };
    let lead = bytes[lead_index];
    let expected_length = match lead {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return 0,
    };
    let available_length = continuation_bytes + 1;
    if available_length >= expected_length
        || !partial_utf8_scalar_is_valid(&bytes[lead_index..], expected_length)
    {
        0
    } else {
        available_length
    }
}

fn partial_utf8_scalar_is_valid(bytes: &[u8], expected_length: usize) -> bool {
    if bytes.len() >= 2 {
        let second = bytes[1];
        let valid_second = match bytes[0] {
            0xe0 => matches!(second, 0xa0..=0xbf),
            0xed => matches!(second, 0x80..=0x9f),
            0xf0 => matches!(second, 0x90..=0xbf),
            0xf4 => matches!(second, 0x80..=0x8f),
            _ => matches!(second, 0x80..=0xbf),
        };
        if !valid_second {
            return false;
        }
    }
    bytes
        .iter()
        .skip(2)
        .take(expected_length.saturating_sub(2))
        .all(|byte| matches!(byte, 0x80..=0xbf))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use machine_god_core::{BackgroundOutputOwner, SessionId, SessionIncarnationId};

    use super::{
        BACKGROUND_OUTPUT_SEGMENT, BackgroundOutputErrorKind, BackgroundOutputRegistry,
        MAX_BACKGROUND_OUTPUT_CLOSED_STREAMS, MAX_BACKGROUND_OUTPUT_LIVE_STREAMS,
        MAX_BACKGROUND_OUTPUT_READ_BYTES, MAX_BACKGROUND_OUTPUT_RETAINED_BYTES,
        saturating_add_length,
    };

    fn owner(name: &str) -> BackgroundOutputOwner {
        BackgroundOutputOwner::new(
            SessionId::new(format!("session-{name}")).unwrap(),
            SessionIncarnationId::new(format!("incarnation-{name}")).unwrap(),
        )
    }

    fn visible(registry: &BackgroundOutputRegistry, id: u64, owner: BackgroundOutputOwner) {
        registry.register(id, owner).unwrap();
        registry.activate(id).unwrap();
    }

    #[test]
    fn registration_is_hidden_bounded_and_removable_before_release() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("capacity");
        assert_eq!(
            registry.register(0, owner.clone()).unwrap_err().kind(),
            BackgroundOutputErrorKind::InvalidRequest
        );
        for id in 1..=MAX_BACKGROUND_OUTPUT_LIVE_STREAMS as u64 {
            registry.register(id, owner.clone()).unwrap();
            assert_eq!(
                registry
                    .read(id, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
                    .unwrap_err()
                    .kind(),
                BackgroundOutputErrorKind::NotFound
            );
        }
        assert_eq!(
            registry
                .register(MAX_BACKGROUND_OUTPUT_LIVE_STREAMS as u64 + 1, owner.clone())
                .unwrap_err()
                .kind(),
            BackgroundOutputErrorKind::Capacity
        );
        registry.remove(1).unwrap();
        registry
            .register(MAX_BACKGROUND_OUTPUT_LIVE_STREAMS as u64 + 1, owner)
            .unwrap();
        assert_eq!(
            registry.remove(1).unwrap_err().kind(),
            BackgroundOutputErrorKind::NotFound
        );
    }

    #[test]
    fn retained_prefix_produced_count_and_page_bound_are_exact() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("limits");
        visible(&registry, 1, owner.clone());
        let produced = vec![b'x'; MAX_BACKGROUND_OUTPUT_RETAINED_BYTES + 1];
        registry.append(1, &produced).unwrap();

        let first = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        assert_eq!(first.bytes().len(), MAX_BACKGROUND_OUTPUT_READ_BYTES);
        assert_eq!(first.next_offset(), MAX_BACKGROUND_OUTPUT_READ_BYTES as u64);
        assert_eq!(
            first.produced_bytes(),
            (MAX_BACKGROUND_OUTPUT_RETAINED_BYTES + 1) as u64
        );
        assert_eq!(
            first.retained_bytes(),
            MAX_BACKGROUND_OUTPUT_RETAINED_BYTES as u64
        );
        assert!(first.truncated());
        assert!(!first.closed());

        let last = registry
            .read(
                1,
                &owner,
                BACKGROUND_OUTPUT_SEGMENT,
                MAX_BACKGROUND_OUTPUT_RETAINED_BYTES as u64,
            )
            .unwrap();
        assert!(last.bytes().is_empty());
        assert_eq!(last.next_offset(), first.produced_bytes());
        assert_eq!(
            registry
                .read(
                    1,
                    &owner,
                    BACKGROUND_OUTPUT_SEGMENT,
                    first.produced_bytes() + 1
                )
                .unwrap_err()
                .kind(),
            BackgroundOutputErrorKind::InvalidRequest
        );
    }

    #[test]
    fn pages_preserve_a_valid_utf8_scalar_across_the_raw_page_boundary() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("utf8-page");
        visible(&registry, 1, owner.clone());
        let mut output = vec![b'a'; MAX_BACKGROUND_OUTPUT_READ_BYTES - 1];
        output.extend_from_slice("é".as_bytes());
        registry.append(1, &output).unwrap();
        registry.close(1).unwrap();

        let first = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        assert_eq!(
            first.bytes(),
            &output[..MAX_BACKGROUND_OUTPUT_READ_BYTES - 1]
        );
        assert_eq!(
            first.next_offset(),
            (MAX_BACKGROUND_OUTPUT_READ_BYTES - 1) as u64
        );
        assert_eq!(first.pending_utf8_bytes(), 0);

        let second = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, first.next_offset())
            .unwrap();
        assert_eq!(second.bytes(), "é".as_bytes());
        assert_eq!(second.next_offset(), output.len() as u64);
        assert_eq!(second.pending_utf8_bytes(), 0);
    }

    #[test]
    fn live_incomplete_utf8_scalar_waits_without_loss_then_advances() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("utf8-live");
        visible(&registry, 1, owner.clone());
        registry.append(1, &[0xc3]).unwrap();

        let pending = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        assert!(pending.bytes().is_empty());
        assert_eq!(pending.next_offset(), 0);
        assert_eq!(pending.retained_bytes(), 1);
        assert_eq!(pending.pending_utf8_bytes(), 1);
        assert!(!pending.closed());

        registry.append(1, &[0xa9]).unwrap();
        let complete = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        assert_eq!(complete.bytes(), "é".as_bytes());
        assert_eq!(complete.next_offset(), 2);
        assert_eq!(complete.pending_utf8_bytes(), 0);
    }

    #[test]
    fn closed_incomplete_utf8_suffix_is_returned_for_lossy_decoding() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("utf8-closed-incomplete");
        visible(&registry, 1, owner.clone());
        registry.append(1, &[0xc3]).unwrap();
        registry.close(1).unwrap();

        let snapshot = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        assert_eq!(snapshot.bytes(), &[0xc3]);
        assert_eq!(snapshot.next_offset(), 1);
        assert_eq!(snapshot.pending_utf8_bytes(), 0);
        assert!(snapshot.closed());
    }

    #[test]
    fn produced_count_saturates_at_u64_max() {
        assert_eq!(saturating_add_length(u64::MAX - 1, 1), u64::MAX);
        assert_eq!(saturating_add_length(u64::MAX - 1, 2), u64::MAX);
    }

    #[test]
    fn bytes_appended_before_activation_remain_hidden_then_become_visible() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("activation");
        registry.register(1, owner.clone()).unwrap();
        registry.append(1, b"early output").unwrap();
        assert_eq!(
            registry
                .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
                .unwrap_err()
                .kind(),
            BackgroundOutputErrorKind::NotFound
        );
        registry.activate(1).unwrap();
        assert_eq!(
            registry
                .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
                .unwrap()
                .bytes(),
            b"early output"
        );
    }

    #[test]
    fn cursor_segment_owner_and_presence_fail_closed() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("right");
        let wrong = self::owner("WRONG_PRIVATE_OWNER");
        visible(&registry, 7, owner.clone());
        registry.append(7, b"output").unwrap();
        for (id, supplied_owner) in [(8, &owner), (7, &wrong)] {
            assert_eq!(
                registry
                    .read(id, supplied_owner, BACKGROUND_OUTPUT_SEGMENT, 0)
                    .unwrap_err()
                    .kind(),
                BackgroundOutputErrorKind::NotFound
            );
        }
        for segment in [0, 2] {
            assert_eq!(
                registry.read(7, &owner, segment, 0).unwrap_err().kind(),
                BackgroundOutputErrorKind::InvalidRequest
            );
        }
    }

    #[test]
    fn closing_evicts_only_the_oldest_closed_stream() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("eviction");
        for id in 1..=MAX_BACKGROUND_OUTPUT_CLOSED_STREAMS as u64 {
            visible(&registry, id, owner.clone());
            registry.close(id).unwrap();
        }
        visible(&registry, 10_000, owner.clone());
        visible(&registry, 10_001, owner.clone());
        registry.close(10_001).unwrap();

        assert_eq!(
            registry
                .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
                .unwrap_err()
                .kind(),
            BackgroundOutputErrorKind::NotFound
        );
        assert!(
            registry
                .read(2, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
                .unwrap()
                .closed()
        );
        assert!(
            !registry
                .read(10_000, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
                .unwrap()
                .closed()
        );
    }

    #[test]
    fn closed_streams_reject_append_but_remain_readable() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("closed");
        visible(&registry, 1, owner.clone());
        registry.append(1, b"complete").unwrap();
        registry.close(1).unwrap();
        assert_eq!(
            registry.append(1, b"late").unwrap_err().kind(),
            BackgroundOutputErrorKind::Closed
        );
        let snapshot = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        assert_eq!(snapshot.bytes(), b"complete");
        assert!(snapshot.closed());
    }

    #[test]
    fn incomplete_close_reports_truncation_without_inventing_observed_bytes() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("incomplete-close");
        visible(&registry, 1, owner.clone());
        registry.append(1, b"observed").unwrap();
        registry.close_incomplete(1).unwrap();

        let snapshot = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        assert_eq!(snapshot.bytes(), b"observed");
        assert_eq!(snapshot.produced_bytes(), 8);
        assert_eq!(snapshot.retained_bytes(), 8);
        assert!(snapshot.truncated());
        assert!(snapshot.closed());
    }

    #[test]
    fn concurrent_append_and_read_preserve_one_bounded_prefix() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("concurrent");
        visible(&registry, 1, owner.clone());
        let registry = Arc::new(registry);
        let barrier = Arc::new(Barrier::new(3));
        let writer_registry = Arc::clone(&registry);
        let writer_barrier = Arc::clone(&barrier);
        let writer = thread::spawn(move || {
            writer_barrier.wait();
            for _ in 0..1_000 {
                writer_registry.append(1, b"abcd").unwrap();
            }
        });
        let reader_registry = Arc::clone(&registry);
        let reader_barrier = Arc::clone(&barrier);
        let reader_owner = owner.clone();
        let reader = thread::spawn(move || {
            reader_barrier.wait();
            for _ in 0..1_000 {
                let snapshot = reader_registry
                    .read(1, &reader_owner, BACKGROUND_OUTPUT_SEGMENT, 0)
                    .unwrap();
                assert!(snapshot.bytes().len() <= MAX_BACKGROUND_OUTPUT_READ_BYTES);
                assert!(snapshot.retained_bytes() <= snapshot.produced_bytes());
            }
        });
        barrier.wait();
        writer.join().unwrap();
        reader.join().unwrap();

        registry.close(1).unwrap();
        let snapshot = registry
            .read(1, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        assert_eq!(snapshot.produced_bytes(), 4_000);
        assert_eq!(snapshot.retained_bytes(), 4_000);
        assert!(snapshot.closed());
    }

    #[test]
    fn debug_and_errors_do_not_disclose_owner_or_output() {
        let registry = BackgroundOutputRegistry::new();
        let owner = owner("PRIVATE_OWNER_SENTINEL");
        visible(&registry, 77, owner.clone());
        registry.append(77, b"PRIVATE_OUTPUT_SENTINEL").unwrap();
        let snapshot = registry
            .read(77, &owner, BACKGROUND_OUTPUT_SEGMENT, 0)
            .unwrap();
        for debug in [format!("{registry:?}"), format!("{snapshot:?}")] {
            assert!(!debug.contains("PRIVATE_OWNER_SENTINEL"));
            assert!(!debug.contains("PRIVATE_OUTPUT_SENTINEL"));
        }
        let error = registry.append(99, b"private").unwrap_err();
        assert_eq!(
            format!("{error:?}"),
            "BackgroundOutputError { kind: NotFound }"
        );
        assert_eq!(error.to_string(), "background output operation failed");
    }
}
