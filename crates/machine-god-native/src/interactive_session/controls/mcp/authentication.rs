//! Explicit human consent and native-owned authentication effects.

use super::{CancelOnDrop, ControlFuture, Error, Receipt};
use crate::{
    NativeConversationRuntime,
    mcp::{
        auth::{McpAuthBrowser, McpAuthBrowserRequest, McpAuthError},
        browser_launcher::{
            NativeMcpBrowserLaunchError, NativeMcpBrowserLaunchOutcome, NativeMcpBrowserLauncher,
            NativeMcpBrowserUrl,
        },
        controller::{
            ControlFence, NativeMcpAuthSelection, NativeMcpAuthenticationError as AuthError,
            NativeMcpAuthenticationReceipt as AuthReceipt, NativeMcpController,
        },
    },
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) fn run(
    conversation: Arc<NativeConversationRuntime>,
    controller: Arc<NativeMcpController>,
    launcher: Option<NativeMcpBrowserLauncher>,
    server: String,
    open: Option<bool>,
    cancellation: CancellationToken,
) -> ControlFuture {
    let cancelled = CancelOnDrop(cancellation.clone());
    Box::pin(async move {
        let _cancelled = cancelled;
        if cancellation.is_cancelled() {
            return Err(authorization(McpAuthError::Cancelled));
        }
        let fence = ControlFence::new(
            conversation
                .acquire_file_control()
                .map_err(Error::Runtime)?,
        );
        let deadline = controller
            .deadline_after(Duration::from_secs(30 * 60))
            .map_err(|error| Error::McpAuthentication(AuthError::Selection(error)))?;
        let selected = controller
            .prepare_authentication(server, fence.clone(), cancellation.clone(), deadline)
            .await
            .map_err(|error| Error::McpAuthentication(AuthError::Selection(error)))?;
        let server = selected.server.clone();
        let receipt = match open {
            Some(false) => AuthReceipt::ConfirmationRequired { server },
            None => AuthReceipt::LoggedOut {
                server,
                outcome: selected.logout().await.map_err(authorization)?,
            },
            Some(true) => {
                let launcher = launcher
                    .as_ref()
                    .ok_or_else(|| authorization(McpAuthError::Unavailable))?;
                let browser = Browser {
                    selected: &selected,
                    launcher,
                };
                let lease = selected
                    .authenticate(&browser)
                    .await
                    .map_err(authorization)?;
                let usable = lease.access_token().is_ok();
                drop(lease);
                // Release command admission before activation, but retain the real
                // conversation fence through the independent activation receipt.
                drop(selected);
                let activation = if usable {
                    Some(controller.reload_configured(cancellation).await)
                } else {
                    None
                };
                AuthReceipt::Authenticated {
                    server,
                    usable,
                    activation,
                }
            }
        };
        drop(fence);
        Ok(Receipt::McpAuthentication(receipt))
    })
}

fn authorization(error: McpAuthError) -> Error {
    Error::McpAuthentication(AuthError::Authorization(error))
}

struct Browser<'a> {
    selected: &'a NativeMcpAuthSelection,
    launcher: &'a NativeMcpBrowserLauncher,
}
impl McpAuthBrowser for Browser<'_> {
    fn approve<'a>(
        &'a self,
        _request: &'a McpAuthBrowserRequest,
        cancellation: &'a CancellationToken,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<bool, McpAuthError>> {
        Box::pin(async move {
            self.selected.check()?;
            if cancellation.is_cancelled() {
                return Err(McpAuthError::Cancelled);
            }
            // This adapter is constructed only for the admitted --open command.
            Ok(true)
        })
    }

    fn launch<'a>(
        &'a self,
        request: &'a McpAuthBrowserRequest,
        cancellation: &'a CancellationToken,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<(), McpAuthError>> {
        Box::pin(async move {
            self.selected.check()?;
            if cancellation.is_cancelled() {
                return Err(McpAuthError::Cancelled);
            }
            let url = NativeMcpBrowserUrl::new(request.url()).map_err(|_| McpAuthError::Invalid)?;
            let outcome = self
                .launcher
                .launch(
                    url,
                    cancellation.clone(),
                    self.selected.owner_cancellation.clone(),
                    deadline,
                )
                .await
                .map_err(|error| match error {
                    NativeMcpBrowserLaunchError::Cancelled => McpAuthError::Cancelled,
                    NativeMcpBrowserLaunchError::TimedOut => McpAuthError::Deadline,
                    NativeMcpBrowserLaunchError::Busy => McpAuthError::Busy,
                    _ => McpAuthError::Unavailable,
                })?;
            self.selected.check()?;
            match outcome {
                NativeMcpBrowserLaunchOutcome::Opened => Ok(()),
                NativeMcpBrowserLaunchOutcome::LauncherFailed
                | NativeMcpBrowserLaunchOutcome::Indeterminate => Err(McpAuthError::Unavailable),
            }
        })
    }
}
