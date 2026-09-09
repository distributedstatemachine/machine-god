use super::super::{FILE_SESSION_SCHEMA_VERSION, ObjectOnly, StoredMessage, validate_record_json};
use super::{Error, NativeSessionMetadata, decode};
use machine_god_core::{
    ContentBlock, Message, Role, SessionId, SessionIncarnationId, SessionRecord, SessionRevision,
    ToolCallId, ToolOutput,
};
use serde::de::{DeserializeSeed, SeqAccess};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::{collections::BTreeMap, fmt};

/// Reject duplicates even in arbitrary embedded values. An EOF can be salvaged
/// only later by the canonical-prefix decoder, never by constructing JSON text.
pub(super) fn check_duplicate_keys(bytes: &[u8]) -> Result<(), Error> {
    struct Unique;
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
            struct UniqueVisitor;
            impl<'de> Visitor<'de> for UniqueVisitor {
                type Value = Unique;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("unique JSON keys")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
                    let mut keys = BTreeSet::new();
                    while let Some(key) = map.next_key::<String>()? {
                        if !keys.insert(key) {
                            return Err(de::Error::custom("duplicate key"));
                        }
                        map.next_value::<Unique>()?;
                    }
                    Ok(Unique)
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Unique, A::Error> {
                    while seq.next_element::<Unique>()?.is_some() {}
                    Ok(Unique)
                }
                fn visit_bool<E: de::Error>(self, _: bool) -> Result<Unique, E> {
                    Ok(Unique)
                }
                fn visit_i64<E: de::Error>(self, _: i64) -> Result<Unique, E> {
                    Ok(Unique)
                }
                fn visit_u64<E: de::Error>(self, _: u64) -> Result<Unique, E> {
                    Ok(Unique)
                }
                fn visit_f64<E: de::Error>(self, _: f64) -> Result<Unique, E> {
                    Ok(Unique)
                }
                fn visit_str<E: de::Error>(self, _: &str) -> Result<Unique, E> {
                    Ok(Unique)
                }
                fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique)
                }
            }
            de.deserialize_any(UniqueVisitor)
        }
    }
    match serde_json::from_slice::<Unique>(bytes) {
        Ok(_) => Ok(()),
        Err(error) if error.is_eof() => Ok(()),
        Err(_) => Err(Error::Corrupt),
    }
}

pub(super) fn decode_source(bytes: &[u8], id: &SessionId) -> Result<(SessionRecord, bool), Error> {
    match decode(bytes, id) {
        Ok(record) => {
            if NativeSessionMetadata::from_metadata(&record.metadata)
                == Err(crate::NativeSessionMetadataError::UnsupportedVersion)
            {
                return Err(Error::UnsupportedVersion);
            }
            return Ok((record, false));
        }
        Err(Error::UnsupportedVersion) => return Err(Error::UnsupportedVersion),
        Err(_) => {}
    }
    check_duplicate_keys(bytes)?;
    let mut state = Prefix::default();
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let error = Envelope(&mut state)
        .deserialize(&mut decoder)
        .err()
        .ok_or(Error::Corrupt)?;
    if !error.is_eof()
        || !state.header_complete
        || state.messages.is_empty()
        || state.metadata_started
    {
        return Err(Error::Corrupt);
    }
    let record = SessionRecord {
        id: state.id.ok_or(Error::Corrupt)?,
        incarnation_id: state.incarnation.ok_or(Error::Corrupt)?,
        revision: state.revision.ok_or(Error::Corrupt)?,
        next_turn_sequence: state.next_turn.ok_or(Error::Corrupt)?,
        messages: state.messages,
        metadata: BTreeMap::new(),
    };
    if &record.id != id || record.revision.0 == 0 || record.next_turn_sequence == 0 {
        return Err(Error::Corrupt);
    }
    Ok((record, true))
}

#[derive(Default)]
struct Prefix {
    id: Option<SessionId>,
    incarnation: Option<SessionIncarnationId>,
    revision: Option<SessionRevision>,
    next_turn: Option<u64>,
    header_complete: bool,
    metadata_started: bool,
    messages: Vec<Message>,
}
struct Envelope<'a>(&'a mut Prefix);
impl<'de> DeserializeSeed<'de> for Envelope<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Envelope<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("canonical native envelope")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        key(&mut map, "schema_version")?;
        if map.next_value::<u32>()? != FILE_SESSION_SCHEMA_VERSION {
            return Err(de::Error::custom("unsupported version"));
        }
        key(&mut map, "record")?;
        map.next_value_seed(Record(self.0))?;
        if map.next_key::<String>()?.is_some() {
            return Err(de::Error::custom("extra envelope key"));
        }
        Ok(())
    }
}
fn key<'de, A: MapAccess<'de>>(map: &mut A, expected: &str) -> Result<(), A::Error> {
    if map.next_key::<String>()?.as_deref() != Some(expected) {
        return Err(de::Error::custom("noncanonical recovery prefix"));
    }
    Ok(())
}
struct Record<'a>(&'a mut Prefix);
impl<'de> DeserializeSeed<'de> for Record<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Record<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("canonical native record")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        key(&mut map, "id")?;
        self.0.id = Some(map.next_value()?);
        key(&mut map, "incarnation_id")?;
        self.0.incarnation = Some(map.next_value()?);
        key(&mut map, "revision")?;
        self.0.revision = Some(map.next_value()?);
        key(&mut map, "next_turn_sequence")?;
        self.0.next_turn = Some(map.next_value()?);
        key(&mut map, "messages")?;
        self.0.header_complete = true;
        map.next_value_seed(Messages(&mut self.0.messages))?;
        self.0.metadata_started = true;
        key(&mut map, "metadata")?;
        map.next_value::<BTreeMap<String, Value>>()?;
        if map.next_key::<String>()?.is_some() {
            return Err(de::Error::custom("extra record key"));
        }
        Ok(())
    }
}
struct Messages<'a>(&'a mut Vec<Message>);
impl<'de> DeserializeSeed<'de> for Messages<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Messages<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("complete messages")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(message) = seq.next_element::<ObjectOnly<StoredMessage>>()? {
            if self.0.len()
                >= machine_god_core::EngineLimits::default()
                    .max_transcript_messages
                    .get()
            {
                return Err(de::Error::custom("message limit"));
            }
            self.0.push(message.0.into());
        }
        Ok(())
    }
}

pub(super) fn close_tools(record: &mut SessionRecord) -> Result<usize, Error> {
    let mut pending = BTreeSet::new();
    let mut messages = Vec::with_capacity(record.messages.len());
    let mut unknown = 0;
    for message in std::mem::take(&mut record.messages) {
        if message.role != Role::Tool {
            close_pending(&mut messages, &mut pending, &mut unknown);
        }
        if message.role == Role::Tool && message.content.is_empty() {
            return Err(Error::Corrupt);
        }
        for block in &message.content {
            match block {
                ContentBlock::ToolCall { call } => {
                    if message.role != Role::Assistant || !pending.insert(call.id.clone()) {
                        return Err(Error::Corrupt);
                    }
                }
                ContentBlock::ToolResult { call_id, .. } => {
                    if message.role != Role::Tool || !pending.remove(call_id) {
                        return Err(Error::Corrupt);
                    }
                }
                _ if message.role == Role::Tool => return Err(Error::Corrupt),
                _ => {}
            }
        }
        messages.push(message);
    }
    close_pending(&mut messages, &mut pending, &mut unknown);
    record.messages = messages;
    Ok(unknown)
}
fn close_pending(
    messages: &mut Vec<Message>,
    pending: &mut BTreeSet<ToolCallId>,
    unknown: &mut usize,
) {
    if pending.is_empty() {
        return;
    }
    let content = std::mem::take(pending).into_iter().map(|call_id| {
        *unknown += 1;
        ContentBlock::ToolResult {call_id, output: ToolOutput {
            content: serde_json::json!({"code":"tool_result_unknown","message":"tool result status is unknown"}),
            is_error: true,
        }}
    }).collect();
    messages.push(Message {
        role: Role::Tool,
        content,
    });
}
pub(super) fn validate_limits(record: &SessionRecord) -> Result<(), Error> {
    let limits = machine_god_core::EngineLimits::default();
    if validate_record_json(record).is_err() {
        return Err(Error::Corrupt);
    }
    if record.messages.len() > limits.max_transcript_messages.get()
        || serde_json::to_vec(&record.messages)
            .map_err(|_| Error::Corrupt)?
            .len()
            > limits.max_transcript_bytes.get()
        || serde_json::to_vec(&record.metadata)
            .map_err(|_| Error::Corrupt)?
            .len()
            > limits.max_session_metadata_bytes.get()
    {
        return Err(Error::Oversized);
    }
    Ok(())
}
