use super::{CONTENT_BYTES, OUTPUT_NODES};
use crate::{
    mcp::{
        catalog::McpDescriptor, control::McpFeatureReply, feature::McpFeatureOutcome,
        pagination::McpCatalogKind,
    },
    mcp_features::{McpFeaturePublication, check_cancellation, trust_envelope},
    tool_output_serializer::{
        CompactToolOutputError, CompactToolOutputLimits, measure_json_value_compact,
    },
};
use machine_god_core::{CancellationToken, ToolError, ToolErrorKind, ToolOutput, json};
use serde_json::{Map, Value, value::RawValue};

mod count;

const LIMITS: CompactToolOutputLimits = CompactToolOutputLimits {
    output_bytes: CONTENT_BYTES,
    json_depth: 64,
    json_nodes: OUTPUT_NODES,
};

pub(super) fn project(
    publication: &McpFeaturePublication,
    reply: &McpFeatureReply,
    cancellation: &CancellationToken,
) -> Result<(ToolOutput, bool), ToolError> {
    check_cancellation(cancellation)?;
    let mut envelope = trust_envelope(publication);
    if let Some(identity) = publication.identity() {
        envelope.insert("identity".to_owned(), Value::String(identity.to_owned()));
    }
    if let Some(argument) = publication.argument() {
        envelope.insert("argument".to_owned(), Value::String(argument.to_owned()));
    }
    let mut untrusted = Map::new();
    match reply {
        McpFeatureReply::Catalog(catalog) => {
            untrusted.insert(
                "catalog_kind".to_owned(),
                Value::String(
                    match catalog.kind() {
                        McpCatalogKind::Tools => "tools",
                        McpCatalogKind::Resources => "resources",
                        McpCatalogKind::ResourceTemplates => "resource_templates",
                        McpCatalogKind::Prompts => "prompts",
                    }
                    .to_owned(),
                ),
            );
            untrusted.insert("items".to_owned(), Value::Array(Vec::new()));
            envelope.insert("untrusted".to_owned(), Value::Object(untrusted));
            let mut content = Value::Object(envelope);
            // Admission already bounds aggregate source nodes. Count raw bytes
            // and check added nesting BEFORE parsing/copying any descriptors.
            let raw = catalog.descriptors().iter().map(descriptor_raw);
            preflight(&content, raw, 3, cancellation)?;
            let items = catalog
                .descriptors()
                .iter()
                .map(|item| {
                    check_cancellation(cancellation)?;
                    json::from_str(descriptor_raw(item).get()).map_err(|_| limit())
                })
                .collect::<Result<Vec<_>, ToolError>>()?;
            content["untrusted"]["items"] = Value::Array(items);
            finish(content, false, false, cancellation)
        }
        McpFeatureReply::Response(response) => {
            let stop = matches!(
                response.outcome(),
                McpFeatureOutcome::UnvalidatedInputRequired
            );
            if stop {
                envelope.insert(
                    "stop".to_owned(),
                    Value::String("McpInputRequired".to_owned()),
                );
            }
            untrusted.insert("response".to_owned(), Value::Null);
            envelope.insert("untrusted".to_owned(), Value::Object(untrusted));
            let mut content = Value::Object(envelope);
            preflight(
                &content,
                std::iter::once(response.raw_json()),
                2,
                cancellation,
            )?;
            content["untrusted"]["response"] =
                json::from_str(response.raw_json().get()).map_err(|_| limit())?;
            let error = stop
                || matches!(
                    response.outcome(),
                    McpFeatureOutcome::ProtocolFailure { .. }
                );
            finish(content, error, stop, cancellation)
        }
    }
}

fn descriptor_raw(descriptor: &McpDescriptor) -> &RawValue {
    match descriptor {
        McpDescriptor::Tool(value) => value.raw_json(),
        McpDescriptor::Resource(value) => value.raw_json(),
        McpDescriptor::ResourceTemplate(value) => value.raw_json(),
        McpDescriptor::Prompt(value) => value.raw_json(),
    }
}

fn preflight<'a>(
    envelope: &Value,
    raws: impl Iterator<Item = &'a RawValue>,
    nesting: usize,
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let mut bytes = measure(envelope, cancellation)?;
    // The fixed trusted envelope uses fewer than 64 nodes, leaving the full
    // admitted wire budget for all raw values together, not for each item.
    let mut remaining_nodes = OUTPUT_NODES - 64;
    for raw in raws {
        check_cancellation(cancellation)?;
        // Canonical serialization cannot exceed admitted raw JSON bytes; the
        // extra comma and retained placeholders make this conservative.
        bytes = bytes
            .checked_add(raw.get().len())
            .and_then(|n| n.checked_add(1))
            .ok_or_else(limit)?;
        if bytes > CONTENT_BYTES {
            return Err(limit());
        }
        json::check_container_depth(raw.get().as_bytes(), 64 - nesting).map_err(|_| limit())?;
        count::charge(raw, &mut remaining_nodes, cancellation)?;
    }
    Ok(())
}

fn finish(
    content: Value,
    is_error: bool,
    stop: bool,
    cancellation: &CancellationToken,
) -> Result<(ToolOutput, bool), ToolError> {
    measure(&content, cancellation)?;
    check_cancellation(cancellation)?;
    Ok((ToolOutput { content, is_error }, stop))
}

pub(super) fn limit() -> ToolError {
    ToolError::new(
        ToolErrorKind::InvalidInput,
        "mcp_features_native_projection_limit",
        "The MCP feature result exceeds its projection bounds",
        false,
    )
}

fn measure(content: &Value, cancellation: &CancellationToken) -> Result<usize, ToolError> {
    measure_json_value_compact(content, LIMITS, cancellation).map_err(|error| match error {
        CompactToolOutputError::Cancelled => crate::mcp_features::cancelled(),
        _ => limit(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(value: String) -> Box<RawValue> {
        RawValue::from_string(value).unwrap()
    }

    #[test]
    fn preflight_charges_nodes_across_descriptors_before_copying() {
        let item = raw(format!("[{}0]", "0,".repeat(131_071)));
        let cancel = CancellationToken::new();
        assert!(preflight(&Value::Null, std::iter::once(item.as_ref()), 3, &cancel).is_ok());
        assert!(
            preflight(
                &Value::Null,
                [item.as_ref(), item.as_ref()].into_iter(),
                3,
                &cancel
            )
            .is_err()
        );
    }

    #[test]
    fn preflight_checks_added_depth_and_escaped_trusted_identity() {
        let nested = raw(format!("{}0{}", "[".repeat(62), "]".repeat(62)));
        let cancel = CancellationToken::new();
        assert!(preflight(&Value::Null, std::iter::once(nested.as_ref()), 2, &cancel).is_ok());
        assert!(preflight(&Value::Null, std::iter::once(nested.as_ref()), 3, &cancel).is_err());
        let escaped = Value::String("\0".repeat(CONTENT_BYTES / 6));
        assert!(preflight(&escaped, std::iter::empty(), 2, &cancel).is_err());
    }

    #[test]
    fn source_bytes_bound_compact_strings_and_exact_numbers() {
        let source =
            raw(r#"{"text":"\u0000\b\n\/\u00e9😀","number":-0,"exponent":1E+400}"#.to_owned());
        let decoded = json::from_str(source.get()).unwrap();
        let measured =
            measure_json_value_compact(&decoded, LIMITS, &CancellationToken::new()).unwrap();
        assert!(measured <= source.get().len());
        let mut nodes = 4;
        count::charge(&source, &mut nodes, &CancellationToken::new()).unwrap();
        assert_eq!(nodes, 0);
    }

    #[test]
    fn private_looking_keys_are_objects_but_exact_numbers_are_single_nodes() {
        let source = raw(
            r#"{"$serde_json::private::Number":1e400,"$serde_json::private::RawValue":{"n":-0}}"#
                .to_owned(),
        );
        let mut nodes = 4;
        count::charge(&source, &mut nodes, &CancellationToken::new()).unwrap();
        assert_eq!(nodes, 0);
        assert!(count::charge(&source, &mut 3, &CancellationToken::new()).is_err());
    }
}
