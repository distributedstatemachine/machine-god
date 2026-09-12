use super::{
    Action, Envelope, Error, McpClientMetadata, McpFeatureExchange, Params, RawValue, Result,
};
use crate::mcp::mrtr::{McpInputRequired, McpValidatedResponses};

pub(super) struct Continuation<'a> {
    pub responses: &'a RawValue,
    pub state: Option<&'a RawValue>,
}

impl McpFeatureExchange {
    pub(crate) fn retained_byte_charge(&self) -> usize {
        self.base_retained_bytes + self.wire.get().len() * 2
    }

    // Data serialization only. The private peer round supplies the correlated
    // input, exact original authority and its own newly allocated ID.
    pub(crate) fn continue_with(
        mut self,
        id: i64,
        input: &McpInputRequired,
        responses: &McpValidatedResponses,
    ) -> Result<Self> {
        if id <= self.options.id
            || !matches!(
                self.request.action(),
                Action::ResourceRead | Action::PromptGet
            )
        {
            return Err(Error::InvalidRequest);
        }
        let responses = input
            .validate_responses(responses.raw_json())
            .map_err(|_| Error::InvalidRequest)?;
        let params = Params {
            request: &self.request,
            cursor: None,
            metadata: McpClientMetadata::for_protocol(
                self.options.protocol.version,
                self.options.progress,
                self.options.form,
                self.options.url,
            ),
            continuation: Some(Continuation {
                responses: responses.wire_json(),
                state: input.request_state_json(),
            }),
        };
        let wire = serde_json::to_string(&Envelope {
            jsonrpc: "2.0",
            id,
            method: self.method(),
            params,
        })
        .map_err(|_| Error::InvalidRequest)?;
        if wire.len() > 128 * 1024
            || wire
                .len()
                .checked_mul(2)
                .and_then(|bytes| bytes.checked_add(self.base_retained_bytes))
                .is_none_or(|bytes| bytes > self.limits.max_retained_bytes)
        {
            return Err(Error::Limit);
        }
        self.wire = RawValue::from_string(wire).map_err(|_| Error::InvalidRequest)?;
        self.options.id = id;
        Ok(self)
    }
}
