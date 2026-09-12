//! Allocation-bounded node preflight over already admitted JSON source.
//!
//! Core's source-shape visitor preserves numeric scalars and literal private
//! object keys. No `Value` tree or collection of children is allocated here.

use machine_god_core::{CancellationToken, ToolError, json};
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;
use std::fmt;

pub(super) fn charge(
    raw: &RawValue,
    remaining: &mut usize,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let mut decoder = serde_json::Deserializer::from_str(raw.get());
    let result = Count {
        remaining,
        cancellation,
    }
    .deserialize(&mut decoder);
    // Do not disguise a cancellation observed inside the visitor as overflow.
    crate::mcp_features::check_cancellation(cancellation)?;
    result.map_err(|_| super::limit())?;
    decoder.end().map_err(|_| super::limit())
}

struct Count<'a> {
    remaining: &'a mut usize,
    cancellation: &'a CancellationToken,
}

impl<'de> DeserializeSeed<'de> for Count<'_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        if self.cancellation.is_cancelled() {
            return Err(de::Error::custom("cancelled"));
        }
        *self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| de::Error::custom("node limit"))?;
        json::visit(decoder, self, |_| ())
    }
}

impl<'de> Visitor<'de> for Count<'_> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bounded admitted JSON")
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        while sequence
            .next_element_seed(Count {
                remaining: self.remaining,
                cancellation: self.cancellation,
            })?
            .is_some()
        {}
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map.next_key::<IgnoredAny>()?.is_some() {
            map.next_value_seed(Count {
                remaining: self.remaining,
                cancellation: self.cancellation,
            })?;
        }
        Ok(())
    }
}
