//! Private original-grant retention. Only native sealed-response/consent routing
//! can reach this transition; raw preparation and the claimed tombstone do not reopen.

use super::{
    Data, Framing, McpSubmission, McpSubmissionError, McpSubmissionRegistry, McpToolReservation,
    Ready, Reservation, Result, RpcId, Slot, bounded_json, check,
};
use crate::mcp::{mrtr::McpValidatedResponses, protocol::McpClientMetadata};
use machine_god_core::{BoxFuture, CancellationToken};
use serde::Serialize;
use serde_json::value::RawValue;
use std::sync::{Arc, atomic::AtomicBool};

pub(super) const MAX_CONTINUATION_BYTES: usize = 384 * 1024;

pub(crate) struct McpContinuationCustody {
    original: Arc<Ready>,
    registry: Arc<McpSubmissionRegistry>,
    cancellation: CancellationToken,
}
impl McpSubmission {
    pub(crate) fn continuation_custody(&self) -> Result<McpContinuationCustody> {
        self.checkpoint()?;
        if self.ready.data.tool_options.is_none() || self.ready.data.tool_reservation.is_none() {
            return Err(McpSubmissionError::Denied);
        }
        Ok(McpContinuationCustody {
            original: self.ready.clone(),
            registry: self.registry.clone(),
            cancellation: self.cancellation.clone(),
        })
    }
    pub(crate) fn write_completion(&self) -> Arc<AtomicBool> {
        self.written.clone()
    }
}
impl McpContinuationCustody {
    pub(crate) fn cancelled_owned(&self) -> BoxFuture<'static, ()> {
        let preparation = self.original.data.cancellation.cancelled();
        let execution = self.cancellation.cancelled();
        let registry = self.registry.cancelled_owned();
        let runtime = self.original.data.runtime.cancelled_owned();
        let proof = self.original.proof.clone();
        Box::pin(async move {
            futures_util::future::select(
                Box::pin(async {
                    futures_util::future::select(preparation, execution).await;
                }),
                Box::pin(async {
                    futures_util::future::select(
                        Box::pin(registry),
                        Box::pin(async {
                            futures_util::future::select(Box::pin(runtime), proof.invalidated())
                                .await;
                        }),
                    )
                    .await;
                }),
            )
            .await;
        })
    }
    pub(crate) fn revalidate(&self) -> Result<()> {
        check(&self.cancellation)?;
        check(&self.original.data.cancellation)?;
        self.registry.live()?;
        self.original.data.runtime.live()?;
        self.original
            .proof
            .revalidate()
            .map_err(|_| McpSubmissionError::Denied)?;
        let state = self
            .registry
            .state
            .lock()
            .map_err(|_| McpSubmissionError::Unavailable)?;
        if !matches!(state.slots.get(&self.original.data.reservation.call), Some(Slot::Claimed(id)) if id == &self.original.data.permission.id)
        {
            return Err(McpSubmissionError::Denied);
        }
        Ok(())
    }

    pub(crate) fn prepare(
        &self,
        reservation: McpToolReservation,
        responses: &McpValidatedResponses,
        state: Option<&RawValue>,
    ) -> Result<McpSubmission> {
        self.revalidate()?;
        let RpcId::Integer(id) = *reservation.rpc_id() else {
            return Err(McpSubmissionError::Invalid);
        };
        let original = &self.original.data;
        let options = original
            .tool_options
            .ok_or(McpSubmissionError::Denied)?
            .renewed(id)?;
        let arguments: &RawValue =
            serde_json::from_slice(&original.arguments).map_err(|_| McpSubmissionError::Invalid)?;
        let payload = bounded_json(
            &Envelope {
                jsonrpc: "2.0",
                id,
                method: "tools/call",
                params: Params {
                    name: original.runtime.binding.remote_tool(),
                    arguments,
                    metadata: options.metadata(),
                    responses: responses.wire_json(),
                    state,
                },
            },
            MAX_CONTINUATION_BYTES,
        )?;
        let wire = match original.framing {
            Framing::Ndjson => {
                let mut wire = payload.into_vec();
                wire.push(b'\n');
                wire.into_boxed_slice()
            }
            Framing::Http => {
                #[cfg(any(test, feature = "mcp-http"))]
                {
                    original
                        .http_head
                        .as_ref()
                        .ok_or(McpSubmissionError::Denied)?
                        .encode_continuation(&payload)?
                }
                #[cfg(not(any(test, feature = "mcp-http")))]
                {
                    return Err(McpSubmissionError::Denied);
                }
            }
        };
        let submission = McpSubmission {
            registry: self.registry.clone(),
            ready: Arc::new(Ready {
                proof: self.original.proof.clone(),
                data: Data {
                    permission: original.permission.clone(),
                    tool: original.tool.clone(),
                    arguments: original.arguments.clone(),
                    wire,
                    framing: original.framing,
                    rpc_id: RpcId::Integer(id),
                    runtime: original.runtime.clone(),
                    cancellation: original.cancellation.clone(),
                    reservation: Reservation {
                        registry: original.reservation.registry.clone(),
                        call: original.reservation.call.clone(),
                        generation: original.reservation.generation,
                    },
                    tool_reservation: Some(reservation),
                    tool_options: Some(options),
                    #[cfg(any(test, feature = "mcp-http"))]
                    http_head: original.http_head.clone(),
                },
            }),
            cancellation: self.cancellation.clone(),
            attempted: false,
            written: Arc::new(AtomicBool::new(false)),
        };
        submission.checkpoint()?;
        Ok(submission)
    }
}

#[derive(Serialize)]
struct Envelope<'a> {
    jsonrpc: &'static str,
    id: i64,
    method: &'static str,
    params: Params<'a>,
}
#[derive(Serialize)]
struct Params<'a> {
    name: &'a str,
    arguments: &'a RawValue,
    #[serde(rename = "_meta")]
    metadata: McpClientMetadata,
    #[serde(rename = "inputResponses")]
    responses: &'a RawValue,
    #[serde(rename = "requestState", skip_serializing_if = "Option::is_none")]
    state: Option<&'a RawValue>,
}

#[cfg(test)]
#[path = "continuation/tests.rs"]
mod tests;
