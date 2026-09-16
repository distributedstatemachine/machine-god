//! One canonical history snapshot and bounded, source-checked reading positions.
mod detail;
use crate::{NativeManagedHistorySnapshot, NativeObservedManagedAgent};
use machine_god_core::{Role, SessionRecord};
use sha2::{Digest, Sha256};
use std::{fmt, io};

/// Per-child presentation only; these modes never modify the saved transcript.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NativeManagedHistoryMode {
    #[default]
    Conversation,
    Transcript,
    Full,
}
impl NativeManagedHistoryMode {
    const fn index(self) -> usize {
        match self {
            Self::Conversation => 0,
            Self::Transcript => 1,
            Self::Full => 2,
        }
    }
    #[must_use]
    pub const fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// Presentation coordinates only. The owner validates them against its exact
/// displayed canonical record; they grant no process or command authority.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct NativeManagedHistoryPosition {
    pub message: usize,
    /// None is the role heading; Some selects a canonical content block.
    pub block: Option<usize>,
    /// UTF-8 offset in text, or in the full mode's compact serialized block.
    pub byte: usize,
}
impl fmt::Debug for NativeManagedHistoryPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedHistoryPosition { .. }")
    }
}

#[derive(Clone, Copy)]
pub struct NativeManagedHistoryView<'a> {
    pub record: &'a SessionRecord,
    pub mode: NativeManagedHistoryMode,
    /// None follows the tail. An anchor remains independent of editor/frame ACK.
    pub position: Option<NativeManagedHistoryPosition>,
}
impl NativeManagedHistoryView<'_> {
    /// Returns complete text for a visible block, or None for collapsed detail.
    /// Structured full detail uses at most one record-sized temporary buffer.
    /// # Errors
    /// Rejects absent/system blocks or detail exceeding the canonical record bound.
    pub fn block_text(
        &self,
        message: usize,
        block: usize,
    ) -> Result<Option<std::borrow::Cow<'_, str>>, super::NativeManagedNavigationError> {
        let message = self
            .record
            .messages
            .get(message)
            .filter(|message| message.role != Role::System)
            .ok_or(super::NativeManagedNavigationError::InvalidAction)?;
        detail::text(
            message
                .content
                .get(block)
                .ok_or(super::NativeManagedNavigationError::InvalidAction)?,
            message.role,
            self.mode,
        )
        .map_err(|()| super::NativeManagedNavigationError::Unavailable)
    }
}
impl fmt::Debug for NativeManagedHistoryView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedHistoryView { .. }")
    }
}

struct Reading {
    owner: NativeObservedManagedAgent,
    mode: NativeManagedHistoryMode,
    anchors: [Option<Anchor>; 3],
}
#[derive(Clone, Copy)]
struct Anchor {
    position: NativeManagedHistoryPosition,
    message_digest: [u8; 32],
}
impl Anchor {
    fn matches(self, record: &SessionRecord, mode: NativeManagedHistoryMode) -> bool {
        valid(record, self.position, mode)
            && digest(record, self.position) == Some(self.message_digest)
    }
}
#[derive(Default)]
pub(super) struct History {
    snapshot: Option<NativeManagedHistorySnapshot>,
    readings: Vec<Reading>,
    position: Option<NativeManagedHistoryPosition>,
    mode: NativeManagedHistoryMode,
}
impl History {
    pub(super) fn clear(&mut self) {
        self.snapshot = None;
        self.position = None;
        self.mode = NativeManagedHistoryMode::default();
    }
    pub(super) fn install(&mut self, snapshot: NativeManagedHistorySnapshot) {
        let reading = self
            .readings
            .iter()
            .find(|reading| reading.owner.same_conversation(snapshot.observation()));
        self.mode = reading.map_or(NativeManagedHistoryMode::default(), |reading| reading.mode);
        self.position = reading
            .and_then(|reading| reading.anchors[self.mode.index()])
            .filter(|anchor| anchor.matches(snapshot.record(), self.mode))
            .map(|anchor| anchor.position);
        self.snapshot = Some(snapshot);
    }
    pub(super) fn view(&self) -> Option<NativeManagedHistoryView<'_>> {
        Some(NativeManagedHistoryView {
            record: self.snapshot.as_ref()?.record(),
            mode: self.mode,
            position: self.position,
        })
    }
    pub(super) fn seek(
        &mut self,
        position: Option<NativeManagedHistoryPosition>,
    ) -> Result<(), super::NativeManagedNavigationError> {
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or(super::NativeManagedNavigationError::NoSelection)?;
        let reading = position
            .map(|position| {
                if !valid(snapshot.record(), position, self.mode) {
                    return Err(super::NativeManagedNavigationError::InvalidAction);
                }
                Ok(Anchor {
                    position,
                    // The installed position was checked against this immutable
                    // snapshot. Scrolling within its message need not hash a
                    // potentially multi-megabyte text again on every keypress.
                    message_digest: self
                        .position
                        .filter(|current| current.message == position.message)
                        .and_then(|_| {
                            self.readings
                                .iter()
                                .find(|reading| {
                                    reading.owner.same_conversation(snapshot.observation())
                                })
                                .and_then(|reading| reading.anchors[self.mode.index()])
                        })
                        .map(|reading| reading.message_digest)
                        .or_else(|| digest(snapshot.record(), position))
                        .ok_or(super::NativeManagedNavigationError::InvalidAction)?,
                })
            })
            .transpose()?;
        let mode = self.mode;
        self.remember()?.anchors[mode.index()] = reading;
        self.position = position;
        Ok(())
    }
    fn remember(&mut self) -> Result<&mut Reading, super::NativeManagedNavigationError> {
        let owner = self
            .snapshot
            .as_ref()
            .ok_or(super::NativeManagedNavigationError::NoSelection)?
            .observation();
        let mut reading = self
            .readings
            .iter()
            .position(|reading| reading.owner.same_conversation(owner))
            .map_or_else(
                || Reading {
                    owner: owner.clone(),
                    mode: self.mode,
                    anchors: [None; 3],
                },
                |index| self.readings.remove(index),
            );
        reading.mode = self.mode;
        if self.readings.len() == 128 {
            self.readings.remove(0);
        }
        self.readings.push(reading);
        Ok(self.readings.last_mut().expect("inserted reading"))
    }
    pub(super) fn set_mode(
        &mut self,
        mode: NativeManagedHistoryMode,
    ) -> Result<(), super::NativeManagedNavigationError> {
        self.remember()?;
        let snapshot = self.snapshot.as_ref().expect("remember checked snapshot");
        self.position = self
            .readings
            .last()
            .and_then(|reading| reading.anchors[mode.index()])
            .filter(|anchor| anchor.matches(snapshot.record(), mode))
            .map(|anchor| anchor.position);
        self.mode = mode;
        self.readings.last_mut().expect("remembered reading").mode = mode;
        Ok(())
    }
}

fn valid(
    record: &SessionRecord,
    position: NativeManagedHistoryPosition,
    mode: NativeManagedHistoryMode,
) -> bool {
    let Some(message) = record.messages.get(position.message) else {
        return false;
    };
    if message.role == Role::System {
        return false;
    }
    match position.block {
        None => position.byte == 0,
        Some(index) => message.content.get(index).is_some_and(|block| {
            match detail::text(block, message.role, mode) {
                Ok(Some(text)) => text.is_char_boundary(position.byte),
                Ok(None) => position.byte == 0,
                Err(()) => false,
            }
        }),
    }
}
fn digest(record: &SessionRecord, position: NativeManagedHistoryPosition) -> Option<[u8; 32]> {
    struct Sink {
        digest: Sha256,
        bytes: usize,
    }
    impl io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes = self
                .bytes
                .checked_add(bytes.len())
                .filter(|bytes| *bytes <= crate::session_store::MAX_FILE_SESSION_BYTES)
                .ok_or_else(|| io::Error::other("history message bound"))?;
            self.digest.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink {
        digest: Sha256::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut sink, record.messages.get(position.message)?).ok()?;
    Some(sink.digest.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{Message, SessionId, SessionIncarnationId};

    #[test]
    fn structured_positions_are_valid_only_in_the_exact_full_detail_projection() {
        let mut record = SessionRecord::empty(
            SessionId::new("child").unwrap(),
            SessionIncarnationId::new("original").unwrap(),
        );
        record.messages.push(Message {
            role: Role::Tool,
            content: vec![machine_god_core::ContentBlock::Json {
                value: serde_json::json!({"value":"α🙂"}),
            }],
        });
        let view = NativeManagedHistoryView {
            record: &record,
            mode: NativeManagedHistoryMode::Full,
            position: None,
        };
        let text = view.block_text(0, 0).unwrap().unwrap();
        let position = NativeManagedHistoryPosition {
            message: 0,
            block: Some(0),
            byte: text.find('🙂').unwrap(),
        };
        assert!(valid(&record, position, NativeManagedHistoryMode::Full));
        assert!(!valid(
            &record,
            position,
            NativeManagedHistoryMode::Transcript
        ));
        assert!(!valid(
            &record,
            NativeManagedHistoryPosition {
                byte: position.byte + 1,
                ..position
            },
            NativeManagedHistoryMode::Full
        ));
        record.messages[0].role = Role::System;
        assert!(!valid(&record, position, NativeManagedHistoryMode::Full));
    }

    #[test]
    fn anchors_follow_original_messages_not_reused_indices_or_public_ids() {
        let owner = NativeObservedManagedAgent::test_observations(1).remove(0);
        let mut record = SessionRecord::empty(
            SessionId::new("child").unwrap(),
            SessionIncarnationId::new("original").unwrap(),
        );
        record.messages = vec![
            Message::text(Role::System, "system"),
            Message::text(Role::Assistant, "α🙂source"),
        ];
        let position = NativeManagedHistoryPosition {
            message: 1,
            block: Some(0),
            byte: 2,
        };
        let mode = NativeManagedHistoryMode::Conversation;
        assert!(valid(&record, position, mode));
        for invalid in [
            NativeManagedHistoryPosition {
                byte: 1,
                ..position
            },
            NativeManagedHistoryPosition {
                message: 0,
                ..position
            },
            NativeManagedHistoryPosition {
                block: Some(9),
                ..position
            },
            NativeManagedHistoryPosition {
                byte: usize::MAX,
                ..position
            },
        ] {
            assert!(!valid(&record, invalid, mode));
        }
        let anchor = Anchor {
            position,
            message_digest: digest(&record, position).unwrap(),
        };
        record
            .metadata
            .insert("unrelated".into(), serde_json::json!(true));
        record.messages.push(Message::text(Role::User, "appended"));
        let mut revised = owner.clone();
        revised.revision += 1;
        assert!(owner.same_conversation(&revised));
        assert!(anchor.matches(&record, mode));
        revised.generation += 1;
        assert!(!owner.same_conversation(&revised));
        let foreign = NativeObservedManagedAgent::test_observations(1).remove(0);
        assert!(!owner.same_conversation(&foreign));
        record.messages[1] = Message::text(Role::Assistant, "α🙂changed");
        assert!(!anchor.matches(&record, mode));
        record.messages.truncate(1);
        assert!(!anchor.matches(&record, mode));
    }
}
