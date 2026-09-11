//! Seeded decoding charges recursion and allocation before accepting children.

use std::fmt;

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use super::wire::{WireError, WireLimits};

pub(super) fn parse(bytes: &[u8], limits: WireLimits) -> Result<Value, WireError> {
    let mut remaining = limits.max_nodes;
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Seed {
        remaining: &mut remaining,
        depth: 1,
        max_depth: limits.max_depth,
    }
    .deserialize(&mut decoder)
    .map_err(|_| WireError::InvalidJson)?;
    decoder.end().map_err(|_| WireError::InvalidJson)?;
    Ok(value)
}

struct Seed<'a> {
    remaining: &'a mut usize,
    depth: usize,
    max_depth: usize,
}

impl Seed<'_> {
    fn charge<E: de::Error>(&mut self) -> Result<(), E> {
        if *self.remaining == 0 || self.depth > self.max_depth {
            return Err(E::custom("JSON limit"));
        }
        *self.remaining -= 1;
        Ok(())
    }

    fn child(&mut self) -> Seed<'_> {
        Seed {
            remaining: self.remaining,
            depth: self.depth + 1,
            max_depth: self.max_depth,
        }
    }
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(mut self, decoder: D) -> Result<Value, D::Error> {
        self.charge()?;
        decoder.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded JSON value")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid number"))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }
    fn visit_seq<A: SeqAccess<'de>>(mut self, mut sequence: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(self.child())? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(mut self, mut object: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        // Keys count as nodes and use the same depth as their containing object.
        while let Some(key) = object.next_key_seed(KeySeed {
            remaining: self.remaining,
        })? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate key"));
            }
            let value = object.next_value_seed(self.child())?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

struct KeySeed<'a> {
    remaining: &'a mut usize,
}
impl<'de> DeserializeSeed<'de> for KeySeed<'_> {
    type Value = String;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<String, D::Error> {
        if *self.remaining == 0 {
            return Err(de::Error::custom("JSON limit"));
        }
        *self.remaining -= 1;
        serde::Deserialize::deserialize(decoder)
    }
}
