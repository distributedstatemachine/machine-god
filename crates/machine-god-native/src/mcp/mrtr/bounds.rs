use super::{Error, McpMrtrLimits, Result};
use crate::mcp::{
    protocol::{WireLimits, parse_json},
    schema::McpSchemaLimits,
};
use serde_json::{Value, value::RawValue};
use std::collections::BTreeMap;

pub(super) type Object<'a> = BTreeMap<String, &'a RawValue>;
#[derive(Clone, Copy)]
pub(super) struct FormBounds {
    pub message: usize,
    pub fields: usize,
    pub options: usize,
    pub label: usize,
}
impl FormBounds {
    pub fn nested(limits: McpMrtrLimits) -> Self {
        Self {
            message: limits.max_string_bytes,
            fields: limits.max_collection_items,
            options: limits.max_collection_items,
            label: limits.max_string_bytes,
        }
    }
    pub fn direct(limits: McpMrtrLimits) -> Self {
        Self {
            message: limits.max_string_bytes.min(8 * 1024),
            fields: limits.max_collection_items.min(64),
            options: limits.max_collection_items.min(64),
            label: limits.max_string_bytes.min(1024),
        }
    }
}
pub(super) fn object(raw: &RawValue) -> Result<Object<'_>> {
    serde_json::from_str(raw.get()).map_err(|_| Error::InvalidRequest)
}
/// Ordered raw-span projection after shared strict duplicate/budget admission.
pub(super) fn entries(raw: &RawValue) -> Result<Vec<(String, &RawValue)>> {
    struct Entries;
    impl<'de> serde::de::Visitor<'de> for Entries {
        type Value = Vec<(String, &'de RawValue)>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an admitted object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = map.next_entry()? {
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    let mut decoder = serde_json::Deserializer::from_str(raw.get());
    serde::Deserializer::deserialize_map(&mut decoder, Entries).map_err(|_| Error::InvalidRequest)
}
pub(super) fn array(raw: &RawValue) -> Result<Vec<&RawValue>> {
    serde_json::from_str(raw.get()).map_err(|_| Error::InvalidRequest)
}
pub(super) fn text(raw: &RawValue, maximum: usize) -> Result<Box<str>> {
    let value: String = serde_json::from_str(raw.get()).map_err(|_| Error::InvalidRequest)?;
    if value.len() > maximum {
        return Err(Error::Limit);
    }
    Ok(value.into_boxed_str())
}
pub(super) fn required<'a>(object: &'a Object<'a>, key: &str) -> Result<&'a RawValue> {
    object.get(key).copied().ok_or(Error::InvalidRequest)
}
pub(super) fn optional_text(
    object: &Object<'_>,
    key: &str,
    maximum: usize,
) -> Result<Option<Box<str>>> {
    object.get(key).map(|raw| text(raw, maximum)).transpose()
}
pub(super) fn compact_len(raw: &RawValue) -> Result<usize> {
    // Caller already admitted the entire bounded duplicate-free input. The
    // shared source-aware Value preserves exact numbers while removing JSON
    // whitespace and normalizing string escapes, as the producer stringifier.
    let value = machine_god_core::json::from_str(raw.get()).map_err(|_| Error::InvalidJson)?;
    serde_json::to_vec(&value)
        .map(|bytes| bytes.len())
        .map_err(|_| Error::InvalidJson)
}
pub(super) fn number(
    raw: &RawValue,
    limits: McpMrtrLimits,
) -> Result<crate::mcp::schema::number::Number> {
    if !matches!(raw.get().as_bytes().first(), Some(b'-' | b'0'..=b'9')) {
        return Err(Error::InvalidSchema);
    }
    crate::mcp::schema::number::Number::parse(
        raw.get(),
        limits.scalar_limits(),
        crate::mcp::schema::McpSchemaError::InvalidSchema,
    )
    .map_err(|_| Error::Limit)
}
impl McpMrtrLimits {
    pub(super) fn validate(self) -> Result<Self> {
        let cap = Self::default();
        for (value, max) in [
            (self.max_requests, cap.max_requests),
            (self.max_name_bytes, cap.max_name_bytes),
            (self.max_json_bytes, cap.max_json_bytes),
            (self.max_string_bytes, cap.max_string_bytes),
            (self.max_collection_items, cap.max_collection_items),
            (self.max_depth, cap.max_depth),
            (self.max_nodes, cap.max_nodes),
            (self.max_retained_bytes, cap.max_retained_bytes),
            (self.max_pattern_bytes, cap.max_pattern_bytes),
            (self.max_pattern_depth, cap.max_pattern_depth),
            (self.max_pattern_states, cap.max_pattern_states),
            (self.max_pattern_repeat, cap.max_pattern_repeat),
            (self.max_pattern_steps, cap.max_pattern_steps),
            (self.max_number_bytes, cap.max_number_bytes),
            (
                self.max_number_expanded_digits,
                cap.max_number_expanded_digits,
            ),
        ] {
            if value == 0 || value > max {
                return Err(Error::InvalidLimits);
            }
        }
        if self.max_number_exponent_abs <= 0
            || self.max_number_exponent_abs > cap.max_number_exponent_abs
        {
            return Err(Error::InvalidLimits);
        }
        Ok(self)
    }
    pub(super) fn scalar_limits(self) -> McpSchemaLimits {
        McpSchemaLimits {
            max_depth: self.max_pattern_depth,
            max_pattern_states: self.max_pattern_states,
            max_pattern_repeat: self.max_pattern_repeat,
            max_pattern_steps: self.max_pattern_steps,
            max_number_bytes: self.max_number_bytes,
            max_number_exponent_abs: self.max_number_exponent_abs,
            max_number_expanded_digits: self.max_number_expanded_digits,
            ..McpSchemaLimits::default()
        }
    }
}

/// Shared wire admission rejects duplicates before borrowed maps can overwrite
/// them. The returned charge is checked before any long-lived copies are made.
pub(super) fn admit(raw: &RawValue, limits: McpMrtrLimits) -> Result<usize> {
    let limits = limits.validate()?;
    let value = parse_json(
        raw.get().as_bytes(),
        WireLimits {
            max_frame_bytes: limits.max_json_bytes,
            max_nodes: limits.max_nodes,
            max_depth: limits.max_depth + 1,
        },
    )
    .map_err(|_| Error::InvalidJson)?;
    let nodes = check(&value, limits)?;
    let charge = raw
        .get()
        .len()
        .checked_mul(6)
        .and_then(|bytes| {
            nodes
                .checked_mul(256)
                .and_then(|extra| bytes.checked_add(extra))
        })
        .and_then(|bytes| bytes.checked_add(4096))
        .ok_or(Error::Limit)?;
    if charge > limits.max_retained_bytes {
        return Err(Error::Limit);
    }
    Ok(charge)
}
fn check(value: &Value, limits: McpMrtrLimits) -> Result<usize> {
    let mut nodes = 1;
    match value {
        Value::String(text) if text.len() > limits.max_string_bytes => return Err(Error::Limit),
        Value::Number(number) if number.as_str().len() > limits.max_string_bytes => {
            return Err(Error::Limit);
        }
        Value::Array(values) => {
            if values.len() > limits.max_collection_items {
                return Err(Error::Limit);
            }
            for child in values {
                nodes += check(child, limits)?;
            }
        }
        Value::Object(values) => {
            if values.len() > limits.max_collection_items {
                return Err(Error::Limit);
            }
            for (key, child) in values {
                if key.len() > limits.max_name_bytes {
                    return Err(Error::Limit);
                }
                nodes += 1 + check(child, limits)?;
            }
        }
        _ => {}
    }
    Ok(nodes)
}
