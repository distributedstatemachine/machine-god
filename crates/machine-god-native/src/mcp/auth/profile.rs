//! Exact selected profile custody, checked only on actual credential workers.

#[cfg(any(test, feature = "ai-gateway-http"))]
use super::McpAuthConfig;
use super::{McpAuthError, Result, redacted};
use crate::mcp::store::{NativeMcpConfigSnapshot, NativeMcpConfigStore, NativeMcpConfigStoreError};
use futures_util::future::select;
use machine_god_core::CancellationToken;
use std::{fmt, sync::Arc};

/// Opaque native-selected profile observation. Copying its Arc does not renew
/// its exact configuration or owner lifetime. It cannot be constructed from
/// model/remote data, and is not browser consent or tool execution authority.
pub struct NativeMcpAuthProfile {
    store: Arc<NativeMcpConfigStore>,
    snapshot: Arc<NativeMcpConfigSnapshot>,
    owner: CancellationToken,
    configuration: CancellationToken,
    command: Option<Arc<dyn CommandCustody>>,
}
redacted!(NativeMcpAuthProfile);

/// Native-only admitted control custody. Workers retain the actual conversation
/// and controller reservations, not merely an outer observer's cancellation.
pub(crate) trait CommandCustody: Send + Sync {
    fn check(&self) -> Result<()>;
    fn cancelled(&self) -> machine_god_core::BoxFuture<'_, ()>;
}

#[cfg(any(test, feature = "ai-gateway-http"))]
pub(crate) struct SelectedConfig {
    pub config: McpAuthConfig,
    pub profile: Arc<NativeMcpAuthProfile>,
}

impl NativeMcpAuthProfile {
    pub(crate) fn new(
        store: Arc<NativeMcpConfigStore>,
        snapshot: Arc<NativeMcpConfigSnapshot>,
        owner: CancellationToken,
        configuration: CancellationToken,
    ) -> Self {
        Self {
            store,
            snapshot,
            owner,
            configuration,
            command: None,
        }
    }

    #[cfg(any(test, feature = "ai-gateway-http"))]
    pub(crate) fn with_command(mut self, command: Arc<dyn CommandCustody>) -> Self {
        self.command = Some(command);
        self
    }

    pub(super) fn check(&self) -> Result<()> {
        if let Some(command) = &self.command {
            command.check()?;
        }
        if self.owner.is_cancelled() || self.configuration.is_cancelled() {
            Err(McpAuthError::Conflict)
        } else {
            Ok(())
        }
    }

    pub(super) async fn stopped(&self) {
        let command = async {
            if let Some(command) = &self.command {
                command.cancelled().await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        select(
            Box::pin(command),
            Box::pin(select(
                self.owner.cancelled(),
                self.configuration.cancelled(),
            )),
        )
        .await;
    }

    pub(super) fn validate(&self) -> Result<()> {
        self.check()?;
        self.unchanged()?;
        self.check()
    }

    pub(super) fn unchanged(&self) -> Result<()> {
        self.store
            .validate_unchanged(&self.snapshot)
            .map_err(map_error)
    }

    pub(super) fn lock(&self) -> Result<crate::bounded_profile_file::LockedProfileUpdate<'_>> {
        self.check()?;
        let guard = self
            .store
            .lock_unchanged(&self.snapshot)
            .map_err(map_error)?;
        self.check()?;
        Ok(guard)
    }
}

fn map_error(error: NativeMcpConfigStoreError) -> McpAuthError {
    match error {
        NativeMcpConfigStoreError::Busy => McpAuthError::Busy,
        NativeMcpConfigStoreError::Conflict => McpAuthError::Conflict,
        _ => McpAuthError::Persistence,
    }
}
