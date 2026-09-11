//! Strict, budgeted JSON admission before semantic decoding.
use std::fmt;
use std::io::{self, Write};

use serde::Serialize;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use super::{MAX_CONFIG_BYTES, MAX_STRING_BYTES, McpConfigError};

#[derive(Default)]
struct Budget {
    nodes: usize,
    strings: usize,
    exceeded: bool,
    server_order: Vec<String>,
}

pub(super) fn decode(bytes: &[u8]) -> Result<Value, McpConfigError> {
    decode_ordered(bytes).map(|(value, _)| value)
}

pub(super) fn decode_ordered(bytes: &[u8]) -> Result<(Value, Vec<String>), McpConfigError> {
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(McpConfigError::Limit);
    }
    machine_god_core::json::check_container_depth(bytes, 9).map_err(|_| McpConfigError::Limit)?;
    let mut budget = Budget::default();
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let result = Seed {
        budget: &mut budget,
        depth: 0,
    }
    .deserialize(&mut de);
    let value = result.map_err(|_| {
        if budget.exceeded {
            McpConfigError::Limit
        } else {
            McpConfigError::Invalid
        }
    })?;
    de.end().map_err(|_| McpConfigError::Invalid)?;
    Ok((value, budget.server_order))
}

struct Seed<'a> {
    budget: &'a mut Budget,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        self.budget.nodes += 1;
        if self.depth > 8 || self.budget.nodes > 16_384 {
            self.budget.exceeded = true;
            return Err(D::Error::custom("MCP JSON limit"));
        }
        machine_god_core::json::visit(de, self, Value::Number)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded MCP JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid number"))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
        charge_string(self.budget, value)?;
        Ok(Value::String(value.to_owned()))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(Seed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key_seed(Key(self.budget))? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate MCP JSON key"));
            }
            if self.depth == 1 {
                self.budget.server_order.push(key.clone());
            }
            let value = map.next_value_seed(Seed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

fn charge_string<E: serde::de::Error>(budget: &mut Budget, value: &str) -> Result<(), E> {
    if value.len() > 16 * 1024 || value.len() > MAX_STRING_BYTES - budget.strings {
        budget.exceeded = true;
        return Err(E::custom("MCP JSON string limit"));
    }
    budget.strings += value.len();
    Ok(())
}

struct Key<'a>(&'a mut Budget);
impl<'de> DeserializeSeed<'de> for Key<'_> {
    type Value = String;
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<String, D::Error> {
        de.deserialize_str(self)
    }
}
impl Visitor<'_> for Key<'_> {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded MCP key")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<String, E> {
        charge_string(self.0, value)?;
        Ok(value.to_owned())
    }
}

pub(super) fn encode(value: &impl Serialize) -> Result<Vec<u8>, McpConfigError> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_CONFIG_BYTES - self.0.len() {
                return Err(io::Error::other("MCP configuration limit"));
            }
            if self.0.len() + bytes.len() > self.0.capacity() {
                let desired = (self.0.len() + bytes.len())
                    .next_power_of_two()
                    .min(MAX_CONFIG_BYTES);
                self.0.reserve_exact(desired - self.0.len());
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded(Vec::new());
    serde_json::to_writer(&mut writer, value).map_err(|_| McpConfigError::Limit)?;
    Ok(writer.0.into_boxed_slice().into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_numbers_do_not_create_phantom_servers_or_consume_depth() {
        let text = br#"{"mcpServers":{"z":1e400,"a":1e-400,"$serde_json::private::Number":-0}}"#;
        let (value, order) = decode_ordered(text).unwrap();
        assert_eq!(order, ["z", "a", "$serde_json::private::Number"]);
        assert_eq!(value["mcpServers"]["z"].to_string(), "1e400");
        assert_eq!(value["mcpServers"]["a"].to_string(), "1e-400");
        assert_eq!(
            value["mcpServers"]["$serde_json::private::Number"].to_string(),
            "-0"
        );
        assert!(
            decode(
                br#"{"x":{"$serde_json::private::RawValue":1,"$serde_json::private::RawValue":2}}"#
            )
            .is_err()
        );
    }
}
