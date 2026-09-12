//! Caller-polled reload retains the exact conversation fence and real receipt.

use super::{CancelOnDrop, ControlFuture, ControlPermit, Error, NativeMcpManagementError, Receipt};
use crate::{NativeConversationRuntime, mcp::controller::NativeMcpController};
use machine_god_core::CancellationToken;
use std::{sync::Arc, time::Duration};

/// Explicit interactive policy; configured peer timeouts remain independently
/// bounded by this outer reload deadline. No wall-clock timestamp is converted.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(60);

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
        let deadline = controller
            .deadline_after(RELOAD_TIMEOUT)
            .map_err(Error::McpReload)?;
        // The controller preserves publication receipts when cancellation races
        // completion. Never select this accepted operation away or check the
        // caller's cancellation again after its exact receipt has returned.
        controller
            .reload(cancellation, deadline)
            .await
            .map(Receipt::McpReload)
            .map_err(Error::McpReload)
    })
}
