//! Issued credential lifetime, retained in the selected clock's domain.

use super::{
    Arc, BoxFuture, CancellationToken, Instant, McpAuthClock, McpAuthError, McpAuthIdentity,
    McpAuthLease, Result,
};
use futures_util::future::select;
use std::time::Duration;

const REFRESH_SKEW_MS: u64 = 60_000;

#[derive(Clone)]
pub(super) struct Lifetime {
    clock: Arc<dyn McpAuthClock>,
    expires_at: Option<Instant>,
}

impl Lifetime {
    pub(super) fn new(clock: Arc<dyn McpAuthClock>, expires_ms: i64) -> Result<Self> {
        let expires_at = if expires_ms == i64::MAX {
            None
        } else {
            // Capture monotonic first: time spent sampling cannot extend the lease.
            let now = clock.now();
            let remaining = (i128::from(expires_ms) - i128::from(clock.unix_millis())).max(0);
            let millis = u64::try_from(remaining).map_err(|_| McpAuthError::Limit)?;
            Some(
                now.checked_add(Duration::from_millis(millis))
                    .ok_or(McpAuthError::Limit)?,
            )
        };
        Ok(Self { clock, expires_at })
    }

    fn due(&self, expires_ms: i64, skew_ms: u64) -> bool {
        self.expires_at.is_some_and(|deadline| {
            deadline.saturating_duration_since(self.clock.now()) <= Duration::from_millis(skew_ms)
                || i128::from(expires_ms) - i128::from(self.clock.unix_millis())
                    <= i128::from(skew_ms)
        })
    }
}

impl McpAuthLease {
    #[must_use]
    pub fn identity(&self) -> &McpAuthIdentity {
        &self.credentials.identity
    }

    fn check_authority(&self) -> Result<()> {
        if let Some(profile) = &self.profile {
            profile.check()?;
        }
        if self.generation.is_cancelled() {
            return Err(McpAuthError::Conflict);
        }
        Ok(())
    }

    /// # Errors
    /// Rejects original generation/profile cutoff and expired credentials.
    pub fn access_token(&self) -> Result<&[u8]> {
        self.check_authority()?;
        if self.lifetime.due(self.credentials.expires_ms, 0) {
            return Err(McpAuthError::Rejected);
        }
        Ok(self.credentials.access.bytes())
    }

    /// Immutable hard expiry in the injected authorization clock's domain.
    /// `None` means the token did not specify expiry, not renewed authority.
    #[must_use]
    pub fn expires_at(&self) -> Option<Instant> {
        self.lifetime.expires_at
    }

    /// Whether refresh is due within the pinned 60-second skew, including expiry.
    /// # Errors
    /// Rejects original generation/profile cutoff; expiry itself returns `true`.
    pub fn refresh_due(&self) -> Result<bool> {
        self.check_authority()?;
        Ok(self
            .lifetime
            .due(self.credentials.expires_ms, REFRESH_SKEW_MS))
    }

    /// Original generation cutoff only; expiry does not spawn a token watcher.
    #[must_use]
    pub fn generation(&self) -> CancellationToken {
        self.generation.clone()
    }

    /// Observes original owner/generation cutoff or selected-clock hard expiry.
    /// Construction starts no timer; dropping the future starts no cleanup work.
    #[must_use]
    pub fn cancelled_owned(&self) -> BoxFuture<'static, ()> {
        let generation = self.generation.clone();
        let profile = self.profile.clone();
        let lifetime = self.lifetime.clone();
        let expires_ms = self.credentials.expires_ms;
        Box::pin(async move {
            if generation.is_cancelled()
                || profile
                    .as_ref()
                    .is_some_and(|profile| profile.check().is_err())
                || lifetime.due(expires_ms, 0)
            {
                return;
            }
            let stopped = async {
                if let Some(profile) = profile {
                    select(generation.cancelled(), Box::pin(profile.stopped())).await;
                } else {
                    generation.cancelled().await;
                }
            };
            let expired = async {
                if let Some(deadline) = lifetime.expires_at {
                    lifetime.clock.sleep_until(deadline).await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            select(Box::pin(stopped), Box::pin(expired)).await;
        })
    }
}

#[cfg(test)]
mod tests;
