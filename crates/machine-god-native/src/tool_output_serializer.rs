use machine_god_core::{CancellationToken, MAX_SAFE_JSON_DEPTH, ToolOutput};
use serde_json::Value;

const STRING_SCAN_CHUNK_BYTES: usize = 1_024;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompactToolOutputError {
    Cancelled,
    OutputLimit,
    JsonDepth,
    JsonNodes,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompactToolOutputLimits {
    pub(crate) output_bytes: usize,
    pub(crate) json_depth: usize,
    pub(crate) json_nodes: usize,
}

/// Serializes a tool output as compact JSON without recursive traversal.
///
/// The destination is cleared before serialization and may contain a bounded
/// prefix when an error is returned. Callers can retain it and reuse its
/// allocation across serializations.
pub(crate) fn serialize_tool_output_compact(
    output: &ToolOutput,
    destination: &mut Vec<u8>,
    limits: CompactToolOutputLimits,
    cancellation: &CancellationToken,
) -> Result<(), CompactToolOutputError> {
    destination.clear();
    validate_limits(limits)?;

    let mut encoder = Encoder::new(destination, limits.output_bytes, cancellation);
    encoder.append(b"{\"content\":")?;
    encode_value_iterative(
        &output.content,
        &mut encoder,
        limits.json_depth,
        limits.json_nodes,
    )?;
    encoder.append(b",\"is_error\":")?;
    encoder.append(if output.is_error { b"true" } else { b"false" })?;
    encoder.append(b"}")
}

/// Serializes one JSON value with the same compact, iterative encoder used for
/// tool outputs.
#[cfg(test)]
pub(crate) fn serialize_json_value_compact(
    value: &Value,
    destination: &mut Vec<u8>,
    limits: CompactToolOutputLimits,
    cancellation: &CancellationToken,
) -> Result<(), CompactToolOutputError> {
    destination.clear();
    validate_limits(limits)?;
    let mut encoder = Encoder::new(destination, limits.output_bytes, cancellation);
    encode_value_iterative(value, &mut encoder, limits.json_depth, limits.json_nodes)
}

/// Measures one compact JSON value without retaining its encoded bytes.
pub(crate) fn measure_json_value_compact(
    value: &Value,
    limits: CompactToolOutputLimits,
    cancellation: &CancellationToken,
) -> Result<usize, CompactToolOutputError> {
    validate_limits(limits)?;
    let mut encoder = Encoder::measuring(limits.output_bytes, cancellation);
    encode_value_iterative(value, &mut encoder, limits.json_depth, limits.json_nodes)?;
    Ok(encoder.written())
}

fn validate_limits(limits: CompactToolOutputLimits) -> Result<(), CompactToolOutputError> {
    if limits.json_depth > MAX_SAFE_JSON_DEPTH {
        Err(CompactToolOutputError::Invalid)
    } else {
        Ok(())
    }
}

enum EncodingOutput<'a> {
    Buffer(&'a mut Vec<u8>),
    Counter(usize),
}

struct Encoder<'a> {
    output: EncodingOutput<'a>,
    max_output_bytes: usize,
    cancellation: &'a CancellationToken,
    #[cfg(test)]
    scanned_string_bytes: usize,
    #[cfg(test)]
    cancel_after_scanned_bytes: Option<usize>,
}

impl<'a> Encoder<'a> {
    fn new(
        destination: &'a mut Vec<u8>,
        max_output_bytes: usize,
        cancellation: &'a CancellationToken,
    ) -> Self {
        Self {
            output: EncodingOutput::Buffer(destination),
            max_output_bytes,
            cancellation,
            #[cfg(test)]
            scanned_string_bytes: 0,
            #[cfg(test)]
            cancel_after_scanned_bytes: None,
        }
    }

    fn measuring(max_output_bytes: usize, cancellation: &'a CancellationToken) -> Self {
        Self {
            output: EncodingOutput::Counter(0),
            max_output_bytes,
            cancellation,
            #[cfg(test)]
            scanned_string_bytes: 0,
            #[cfg(test)]
            cancel_after_scanned_bytes: None,
        }
    }

    fn check_cancelled(&self) -> Result<(), CompactToolOutputError> {
        if self.cancellation.is_cancelled() {
            Err(CompactToolOutputError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn remaining(&self) -> usize {
        self.max_output_bytes.saturating_sub(self.written())
    }

    fn written(&self) -> usize {
        match &self.output {
            EncodingOutput::Buffer(destination) => destination.len(),
            EncodingOutput::Counter(bytes) => *bytes,
        }
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), CompactToolOutputError> {
        self.check_cancelled()?;
        if bytes.len() > self.remaining() {
            return Err(CompactToolOutputError::OutputLimit);
        }
        match &mut self.output {
            EncodingOutput::Buffer(destination) => {
                reserve_for_append(destination, bytes.len(), self.max_output_bytes)?;
                destination.extend_from_slice(bytes);
            }
            EncodingOutput::Counter(written) => {
                *written += bytes.len();
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn note_scanned_string_byte(&mut self) {
        self.scanned_string_bytes += 1;
        if self.cancel_after_scanned_bytes == Some(self.scanned_string_bytes) {
            self.cancellation.cancel();
        }
    }
}

fn reserve_for_append(
    destination: &mut Vec<u8>,
    additional: usize,
    max_output_bytes: usize,
) -> Result<(), CompactToolOutputError> {
    let required = destination
        .len()
        .checked_add(additional)
        .ok_or(CompactToolOutputError::OutputLimit)?;
    if required <= destination.capacity() {
        return Ok(());
    }

    let geometric = destination
        .capacity()
        .max(STRING_SCAN_CHUNK_BYTES)
        .saturating_mul(2)
        .min(max_output_bytes);
    let target = required.max(geometric);
    destination
        .try_reserve_exact(target - destination.len())
        .map_err(|_| CompactToolOutputError::Invalid)
}

enum ValueFrame<'a> {
    Array {
        values: std::slice::Iter<'a, Value>,
        depth: usize,
        first: bool,
    },
    Object {
        values: serde_json::map::Iter<'a>,
        depth: usize,
        first: bool,
    },
}

enum FrameAction<'a> {
    ArrayValue {
        value: &'a Value,
        depth: usize,
        comma: bool,
    },
    ObjectValue {
        key: &'a str,
        value: &'a Value,
        depth: usize,
        comma: bool,
    },
    CloseArray,
    CloseObject,
    Done,
}

fn encode_value_iterative(
    root: &Value,
    encoder: &mut Encoder<'_>,
    max_depth: usize,
    max_nodes: usize,
) -> Result<(), CompactToolOutputError> {
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(max_depth)
        .map_err(|_| CompactToolOutputError::Invalid)?;
    let mut current = Some((root, 0_usize));
    let mut nodes = 0_usize;

    loop {
        if let Some((value, depth)) = current.take() {
            encoder.check_cancelled()?;
            nodes = nodes
                .checked_add(1)
                .filter(|nodes| *nodes <= max_nodes)
                .ok_or(CompactToolOutputError::JsonNodes)?;
            encode_node(value, depth, max_depth, encoder, &mut frames)?;
            continue;
        }

        let action = next_frame_action(&mut frames);

        match action {
            FrameAction::ArrayValue {
                value,
                depth,
                comma,
            } => {
                if comma {
                    encoder.append(b",")?;
                }
                current = Some((value, depth));
            }
            FrameAction::ObjectValue {
                key,
                value,
                depth,
                comma,
            } => {
                if comma {
                    encoder.append(b",")?;
                }
                encode_string(key, encoder)?;
                encoder.append(b":")?;
                current = Some((value, depth));
            }
            FrameAction::CloseArray => {
                frames.pop();
                encoder.append(b"]")?;
            }
            FrameAction::CloseObject => {
                frames.pop();
                encoder.append(b"}")?;
            }
            FrameAction::Done => return Ok(()),
        }
    }
}

fn encode_node<'a>(
    value: &'a Value,
    depth: usize,
    max_depth: usize,
    encoder: &mut Encoder<'_>,
    frames: &mut Vec<ValueFrame<'a>>,
) -> Result<(), CompactToolOutputError> {
    match value {
        Value::Null => encoder.append(b"null"),
        Value::Bool(value) => encoder.append(if *value { b"true" } else { b"false" }),
        Value::Number(value) => encode_number(value, encoder),
        Value::String(value) => encode_string(value, encoder),
        Value::Array(values) => {
            if depth >= max_depth {
                return Err(CompactToolOutputError::JsonDepth);
            }
            encoder.append(b"[")?;
            frames.push(ValueFrame::Array {
                values: values.iter(),
                depth: depth + 1,
                first: true,
            });
            Ok(())
        }
        Value::Object(values) => {
            if depth >= max_depth {
                return Err(CompactToolOutputError::JsonDepth);
            }
            encoder.append(b"{")?;
            frames.push(ValueFrame::Object {
                values: values.iter(),
                depth: depth + 1,
                first: true,
            });
            Ok(())
        }
    }
}

fn encode_number(
    value: &serde_json::Number,
    encoder: &mut Encoder<'_>,
) -> Result<(), CompactToolOutputError> {
    let mut writer = NumberWriter {
        encoder,
        error: None,
    };
    if std::fmt::write(&mut writer, format_args!("{value}")).is_err() {
        return Err(writer.error.unwrap_or(CompactToolOutputError::Invalid));
    }
    Ok(())
}

struct NumberWriter<'a, 'output> {
    encoder: &'a mut Encoder<'output>,
    error: Option<CompactToolOutputError>,
}

impl std::fmt::Write for NumberWriter<'_, '_> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        match self.encoder.append(value.as_bytes()) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.error = Some(error);
                Err(std::fmt::Error)
            }
        }
    }
}

fn next_frame_action<'a>(frames: &mut [ValueFrame<'a>]) -> FrameAction<'a> {
    match frames.last_mut() {
        Some(ValueFrame::Array {
            values,
            depth,
            first,
        }) => match values.next() {
            Some(value) => {
                let comma = !*first;
                *first = false;
                FrameAction::ArrayValue {
                    value,
                    depth: *depth,
                    comma,
                }
            }
            None => FrameAction::CloseArray,
        },
        Some(ValueFrame::Object {
            values,
            depth,
            first,
        }) => match values.next() {
            Some((key, value)) => {
                let comma = !*first;
                *first = false;
                FrameAction::ObjectValue {
                    key,
                    value,
                    depth: *depth,
                    comma,
                }
            }
            None => FrameAction::CloseObject,
        },
        None => FrameAction::Done,
    }
}

fn encode_string(value: &str, encoder: &mut Encoder<'_>) -> Result<(), CompactToolOutputError> {
    encoder.append(b"\"")?;
    let bytes = value.as_bytes();
    let mut chunk_start = 0_usize;
    while chunk_start < bytes.len() {
        encoder.check_cancelled()?;
        let chunk_end = chunk_start
            .checked_add(STRING_SCAN_CHUNK_BYTES)
            .unwrap_or(bytes.len())
            .min(bytes.len());
        let chunk = &bytes[chunk_start..chunk_end];
        if chunk.len() > encoder.remaining() {
            return Err(CompactToolOutputError::OutputLimit);
        }

        let mut run_start = 0_usize;
        for (index, byte) in chunk.iter().copied().enumerate() {
            #[cfg(test)]
            encoder.note_scanned_string_byte();
            let escape = match byte {
                b'"' => Some(&b"\\\""[..]),
                b'\\' => Some(&b"\\\\"[..]),
                b'\x08' => Some(&b"\\b"[..]),
                b'\t' => Some(&b"\\t"[..]),
                b'\n' => Some(&b"\\n"[..]),
                b'\x0c' => Some(&b"\\f"[..]),
                b'\r' => Some(&b"\\r"[..]),
                0x00..=0x1f => None,
                _ => continue,
            };
            encoder.append(&chunk[run_start..index])?;
            if byte <= 0x1f && escape.is_none() {
                let unicode_escape = [
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX_DIGITS[usize::from(byte >> 4)],
                    HEX_DIGITS[usize::from(byte & 0x0f)],
                ];
                encoder.append(&unicode_escape)?;
            } else {
                encoder.append(escape.expect("non-control escape classified"))?;
            }
            run_start = index + 1;
        }
        encoder.append(&chunk[run_start..])?;
        chunk_start = chunk_end;
    }
    encoder.append(b"\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Number, json};

    fn limits(max_output_bytes: usize) -> CompactToolOutputLimits {
        CompactToolOutputLimits {
            output_bytes: max_output_bytes,
            json_depth: MAX_SAFE_JSON_DEPTH,
            json_nodes: 10_000,
        }
    }

    #[test]
    fn compact_output_matches_serde_json_for_values_numbers_and_escapes() {
        let controls: String = (0_u8..=0x1f).map(char::from).collect();
        let mut object = Map::new();
        object.insert("z-key".to_owned(), Value::Null);
        object.insert(
            format!("key-{controls}-\"-\\-λ"),
            Value::String(format!("{controls}\"\\/λ🦀")),
        );
        object.insert(
            "numbers".to_owned(),
            Value::Array(vec![
                Value::Number(Number::from(i64::MIN)),
                Value::Number(Number::from(u64::MAX)),
                Value::Number(Number::from_f64(-0.0).unwrap()),
                Value::Number(Number::from_f64(1.25e100).unwrap()),
            ]),
        );
        object.insert("nested".to_owned(), json!([true, false, {"array": []}]));
        for is_error in [false, true] {
            let output = ToolOutput {
                content: Value::Object(object.clone()),
                is_error,
            };
            let expected = serde_json::to_vec(&output).unwrap();
            let mut actual = Vec::new();
            serialize_tool_output_compact(
                &output,
                &mut actual,
                limits(expected.len()),
                &CancellationToken::new(),
            )
            .unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn compact_value_serialization_and_measurement_match_serde_json() {
        let value = json!({
            "controls": "\u{0}\u{8}\t\n\r\"\\/",
            "unicode": "λ🦀",
            "numbers": [i64::MIN, u64::MAX, -0.0, 1.25e100],
            "nested": [{"empty": []}, true, null],
        });
        let expected = serde_json::to_vec(&value).unwrap();
        let limits = CompactToolOutputLimits {
            output_bytes: expected.len(),
            json_depth: MAX_SAFE_JSON_DEPTH,
            json_nodes: 100,
        };
        let cancellation = CancellationToken::new();
        let mut actual = Vec::new();
        serialize_json_value_compact(&value, &mut actual, limits, &cancellation).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            measure_json_value_compact(&value, limits, &cancellation).unwrap(),
            expected.len()
        );

        let short = CompactToolOutputLimits {
            output_bytes: expected.len() - 1,
            ..limits
        };
        assert_eq!(
            serialize_json_value_compact(&value, &mut actual, short, &cancellation),
            Err(CompactToolOutputError::OutputLimit)
        );
        assert_eq!(
            measure_json_value_compact(&value, short, &cancellation),
            Err(CompactToolOutputError::OutputLimit)
        );

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert_eq!(
            serialize_json_value_compact(&value, &mut actual, limits, &cancelled),
            Err(CompactToolOutputError::Cancelled)
        );
        assert_eq!(
            measure_json_value_compact(&value, limits, &cancelled),
            Err(CompactToolOutputError::Cancelled)
        );
    }

    fn measure_output_allocations(
        output: &ToolOutput,
        limits: CompactToolOutputLimits,
    ) -> (Vec<u8>, allocation_counter::AllocationInfo) {
        let cancellation = CancellationToken::new();
        let mut actual = Vec::new();
        let mut result = None;
        let allocations = allocation_counter::measure(|| {
            result = Some(serialize_tool_output_compact(
                output,
                &mut actual,
                limits,
                &cancellation,
            ));
        });
        assert_eq!(result, Some(Ok(())));
        (actual, allocations)
    }

    #[test]
    fn multi_megabyte_strings_use_amortized_fallible_growth() {
        for output in [
            ToolOutput::success("x".repeat(2 * 1024 * 1024)),
            ToolOutput::success("\"".repeat(1024 * 1024)),
        ] {
            let expected = serde_json::to_vec(&output).unwrap();
            let (actual, allocations) = measure_output_allocations(
                &output,
                CompactToolOutputLimits {
                    output_bytes: expected.len(),
                    json_depth: MAX_SAFE_JSON_DEPTH,
                    json_nodes: 1,
                },
            );
            assert_eq!(actual, expected);
            assert!(
                allocations.count_total <= 32,
                "string encoding allocated too often: {allocations:?}"
            );
        }
    }

    #[test]
    fn number_heavy_output_does_not_allocate_per_node() {
        let output = ToolOutput::success(Value::Array(
            (0_u64..100_000)
                .map(Number::from)
                .map(Value::Number)
                .collect(),
        ));
        let expected = serde_json::to_vec(&output).unwrap();
        let (actual, allocations) = measure_output_allocations(
            &output,
            CompactToolOutputLimits {
                output_bytes: expected.len(),
                json_depth: MAX_SAFE_JSON_DEPTH,
                json_nodes: 100_001,
            },
        );
        assert_eq!(actual, expected);
        assert!(
            allocations.count_total <= 32,
            "number encoding allocated per node: {allocations:?}"
        );
    }

    #[test]
    fn output_limit_stops_before_scanning_a_huge_unescaped_string() {
        let cancellation = CancellationToken::new();
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new(&mut bytes, 8, &cancellation);
        let error = encode_string(&"x".repeat(4 * 1024 * 1024), &mut encoder).unwrap_err();
        assert_eq!(error, CompactToolOutputError::OutputLimit);
        assert_eq!(encoder.scanned_string_bytes, 0);
        assert!(bytes.len() <= 8);
    }

    #[test]
    fn depth_and_node_limits_are_independent() {
        let output = ToolOutput::success(json!([[[0]]]));
        let cancellation = CancellationToken::new();
        let mut bytes = Vec::new();
        assert_eq!(
            serialize_tool_output_compact(
                &output,
                &mut bytes,
                CompactToolOutputLimits {
                    output_bytes: 1_024,
                    json_depth: 2,
                    json_nodes: 100,
                },
                &cancellation,
            ),
            Err(CompactToolOutputError::JsonDepth)
        );
        assert_eq!(
            serialize_tool_output_compact(
                &output,
                &mut bytes,
                CompactToolOutputLimits {
                    output_bytes: 1_024,
                    json_depth: MAX_SAFE_JSON_DEPTH,
                    json_nodes: 3,
                },
                &cancellation,
            ),
            Err(CompactToolOutputError::JsonNodes)
        );
    }

    #[test]
    fn string_scans_observe_cancellation_at_a_bounded_interval() {
        let cancellation = CancellationToken::new();
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new(&mut bytes, usize::MAX, &cancellation);
        encoder.cancel_after_scanned_bytes = Some(17);
        let error =
            encode_string(&"x".repeat(4 * STRING_SCAN_CHUNK_BYTES), &mut encoder).unwrap_err();
        assert_eq!(error, CompactToolOutputError::Cancelled);
        assert_eq!(encoder.scanned_string_bytes, STRING_SCAN_CHUNK_BYTES);
    }

    #[test]
    fn cancelled_and_invalid_configuration_are_distinct() {
        let output = ToolOutput::success("content");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut bytes = Vec::new();
        assert_eq!(
            serialize_tool_output_compact(&output, &mut bytes, limits(1_024), &cancellation),
            Err(CompactToolOutputError::Cancelled)
        );
        assert_eq!(
            serialize_tool_output_compact(
                &output,
                &mut bytes,
                CompactToolOutputLimits {
                    output_bytes: 1_024,
                    json_depth: MAX_SAFE_JSON_DEPTH + 1,
                    json_nodes: 100,
                },
                &CancellationToken::new(),
            ),
            Err(CompactToolOutputError::Invalid)
        );
    }
}
