//! MCP profile management shares the exact runtime fence and owned worker scope.

use super::{
    ControlFuture, NativeInteractiveControlError as Error,
    NativeInteractiveControlReceipt as Receipt,
};
use crate::mcp::{
    commands::McpCommand,
    management::{NativeMcpManagementError, NativeMcpManagementService},
};
use crate::{NativeConversationRuntime, NativeInteractiveError, NativeReferenceHost};
use machine_god_core::CancellationToken;
use std::sync::Arc;

pub(super) fn prepare(
    runtime: Arc<NativeConversationRuntime>,
    host: &NativeReferenceHost,
    command: McpCommand,
) -> Result<(CancellationToken, ControlFuture), NativeInteractiveError> {
    let token = CancellationToken::new();
    match NativeMcpManagementService::validate_command(&command) {
        Ok(()) => {}
        Err(NativeMcpManagementError::RuntimeUnavailable) => {
            return Ok((
                token,
                Box::pin(async { Err(Error::Mcp(NativeMcpManagementError::RuntimeUnavailable)) }),
            ));
        }
        Err(_) => return Err(NativeInteractiveError::Configuration),
    }
    let Some(service) = host.mcp_management() else {
        return Ok((token, Box::pin(async { Err(Error::Unavailable) })));
    };
    let workers = host
        .control_workers()
        .ok_or(NativeInteractiveError::Configuration)?;
    let future = super::owned_operation::run(
        runtime,
        workers,
        token.clone(),
        Error::Mcp(NativeMcpManagementError::Cancelled),
        move |cancellation| {
            service
                .execute(command, cancellation)
                .map(Receipt::Mcp)
                .map_err(Error::Mcp)
        },
    );
    Ok((token, future))
}
