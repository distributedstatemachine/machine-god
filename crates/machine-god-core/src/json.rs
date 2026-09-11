//! Lossless JSON decoding for provider-neutral values.
//!
//! Numbers are parsed from their source tokens, never through binary floats.
//! Real objects are decoded as maps, including keys which happen to match
//! `serde_json`'s private number or raw-value representation.

use std::{borrow::Cow, collections::BTreeMap, fmt};

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value, value::RawValue};

const MAX_DOCUMENT_DEPTH: usize = 128;

/// Dispatches a JSON value by source shape while retaining the caller's visitor
/// and its allocation/depth/node policy. A number consumes one scalar node.
///
/// The visitor must produce owned data. The input may be JSON text or an
/// already-owned `serde_json` value; other serde formats are not supported.
///
/// # Errors
/// Returns malformed input, unsupported representation, or visitor errors.
pub fn visit<'de, D, V, F, T>(decoder: D, visitor: V, number: F) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    V: for<'a> Visitor<'a, Value = T>,
    F: FnOnce(Number) -> T,
{
    let raw = RawToken::deserialize(decoder)?;
    let text = raw.0.as_ref();
    if matches!(text.as_bytes().first(), Some(b'-' | b'0'..=b'9')) {
        // RawValue has already validated the complete token. Parsing through
        // Number::from_str would normalize integer -0 and exponent spelling.
        return Ok(number(Number::from_string_unchecked(text.to_owned())));
    }
    let mut decoder = serde_json::Deserializer::from_str(text);
    let value = match text.as_bytes().first() {
        Some(b'{') => decoder.deserialize_map(visitor),
        Some(b'[') => decoder.deserialize_seq(visitor),
        _ => decoder.deserialize_any(visitor),
    }
    .map_err(de::Error::custom)?;
    decoder.end().map_err(de::Error::custom)?;
    Ok(value)
}

// serde_json's raw-value newtype protocol is intentionally used only here.
// Unlike Box<RawValue>, this borrows source slices while walking children, so
// deeply nested input does not retain a separate owned copy of every subtree.
// Value deserializers supply an owned serialization through the same protocol.
struct RawToken<'a>(Cow<'a, str>);

impl<'de> Deserialize<'de> for RawToken<'de> {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        const TOKEN: &str = "$serde_json::private::RawValue";
        struct RawVisitor;
        struct Key;
        struct Text;
        impl<'de> DeserializeSeed<'de> for Key {
            type Value = bool;
            fn deserialize<D: Deserializer<'de>>(self, decoder: D) -> Result<bool, D::Error> {
                decoder.deserialize_str(self)
            }
        }
        impl Visitor<'_> for Key {
            type Value = bool;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("raw JSON marker")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<bool, E> {
                Ok(value == TOKEN)
            }
        }
        impl<'de> DeserializeSeed<'de> for Text {
            type Value = RawToken<'de>;
            fn deserialize<D: Deserializer<'de>>(
                self,
                decoder: D,
            ) -> Result<Self::Value, D::Error> {
                decoder.deserialize_str(self)
            }
        }
        impl<'de> Visitor<'de> for Text {
            type Value = RawToken<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("raw JSON text")
            }
            fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
                Ok(RawToken(Cow::Borrowed(value)))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(RawToken(Cow::Owned(value.to_owned())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(RawToken(Cow::Owned(value)))
            }
        }
        impl<'de> Visitor<'de> for RawVisitor {
            type Value = RawToken<'de>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("raw JSON value")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                if map.next_key_seed(Key)? != Some(true) {
                    return Err(de::Error::custom("invalid raw JSON representation"));
                }
                map.next_value_seed(Text)
            }
        }
        decoder.deserialize_newtype_struct(TOKEN, RawVisitor)
    }
}

/// Deserializes a JSON value without interpreting literal private-looking keys.
/// As with ordinary `serde_json::Value` fields, duplicate object members use the
/// last value. Boundary codecs retain their own stricter duplicate policies.
///
/// # Errors
/// Returns malformed input or values deeper than 64 containers.
pub fn deserialize<'de, D: Deserializer<'de>>(decoder: D) -> Result<Value, D::Error> {
    Seed {
        depth: 0,
        max_depth: crate::MAX_SAFE_JSON_DEPTH,
        reject_duplicates: false,
    }
    .deserialize(decoder)
}

/// Decodes a complete JSON document, preserving arbitrary-precision numbers.
/// Documents have a fixed ceiling of 128 containers; native boundary codecs
/// apply their own smaller limits to the relevant payload roots.
///
/// # Errors
/// Rejects malformed JSON, duplicate keys and excessive container depth.
pub fn from_str(text: &str) -> Result<Value, serde_json::Error> {
    check_container_depth(text.as_bytes(), MAX_DOCUMENT_DEPTH)?;
    let mut decoder = serde_json::Deserializer::from_str(text);
    let value = Seed {
        depth: 0,
        max_depth: MAX_DOCUMENT_DEPTH,
        reject_duplicates: true,
    }
    .deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}

/// Decodes UTF-8 JSON bytes using the lossless value contract.
///
/// # Errors
/// Rejects invalid UTF-8, malformed JSON, duplicates and excessive depth.
pub fn from_slice(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    check_container_depth(bytes, MAX_DOCUMENT_DEPTH)?;
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Seed {
        depth: 0,
        max_depth: MAX_DOCUMENT_DEPTH,
        reject_duplicates: true,
    }
    .deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}

/// Rejects excessive lexical container nesting without allocating scratch.
/// This is a preflight, not a syntax validator: the subsequent JSON decoder
/// still owns UTF-8, delimiter, token, duplicate, and EOF validation.
///
/// # Errors
/// Returns an error as soon as nesting exceeds `maximum` containers.
pub fn check_container_depth(bytes: &[u8], maximum: usize) -> Result<(), serde_json::Error> {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for &byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth = depth
                        .checked_add(1)
                        .filter(|depth| *depth <= maximum)
                        .ok_or_else(|| {
                            <serde_json::Error as de::Error>::custom("JSON depth limit exceeded")
                        })?;
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

/// Converts a serializable value without the numeric normalization performed
/// by `serde_json::to_value`. In particular, integer negative zero is retained.
///
/// # Errors
/// Returns serialization errors or documents exceeding the decoding bounds.
pub fn to_value<T: serde::Serialize>(value: T) -> Result<Value, serde_json::Error> {
    from_str(&serde_json::to_string(&value)?)
}

/// Tests mathematical zero without underflowing tiny nonzero exponents.
#[must_use]
pub fn number_is_zero(number: &Number) -> bool {
    number
        .as_str()
        .split(['e', 'E'])
        .next()
        .is_some_and(|mantissa| {
            mantissa
                .bytes()
                .all(|byte| matches!(byte, b'-' | b'.' | b'0'))
        })
}

/// Deserializes metadata values using the same lossless JSON contract.
///
/// # Errors
/// Rejects malformed maps and invalid or excessively deep JSON values.
pub fn deserialize_map<'de, D: Deserializer<'de>>(
    decoder: D,
) -> Result<BTreeMap<String, Value>, D::Error> {
    #[derive(Deserialize)]
    struct Entry(#[serde(deserialize_with = "deserialize")] Value);
    BTreeMap::<String, Entry>::deserialize(decoder)
        .map(|map| map.into_iter().map(|(key, value)| (key, value.0)).collect())
}

struct Seed {
    depth: usize,
    max_depth: usize,
    reject_duplicates: bool,
}

impl<'de> de::DeserializeSeed<'de> for Seed {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, decoder: D) -> Result<Value, D::Error> {
        if self.depth > self.max_depth {
            return Err(de::Error::custom("JSON depth limit exceeded"));
        }
        visit(decoder, self, Value::Number)
    }
}

use serde::de::DeserializeSeed;

/// Decodes an internally tagged object through an externally tagged payload.
/// This avoids serde's intermediate Content representation, which cannot
/// distinguish exact numbers from literal objects with private-looking keys.
///
/// # Errors
/// Rejects missing/non-string tags, duplicate fields and invalid payloads.
pub fn deserialize_tagged<'de, D, T>(decoder: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    struct Fields;
    impl<'de> Visitor<'de> for Fields {
        type Value = BTreeMap<String, Box<RawValue>>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("tagged JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
            let mut fields = BTreeMap::new();
            while let Some(key) = entries.next_key::<String>()? {
                if fields.contains_key(&key) {
                    return Err(de::Error::custom("duplicate JSON object key"));
                }
                fields.insert(key, entries.next_value()?);
            }
            Ok(fields)
        }
    }
    let mut fields = decoder.deserialize_map(Fields)?;
    let tag = fields
        .remove("type")
        .ok_or_else(|| de::Error::custom("missing JSON type tag"))?;
    let tag: String = serde_json::from_str(tag.get()).map_err(de::Error::custom)?;
    let mut tagged = BTreeMap::new();
    let payload = serde_json::to_string(&fields).map_err(de::Error::custom)?;
    tagged.insert(
        tag,
        RawValue::from_string(payload).map_err(de::Error::custom)?,
    );
    let text = serde_json::to_string(&tagged).map_err(de::Error::custom)?;
    serde_json::from_str(&text).map_err(de::Error::custom)
}

// Keep each enum's public schema in one declaration, but do not deserialize
// through serde's internally tagged buffering of arbitrary JSON values.
macro_rules! tagged {
    ($(#[$attr:meta])* $vis:vis enum $name:ident {
        $($(#[$variant_attr:meta])* $variant:ident $( {
            $($(#[$field_attr:meta])* $field:ident : $ty:ty),* $(,)?
        } )?),* $(,)?
    }) => {
        $(#[$attr])* $vis enum $name {
            $($(#[$variant_attr])* $variant $( {
                $($(#[$field_attr])* $field: $ty),*
            } )?),*
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "snake_case")]
                enum Payload {
                    $($variant { $($($(#[$field_attr])* $field: $ty),*)? }),*
                }
                let payload: Payload = $crate::json::deserialize_tagged(decoder)?;
                Ok(match payload { $(Payload::$variant { $($($field),*)? } => Self::$variant $( { $($field),* } )?),* })
            }
        }
    };
}
pub(crate) use tagged;

impl<'de> Visitor<'de> for Seed {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        if self.depth >= self.max_depth {
            return Err(de::Error::custom("JSON depth limit exceeded"));
        }
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(Seed {
            depth: self.depth + 1,
            max_depth: self.max_depth,
            reject_duplicates: self.reject_duplicates,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        if self.depth >= self.max_depth {
            return Err(de::Error::custom("JSON depth limit exceeded"));
        }
        let mut values = Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            if self.reject_duplicates && values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            values.insert(
                key,
                entries.next_value_seed(Seed {
                    depth: self.depth + 1,
                    max_depth: self.max_depth,
                    reject_duplicates: self.reject_duplicates,
                })?,
            );
        }
        Ok(Value::Object(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + fmt::Debug>(
        value: &T,
    ) {
        let bytes = serde_json::to_vec(value).unwrap();
        assert_eq!(&serde_json::from_slice::<T>(&bytes).unwrap(), value);
        assert_eq!(
            &serde_json::from_value::<T>(to_value(value).unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn exact_numbers_and_literal_private_keys() {
        for token in ["9007199254740993.0", "1e400", "1e-400", "-0", "-0.0"] {
            let value = from_str(token).unwrap();
            assert_eq!(value.as_number().unwrap().to_string(), token);
        }
        for key in [
            "$serde_json::private::Number",
            "$serde_json::private::RawValue",
        ] {
            let text = format!("{{\"{key}\":\"1e400\"}}");
            let value = from_str(&text).unwrap();
            assert!(value.is_object());
            assert_eq!(serde_json::to_string(&value).unwrap(), text);
        }
    }

    #[test]
    fn typed_history_capability_and_event_roundtrips_preserve_source_values() {
        use crate::{
            Capability, ContentBlock, ModelEvent, ToolCall, ToolCallId, ToolName, ToolOutput,
            ToolSpec, TurnEvent,
        };
        let arguments = from_str(r#"{"n":9007199254740993.0000000001,"large":1e400,"tiny":1e-400,"zero":-0,"$serde_json::private::Number":"1","nested":{"$serde_json::private::RawValue":"false"}}"#).unwrap();
        let call = ToolCall {
            id: ToolCallId::new("call-1").unwrap(),
            name: ToolName::new("mcp.exact").unwrap(),
            arguments: arguments.clone(),
        };
        roundtrip(&ToolSpec {
            name: call.name.clone(),
            description: "exact".into(),
            input_schema: arguments.clone(),
        });
        roundtrip(&ContentBlock::Json {
            value: arguments.clone(),
        });
        roundtrip(&ContentBlock::ToolCall { call: call.clone() });
        roundtrip(&ContentBlock::ToolResult {
            call_id: call.id.clone(),
            output: ToolOutput::success(arguments.clone()),
        });
        roundtrip(&Capability::Tool {
            name: call.name.clone(),
            call_id: call.id.clone(),
            arguments: arguments.clone(),
        });
        roundtrip(&Capability::Custom {
            name: "custom".into(),
            details: arguments.clone(),
        });
        roundtrip(&TurnEvent::Model {
            event: ModelEvent::ToolCall { call },
        });
        let metadata = BTreeMap::from([("value".into(), arguments)]);
        roundtrip(&crate::InferenceOptions {
            metadata,
            ..Default::default()
        });
    }

    #[test]
    fn exact_zero_does_not_confuse_underflow_with_zero() {
        for token in ["0", "-0", "-0.000e-999999", "0e999999"] {
            assert!(number_is_zero(
                from_str(token).unwrap().as_number().unwrap()
            ));
        }
        for token in ["1e-400", "-1e-400", "0.00000000000001e-999999"] {
            assert!(!number_is_zero(
                from_str(token).unwrap().as_number().unwrap()
            ));
        }
    }

    #[test]
    fn raw_tokens_borrow_source_and_literal_keys_are_not_a_private_protocol() {
        let mut decoder = serde_json::Deserializer::from_str("{\"n\":1e400}");
        assert!(matches!(
            RawToken::deserialize(&mut decoder).unwrap().0,
            Cow::Borrowed(_)
        ));
        for text in [
            r#"{"$serde_json::private::Number":null}"#,
            r#"{"$serde_json::private::RawValue":"invalid raw JSON"}"#,
            r#"{"\u0024serde_json::private::Number":"1"}"#,
        ] {
            assert!(from_str(text).unwrap().is_object());
        }
        assert!(
            from_str(r#"{"$serde_json::private::Number":1,"$serde_json::private::Number":2}"#)
                .is_err()
        );
    }

    #[test]
    fn depth_preflight_is_constant_space_and_respects_quotes_and_exact_limit() {
        assert!(check_container_depth(br#"{"text":"[[[\\\"{{{","x":[1e400]}"#, 2).is_ok());
        assert!(check_container_depth(b"[[0]]", 2).is_ok());
        assert!(check_container_depth(b"[[0]]", 1).is_err());
        assert!(check_container_depth(b"[[", 1).is_err());
        // EOF is intentionally retained for the canonical recovery parser.
        assert!(check_container_depth(b"[", 1).is_ok());
        let text = "[".repeat(100_000);
        assert!(check_container_depth(text.as_bytes(), 64).is_err());
    }

    #[test]
    fn typed_values_keep_last_wins_and_container_roots_have_independent_budgets() {
        let output: crate::ToolOutput =
            serde_json::from_str(r#"{"content":{"a":1,"a":1e400},"is_error":false}"#).unwrap();
        assert_eq!(output.content["a"].to_string(), "1e400");
        let deep = format!("{}1e400{}", "[".repeat(64), "]".repeat(64));
        let wire = format!("{{\"type\":\"json\",\"value\":{deep}}}");
        assert!(serde_json::from_str::<crate::ContentBlock>(&wire).is_ok());
        let too_deep = format!("{{\"type\":\"json\",\"value\":[{deep}]}}");
        assert!(serde_json::from_str::<crate::ContentBlock>(&too_deep).is_err());
        assert_eq!(
            serde_json::from_str::<crate::TurnEvent>(r#"{"type":"started","ignored":1}"#).unwrap(),
            crate::TurnEvent::Started
        );
    }
}
