use super::{
    Error, McpToolInputRequired, McpToolProtocolFailure, McpToolResponseContext,
    McpToolResponseDisposition, NativeMcpToolResultAdmission, Result,
};
use crate::mcp::{
    catalog::fields,
    feature::{McpFeatureCodecError, content},
    mrtr::{McpInputRequired, McpMrtrLimits},
    protocol::{ProtocolVersion, RpcKind, WireLimits, parse_envelope},
    schema::McpSchemaValidation,
};
use machine_god_core::ToolOutput;
use serde_json::value::RawValue;

impl NativeMcpToolResultAdmission {
    /// Validates exact correlation and the complete tools/call result before
    /// exposing any content. Malformed input-required results never become success.
    /// # Errors
    /// Rejects malformed/foreign data, invalid output schema instances and limits.
    pub fn admit(
        &self,
        context: McpToolResponseContext,
        bytes: &[u8],
    ) -> Result<McpToolResponseDisposition> {
        if bytes
            .len()
            .checked_mul(4)
            .is_none_or(|n| n > self.limits.max_retained_bytes)
        {
            return Err(Error::Limit);
        }
        // Bounds precede all raw-span maps and copies. Retain the exact decoded
        // result for complete output instead of allocating a second full Value.
        let (kind, value) = {
            let envelope = parse_envelope(
                bytes,
                WireLimits {
                    max_frame_bytes: self.limits.max_response_bytes,
                    max_depth: 33,
                    max_nodes: self.limits.max_nodes,
                },
            )
            .map_err(|_| Error::InvalidResponse)?;
            envelope
                .correlate(context.request_id(), false)
                .map_err(|_| Error::Correlation)?;
            (envelope.kind(), envelope.into_value())
        };
        let raw: &RawValue = serde_json::from_slice(bytes).map_err(|_| Error::InvalidResponse)?;
        let envelope = fields::object(raw).map_err(|_| Error::InvalidResponse)?;
        let result = *envelope
            .get(if kind == RpcKind::Error {
                "error"
            } else {
                "result"
            })
            .ok_or(Error::InvalidResponse)?;
        let object = fields::object(result).map_err(|_| Error::InvalidResponse)?;
        if kind == RpcKind::Error {
            return self.protocol_failure(result, &object);
        }
        match fields::optional(&object, "resultType", 32)
            .map_err(|_| Error::InvalidResponse)?
            .as_deref()
        {
            Some("input_required") => {
                if context.protocol().version != ProtocolVersion::Modern {
                    return Err(Error::UnsupportedResultType);
                }
                let required = McpInputRequired::parse(result, McpMrtrLimits::default())
                    .map_err(|_| Error::InvalidInputRequired)?;
                Ok(McpToolResponseDisposition::InputRequired(Box::new(
                    McpToolInputRequired { context, required },
                )))
            }
            None | Some("complete") => {
                let serde_json::Value::Object(mut envelope) = value else {
                    return Err(Error::InvalidResponse);
                };
                let value = envelope.remove("result").ok_or(Error::InvalidResponse)?;
                drop(envelope);
                self.complete(&context, &object, value)
            }
            Some(_) => Err(Error::UnsupportedResultType),
        }
    }

    fn complete(
        &self,
        context: &McpToolResponseContext,
        object: &fields::Object<'_>,
        value: serde_json::Value,
    ) -> Result<McpToolResponseDisposition> {
        content::compact_value_size(&value, self.limits.max_result_bytes).map_err(content_error)?;
        let items = fields::array(object.get("content").ok_or(Error::InvalidContent)?)
            .map_err(|_| Error::InvalidContent)?;
        if items.len() > self.limits.max_content_items {
            return Err(Error::Limit);
        }
        let mut total = 0;
        for item in items {
            content::admit_with_policy(
                item,
                self.limits.content_limits(),
                &mut total,
                content::Policy::Tool,
            )
            .map_err(content_error)?;
        }
        let is_error = object
            .get("isError")
            .map(|raw| fields::boolean(raw).map_err(|_| Error::InvalidResponse))
            .transpose()?
            .unwrap_or(false);
        if let Some(schema) = context.descriptor().output_schema() {
            let structured = object
                .get("structuredContent")
                .ok_or(Error::InvalidStructuredContent)?;
            match schema
                .validate_json(structured.get().as_bytes())
                .map_err(|_| Error::InvalidStructuredContent)?
            {
                McpSchemaValidation::Valid | McpSchemaValidation::ServerAuthoritative => {}
                McpSchemaValidation::Invalid(_) => return Err(Error::InvalidStructuredContent),
            }
        }
        Ok(McpToolResponseDisposition::Complete(ToolOutput {
            content: value,
            is_error,
        }))
    }

    fn protocol_failure(
        &self,
        raw: &RawValue,
        object: &fields::Object<'_>,
    ) -> Result<McpToolResponseDisposition> {
        content::compact_size(raw, self.limits.max_result_bytes).map_err(content_error)?;
        let code: i64 =
            serde_json::from_str(object.get("code").ok_or(Error::InvalidResponse)?.get())
                .map_err(|_| Error::InvalidResponse)?;
        let message = fields::optional(object, "message", 64 * 1024)
            .map_err(|_| Error::InvalidResponse)?
            .ok_or(Error::InvalidResponse)?;
        if let Some(data) = object.get("data") {
            content::compact_size(data, 128 * 1024).map_err(content_error)?;
        }
        Ok(McpToolResponseDisposition::ProtocolFailure(
            McpToolProtocolFailure {
                code,
                message,
                raw: raw.to_owned(),
            },
        ))
    }
}

fn content_error(error: McpFeatureCodecError) -> Error {
    match error {
        McpFeatureCodecError::Limit => Error::Limit,
        _ => Error::InvalidContent,
    }
}
