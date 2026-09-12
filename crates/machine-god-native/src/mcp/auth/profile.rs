//! Exact selected profile custody, checked only on actual credential workers.

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
}
redacted!(NativeMcpAuthProfile);

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
        }
    }

    pub(super) fn check(&self) -> Result<()> {
        if self.owner.is_cancelled() || self.configuration.is_cancelled() {
            Err(McpAuthError::Conflict)
        } else {
            Ok(())
        }
    }

    pub(super) async fn stopped(&self) {
        select(self.owner.cancelled(), self.configuration.cancelled()).await;
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
