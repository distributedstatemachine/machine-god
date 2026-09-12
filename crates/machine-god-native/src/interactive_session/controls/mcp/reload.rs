//! Caller-polled reload retains the exact conversation fence and real receipt.

use super::{CancelOnDrop, ControlFuture, ControlPermit, Error, NativeMcpManagementError, Receipt};
use crate::{NativeConversationRuntime, mcp::controller::NativeMcpController};
use machine_god_core::CancellationToken;
use std::sync::Arc;

pub(super) fn run(
    conversation: Arc<NativeConversationRuntime>,
    controller: Arc<NativeMcpController>,
    cancellation: CancellationToken,
) -> ControlFuture {
    let cancelled = CancelOnDrop(cancellation.clone());
    Box::pin(async move {
        let _cancelled = cancelled;
        if cancellation.is_cancelled() {
            return Err(Error::Mcp(NativeMcpManagementError::Cancelled));
        }
        let _permit = ControlPermit::acquire(&conversation)?;
        // The controller preserves publication receipts when cancellation races
        // completion. Never select this accepted operation away or check the
        // caller's cancellation again after its exact receipt has returned.
        controller
            .reload_configured(cancellation)
            .await
            .map(Receipt::McpReload)
            .map_err(Error::McpReload)
    })
}
