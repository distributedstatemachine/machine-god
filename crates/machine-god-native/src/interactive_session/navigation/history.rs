//! One canonical history snapshot and bounded, source-checked reading positions.
use crate::{NativeManagedHistorySnapshot, NativeObservedManagedAgent};
use machine_god_core::{ContentBlock, Role, SessionRecord};
use sha2::{Digest, Sha256};
use std::{fmt, io};

/// Presentation coordinates only. The owner validates them against its exact
/// displayed canonical record; they grant no process or command authority.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct NativeManagedHistoryPosition {
    pub message: usize,
    /// None is the role heading; Some selects a canonical content block.
    pub block: Option<usize>,
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
    /// None follows the tail. An anchor remains independent of editor/frame ACK.
    pub position: Option<NativeManagedHistoryPosition>,
}
impl fmt::Debug for NativeManagedHistoryView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeManagedHistoryView { .. }")
    }
}

struct Reading {
    owner: NativeObservedManagedAgent,
    position: NativeManagedHistoryPosition,
    message_digest: [u8; 32],
}
impl Reading {
    fn matches(&self, owner: &NativeObservedManagedAgent, record: &SessionRecord) -> bool {
        self.owner.same_conversation(owner)
            && valid(record, self.position)
            && digest(record, self.position) == Some(self.message_digest)
    }
}
#[derive(Default)]
pub(super) struct History {
    snapshot: Option<NativeManagedHistorySnapshot>,
    readings: Vec<Reading>,
    position: Option<NativeManagedHistoryPosition>,
}
impl History {
    pub(super) fn clear(&mut self) {
        self.snapshot = None;
        self.position = None;
    }
    pub(super) fn install(&mut self, snapshot: NativeManagedHistorySnapshot) {
        self.position = self
            .readings
            .iter()
            .find(|reading| reading.matches(snapshot.observation(), snapshot.record()))
            .map(|reading| reading.position);
        self.snapshot = Some(snapshot);
    }
    pub(super) fn view(&self) -> Option<NativeManagedHistoryView<'_>> {
        Some(NativeManagedHistoryView {
            record: self.snapshot.as_ref()?.record(),
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
                if !valid(snapshot.record(), position) {
                    return Err(super::NativeManagedNavigationError::InvalidAction);
                }
                Ok(Reading {
                    owner: snapshot.observation().clone(),
                    position,
                    // The installed position was checked against this immutable
                    // snapshot. Scrolling within its message need not hash a
                    // potentially multi-megabyte text again on every keypress.
                    message_digest: self
                        .position
                        .filter(|current| current.message == position.message)
                        .and_then(|_| {
                            self.readings.iter().find(|reading| {
                                reading.owner.same_conversation(snapshot.observation())
                            })
                        })
                        .map(|reading| reading.message_digest)
                        .or_else(|| digest(snapshot.record(), position))
                        .ok_or(super::NativeManagedNavigationError::InvalidAction)?,
                })
            })
            .transpose()?;
        self.readings
            .retain(|reading| !reading.owner.same_conversation(snapshot.observation()));
        if let Some(reading) = reading {
            if self.readings.len() == 128 {
                self.readings.remove(0);
            }
            self.readings.push(reading);
        }
        self.position = position;
        Ok(())
    }
}

fn valid(record: &SessionRecord, position: NativeManagedHistoryPosition) -> bool {
    let Some(message) = record.messages.get(position.message) else {
        return false;
    };
    if message.role == Role::System {
        return false;
    }
    match position.block {
        None => position.byte == 0,
        Some(index) => match message.content.get(index) {
            Some(ContentBlock::Text { text })
                if matches!(message.role, Role::User | Role::Assistant) =>
            {
                text.is_char_boundary(position.byte)
            }
            Some(_) => position.byte == 0,
            None => false,
        },
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
        assert!(valid(&record, position));
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
            assert!(!valid(&record, invalid));
        }
        let reading = Reading {
            owner: owner.clone(),
            position,
            message_digest: digest(&record, position).unwrap(),
        };
        record
            .metadata
            .insert("unrelated".into(), serde_json::json!(true));
        record.messages.push(Message::text(Role::User, "appended"));
        let mut revised = owner.clone();
        revised.revision += 1;
        assert!(reading.matches(&revised, &record));
        revised.generation += 1;
        assert!(!reading.matches(&revised, &record));
        let foreign = NativeObservedManagedAgent::test_observations(1).remove(0);
        assert!(!reading.matches(&foreign, &record));
        record.messages[1] = Message::text(Role::Assistant, "α🙂changed");
        assert!(!reading.matches(&owner, &record));
        record.messages.truncate(1);
        assert!(!reading.matches(&owner, &record));
    }
}
