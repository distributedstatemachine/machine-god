//! Private, non-clone custody of one correlated feature response.
use super::{
    McpFeatureCodecError as Error, McpFeatureControlAuthority, McpFeatureOperationOptions,
    McpFeatureReply,
};
use crate::mcp::{
    feature::{McpFeatureExchange, McpFeatureOutcome},
    mrtr::{McpInputRequired, McpMrtrLimits, McpValidatedResponses},
};
use std::{fmt, sync::Arc};

pub(crate) struct McpFeatureRound {
    reply: McpFeatureReply,
    pending: Option<Pending>,
}
struct Pending {
    exchange: McpFeatureExchange,
    authority: McpFeatureControlAuthority,
    options: McpFeatureOperationOptions,
    peer: Arc<()>,
    input: Arc<McpInputRequired>,
}
impl fmt::Debug for McpFeatureRound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpFeatureRound { <redacted> }")
    }
}
impl McpFeatureRound {
    pub(crate) fn new(
        reply: McpFeatureReply,
        exchange: McpFeatureExchange,
        authority: McpFeatureControlAuthority,
        options: McpFeatureOperationOptions,
        peer: Arc<()>,
    ) -> Result<Self, Error> {
        if !authority.is_live() {
            return Err(Error::Closed);
        }
        let pending = if let McpFeatureReply::Response(response) = &reply
            && matches!(
                response.outcome(),
                McpFeatureOutcome::UnvalidatedInputRequired
            ) {
            let defaults = McpMrtrLimits::default();
            let input = McpInputRequired::parse(
                response.result_json(),
                McpMrtrLimits {
                    max_json_bytes: defaults
                        .max_json_bytes
                        .min(options.codec.max_response_bytes),
                    max_nodes: defaults.max_nodes.min(options.codec.max_nodes),
                    max_retained_bytes: defaults
                        .max_retained_bytes
                        .min(options.codec.max_retained_bytes),
                    ..defaults
                },
            )
            .map_err(|_| Error::InvalidResponse)?;
            let charge = response
                .raw_json()
                .get()
                .len()
                .checked_mul(5)
                .and_then(|bytes| bytes.checked_add(exchange.retained_byte_charge()))
                .and_then(|bytes| bytes.checked_add(input.retained_byte_charge()))
                .and_then(|bytes| bytes.checked_add(1024))
                .ok_or(Error::Limit)?;
            if charge > options.codec.max_retained_bytes {
                return Err(Error::Limit);
            }
            Some(Pending {
                exchange,
                authority,
                options,
                peer,
                input: Arc::new(input),
            })
        } else {
            None
        };
        Ok(Self { reply, pending })
    }

    pub(crate) fn reply(&self) -> &McpFeatureReply {
        &self.reply
    }
    pub(crate) fn input(&self) -> Option<&Arc<McpInputRequired>> {
        self.pending.as_ref().map(|pending| &pending.input)
    }
    pub(crate) fn into_reply(self) -> McpFeatureReply {
        self.reply
    }
    pub(crate) fn check_peer(&self, peer: &Arc<()>) -> Result<(), Error> {
        let pending = self.pending.as_ref().ok_or(Error::Unsupported)?;
        if !Arc::ptr_eq(&pending.peer, peer) {
            return Err(Error::InvalidRequest);
        }
        if !pending.authority.is_live() {
            return Err(Error::Closed);
        }
        Ok(())
    }
    pub(crate) fn resume(
        self,
        peer: &Arc<()>,
        id: i64,
        responses: McpValidatedResponses,
    ) -> Result<
        (
            McpFeatureExchange,
            McpFeatureControlAuthority,
            McpFeatureOperationOptions,
        ),
        Error,
    > {
        self.check_peer(peer)?;
        let pending = self.pending.ok_or(Error::Unsupported)?;
        let exchange = pending
            .exchange
            .continue_with(id, &pending.input, &responses)?;
        Ok((exchange, pending.authority, pending.options))
    }
}
