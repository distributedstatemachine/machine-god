use super::{NativeMcpStartupError as Error, NativeMcpStartupPhase as Phase, control};
use crate::mcp::{
    protocol::NegotiatedProtocol,
    runtime::{
        NativeMcpOwnedPeer, NativeMcpPeerCompletion, NativeMcpRuntime, NativeMcpRuntimeCandidate,
        NativeMcpServerCandidate,
    },
};
use machine_god_core::CancellationToken;
use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_COMPLETIONS_PER_SERVER: usize = 4;

struct Completion {
    sealed: AtomicBool,
    peers: Mutex<Vec<NativeMcpPeerCompletion>>,
}
/// Observation only, including failed startup attempts and deferred child reap.
/// Completion says nothing about remote application effects or token revocation.
#[derive(Clone)]
pub struct NativeMcpStartupCompletion(Arc<Completion>);
impl NativeMcpStartupCompletion {
    pub(super) fn new() -> Self {
        Self(Arc::new(Completion {
            sealed: AtomicBool::new(false),
            peers: Mutex::new(Vec::new()),
        }))
    }
    pub(super) fn record(&self, peer: NativeMcpPeerCompletion) -> bool {
        let mut peers = self
            .0
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        peers.retain(|value| !value.is_complete());
        if self.0.sealed.load(Ordering::Acquire) || peers.len() == MAX_COMPLETIONS_PER_SERVER {
            return false;
        }
        peers.push(peer);
        true
    }
    pub(super) fn seal(&self) {
        self.0.sealed.store(true, Ordering::Release);
    }
    pub(super) fn settled(&self) -> bool {
        self.0
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .all(NativeMcpPeerCompletion::is_complete)
    }
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.0.sealed.load(Ordering::Acquire) && self.settled()
    }
    pub(super) async fn settle(
        &self,
        clock: &control::Clock,
        guards: &[CancellationToken],
        deadline: Instant,
    ) -> super::Result<()> {
        while !self.settled() {
            control::check(clock, guards, deadline)?;
            let next = clock
                .now()
                .checked_add(Duration::from_millis(5))
                .ok_or(Error::Limit)?
                .min(deadline);
            control::bounded(clock.0.sleep_until(next), clock, guards, deadline).await?;
        }
        control::check(clock, guards, deadline)
    }
}

/// Fixed server outcome, distinct from whether its owned cleanup has finished.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpStartupState {
    NotAttempted,
    Disabled,
    Deferred,
    Ready(NegotiatedProtocol),
    Failed(Error),
}
#[derive(Clone)]
pub struct NativeMcpStartupServerReceipt {
    pub name: Box<str>,
    pub required: bool,
    pub state: NativeMcpStartupState,
    pub attempts: u16,
    pub cleanup: NativeMcpStartupCompletion,
}
#[derive(Clone)]
pub struct NativeMcpStartupReceipt {
    pub phase: Phase,
    pub servers: Arc<[NativeMcpStartupServerReceipt]>,
    pub failure: Option<Error>,
}
impl NativeMcpStartupReceipt {
    /// A deferred-only observation cannot establish required-server readiness.
    #[must_use]
    pub fn required_ready(&self) -> bool {
        self.phase != Phase::AskDeferred
            && self.failure.is_none()
            && self.servers.iter().all(|server| {
                !server.required || matches!(server.state, NativeMcpStartupState::Ready(_))
            })
    }
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.failure.is_some()
            || self.servers.iter().any(|server| {
                matches!(server.state, NativeMcpStartupState::Failed(_))
                    || server.required && server.state == NativeMcpStartupState::Disabled
            })
    }
    #[must_use]
    pub fn cleanup_complete(&self) -> bool {
        self.servers
            .iter()
            .all(|server| server.cleanup.is_complete())
    }
}

/// Explicit acceptance policy: startup may tolerate unavailable optional peers;
/// reload can require every selected server before replacing an old runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMcpStartupRequirement {
    Required,
    AllSelected,
}

pub struct NativeMcpStartupFailure {
    pub error: Error,
    pub receipt: NativeMcpStartupReceipt,
}
impl fmt::Display for NativeMcpStartupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for NativeMcpStartupFailure {}

/// Complete private batch. Drop closes its concrete peers; completion receipts
/// can be cloned beforehand and retain observation, not worker/peer authority.
pub struct NativeMcpStartupBatch {
    pub(super) servers: Vec<NativeMcpServerCandidate>,
    pub(super) receipt: NativeMcpStartupReceipt,
    pub(super) _permit: Option<BuildPermit>,
}

pub(super) struct BuildPermit {
    pub pending: Arc<AtomicBool>,
    #[cfg(feature = "mcp-http")]
    pub identities: Option<super::authentication::IdentityCleanup>,
}
impl Drop for BuildPermit {
    fn drop(&mut self) {
        #[cfg(feature = "mcp-http")]
        drop(self.identities.take());
        self.pending.store(false, Ordering::Release);
    }
}
impl NativeMcpStartupBatch {
    #[must_use]
    pub fn receipt(&self) -> &NativeMcpStartupReceipt {
        &self.receipt
    }
    #[must_use]
    pub fn servers(&self) -> &[NativeMcpServerCandidate] {
        &self.servers
    }

    /// Builds an unpublished full runtime candidate. Never publishes or replaces
    /// the current runtime, even when descriptor/schema admission succeeds.
    /// # Errors
    /// Rejects deferred-only replacement, failed required readiness, selected
    /// reload failures or aggregate runtime admission, preserving cleanup evidence.
    pub fn prepare(
        mut self,
        runtime: &NativeMcpRuntime,
        reserved: &[&str],
        requirement: NativeMcpStartupRequirement,
    ) -> std::result::Result<
        (NativeMcpRuntimeCandidate, NativeMcpStartupReceipt),
        NativeMcpStartupFailure,
    > {
        let rejected = if self.receipt.phase == Phase::AskDeferred {
            Some(Error::DeferredBatch)
        } else if let Some(error) = self.receipt.failure {
            Some(error)
        } else if !self.receipt.required_ready() {
            Some(Error::RequiredUnavailable)
        } else if requirement == NativeMcpStartupRequirement::AllSelected
            && self.receipt.has_failures()
        {
            Some(Error::Unavailable)
        } else {
            None
        };
        if let Some(error) = rejected {
            return Err(NativeMcpStartupFailure {
                error,
                receipt: self.receipt,
            });
        }
        runtime
            .prepare_candidate(std::mem::take(&mut self.servers), reserved)
            .map(|candidate| (candidate, self.receipt.clone()))
            .map_err(|_| NativeMcpStartupFailure {
                error: Error::Invalid,
                receipt: self.receipt,
            })
    }

    /// Transfers only deferred candidates to a separately authorized atomic
    /// additive publication path. It cannot establish required readiness.
    /// # Errors
    /// Rejects other phases or global cancellation/deadline/aggregate failure.
    pub fn into_deferred_servers(
        self,
    ) -> std::result::Result<
        (Vec<NativeMcpServerCandidate>, NativeMcpStartupReceipt),
        NativeMcpStartupFailure,
    > {
        if self.receipt.phase != Phase::AskDeferred || self.receipt.failure.is_some() {
            return Err(NativeMcpStartupFailure {
                error: self.receipt.failure.unwrap_or(Error::DeferredBatch),
                receipt: self.receipt,
            });
        }
        Ok((self.servers, self.receipt))
    }
}

pub(super) struct AttemptOwner {
    pub completion: NativeMcpStartupCompletion,
    pub cancellation: CancellationToken,
    pub transferred: bool,
}
impl Drop for AttemptOwner {
    fn drop(&mut self) {
        if !self.transferred {
            self.cancellation.cancel();
        }
        self.completion.seal();
    }
}
pub(super) fn protocol(peer: &NativeMcpOwnedPeer) -> NegotiatedProtocol {
    match peer {
        NativeMcpOwnedPeer::Stdio(peer) => peer.protocol(),
        #[cfg(feature = "mcp-http")]
        NativeMcpOwnedPeer::Http(peer) => peer.protocol(),
        #[cfg(test)]
        NativeMcpOwnedPeer::Script(_) => unreachable!("startup only creates concrete peers"),
    }
}

macro_rules! redacted { ($($ty:ty),+ $(,)?) => {$(impl fmt::Debug for $ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($ty), " { <redacted> }")) }
})+}; }
redacted!(
    NativeMcpStartupCompletion,
    NativeMcpStartupServerReceipt,
    NativeMcpStartupReceipt,
    NativeMcpStartupFailure,
    NativeMcpStartupBatch
);
