//! MCP controls retain exact conversation admission and native effect owners.

#[cfg(feature = "mcp-http")]
mod authentication;
mod feature;
mod reload;
#[cfg(test)]
mod tests;
pub use feature::NativeMcpHumanFeatureReceipt;

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
    browser: Option<crate::mcp::browser_launcher::NativeMcpBrowserLauncher>,
    selected: crate::managed::manager::factory::ManagedMcpControls,
) -> Result<(CancellationToken, ControlFuture), NativeInteractiveError> {
    let token = CancellationToken::new();
    #[cfg(not(feature = "mcp-http"))]
    let _ = browser;
    let command = match command {
        #[cfg(feature = "mcp-http")]
        McpCommand::Authenticate {
            server,
            open_browser,
        } => {
            let Some(controller) = selected.controller else {
                return Ok((token, unavailable()));
            };
            return Ok((
                token.clone(),
                authentication::run(
                    runtime,
                    controller,
                    browser,
                    server,
                    Some(open_browser),
                    token,
                ),
            ));
        }
        #[cfg(feature = "mcp-http")]
        McpCommand::Logout { server } => {
            let Some(controller) = selected.controller else {
                return Ok((token, unavailable()));
            };
            return Ok((
                token.clone(),
                authentication::run(runtime, controller, browser, server, None, token),
            ));
        }
        McpCommand::Reload => {
            let Some(controller) = selected.controller else {
                return Ok((token, unavailable()));
            };
            return Ok((token.clone(), reload::run(runtime, controller, token)));
        }
        McpCommand::Feature(command) => {
            let request = crate::McpFeatureRequest::try_from(command)
                .map_err(|_| NativeInteractiveError::Configuration)?;
            let Some(mcp) = selected.runtime else {
                return Ok((token, unavailable()));
            };
            return Ok((token.clone(), feature::run(runtime, mcp, request, token)));
        }
        command => command,
    };
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

fn unavailable() -> ControlFuture {
    Box::pin(async { Err(Error::Mcp(NativeMcpManagementError::RuntimeUnavailable)) })
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        contain_release(|| {
            self.0.cancel();
        });
    }
}

/// Admission release can wake a caller. A panicking waiter cannot erase an
/// already-completed receipt or cause a second unwind while dropping a future.
struct ControlPermit(Option<crate::conversation_lifecycle::LifecyclePermit>);
impl ControlPermit {
    fn acquire(runtime: &NativeConversationRuntime) -> Result<Self, Error> {
        runtime
            .acquire_file_control()
            .map(|permit| Self(Some(permit)))
            .map_err(Error::Runtime)
    }
}
impl Drop for ControlPermit {
    fn drop(&mut self) {
        if let Some(permit) = self.0.take() {
            contain_release(|| drop(permit));
        }
    }
}
fn contain_release(operation: impl FnOnce()) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        std::mem::forget(payload);
    }
}
