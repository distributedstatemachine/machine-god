use super::{Error, McpTransportConfig, NativeMcpStartup, Result};
use crate::mcp::{config::MAX_SERVERS, http_peer::McpHttpAuthentication};
use machine_god_core::CancellationToken;
use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

/// Historical HTTP data only. Neither this receipt nor successful revalidation
/// grants browser, credential, destination, prompt or executable authority.
pub struct NativeMcpStartupAuthChallenge {
    source: Weak<Mutex<Challenges>>,
    server: Box<str>,
    response: McpHttpAuthentication,
    generations: Vec<CancellationToken>,
    charge: usize,
}

enum Latest {
    Missing,
    Observed(Weak<NativeMcpStartupAuthChallenge>),
    Limit,
}
pub(in crate::mcp::startup) struct Challenges {
    latest: [Latest; MAX_SERVERS],
    retained: Vec<Arc<NativeMcpStartupAuthChallenge>>,
}
impl Default for Challenges {
    fn default() -> Self {
        Self {
            latest: std::array::from_fn(|_| Latest::Missing),
            retained: Vec::new(),
        }
    }
}
fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
impl Challenges {
    fn prune(&mut self) -> Vec<Arc<NativeMcpStartupAuthChallenge>> {
        self.retained.extract_if(.., |value| {
            Arc::strong_count(value) == 1 && !self.latest.iter().any(|latest| {
                matches!(latest, Latest::Observed(current) if current.ptr_eq(&Arc::downgrade(value)))
            })
        }).collect()
    }
    fn charge(&self) -> usize {
        // Fixed source slots are constructor metadata, like configuration and
        // cleanup slots. Every retained response and its allocation overhead is
        // charged; external Arc owners keep replaced records charged as well.
        self.retained.iter().map(|value| value.charge).sum()
    }
}

impl NativeMcpStartupAuthChallenge {
    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }
    #[must_use]
    pub fn status(&self) -> u16 {
        self.response.status
    }
    #[must_use]
    pub fn challenges(&self) -> &[Box<[u8]>] {
        &self.response.challenges
    }

    /// Checks the exact source and latest selected lifetime observations only.
    /// Actual profile observation and command consent remain host obligations.
    /// # Errors
    /// Rejects foreign, replaced, dropped or cancelled startup witnesses.
    pub fn revalidate(&self, startup: &NativeMcpStartup) -> Result<()> {
        if !self.source.ptr_eq(&Arc::downgrade(&startup.challenges)) {
            return Err(Error::Invalid);
        }
        if self.generations.iter().any(CancellationToken::is_cancelled) {
            return Err(Error::Cancelled);
        }
        let index = startup.challenge_index(&self.server)?;
        let state = lock(&startup.challenges);
        if matches!(&state.latest[index], Latest::Observed(current) if std::ptr::eq(current.as_ptr(), self))
        {
            Ok(())
        } else {
            Err(Error::Unavailable)
        }
    }
}

impl NativeMcpStartup {
    pub(in crate::mcp::startup) fn observe_http_error(
        &self,
        server: &str,
        auth_generation: Option<&CancellationToken>,
        maximum: usize,
        error: crate::mcp::http_peer::McpHttpPeerError,
    ) -> Error {
        use crate::mcp::http_peer::McpHttpPeerError;
        match error {
            McpHttpPeerError::Authentication(response) => {
                self.capture_challenge(server, response, auth_generation, maximum);
                Error::Authentication
            }
            McpHttpPeerError::Deadline => Error::Deadline,
            McpHttpPeerError::Cancelled => Error::Cancelled,
            McpHttpPeerError::Limit => Error::Limit,
            _ => Error::Unavailable,
        }
    }

    fn challenge_index(&self, server: &str) -> Result<usize> {
        self.configuration
            .servers()
            .iter()
            .position(|entry| {
                entry.name() == server && !matches!(entry.transport(), McpTransportConfig::Stdio(_))
            })
            .ok_or(Error::Invalid)
    }

    /// Borrows exact observed bytes through a bounded retained owner. Lookup is
    /// historical even after cancellation; use revalidation before deriving any
    /// separately authorized command input from the latest observation.
    /// # Errors
    /// Rejects unknown/nonremote names, or reports the latest retention limit.
    pub fn authentication_challenge(
        &self,
        server: &str,
    ) -> Result<Option<Arc<NativeMcpStartupAuthChallenge>>> {
        let index = self.challenge_index(server)?;
        let state = lock(&self.challenges);
        match &state.latest[index] {
            Latest::Missing => Ok(None),
            Latest::Limit => Err(Error::Limit),
            Latest::Observed(value) => Ok(value.upgrade()),
        }
    }

    pub(in crate::mcp::startup) fn challenge_charge(&self) -> usize {
        let (charge, discarded) = {
            let mut state = lock(&self.challenges);
            let discarded = state.prune();
            (state.charge(), discarded)
        };
        drop(discarded);
        charge
    }

    pub(in crate::mcp::startup) fn clear_challenge(&self, server: &str) {
        if let Ok(index) = self.challenge_index(server) {
            let discarded = {
                let mut state = lock(&self.challenges);
                state.latest[index] = Latest::Missing;
                state.prune()
            };
            drop(discarded);
        }
    }

    pub(in crate::mcp::startup) fn capture_challenge(
        &self,
        server: &str,
        response: McpHttpAuthentication,
        auth_generation: Option<&CancellationToken>,
        maximum: usize,
    ) {
        let Ok(index) = self.challenge_index(server) else {
            return;
        };
        let mut generations = vec![self.owner.clone(), self.configuration_generation.clone()];
        if let Some(network) = &self.network {
            generations.push(network.owner_cancellation());
        }
        generations.extend(auth_generation.cloned());
        // The HTTP peer already enforces these bounds. Recheck before retention
        // so this private handoff never relies on an unchecked future caller.
        let bytes = response
            .challenges
            .iter()
            .try_fold(0usize, |total, header| total.checked_add(header.len()));
        let charge = bytes.and_then(|bytes| bytes.checked_add(server.len() + 512));
        let discarded = {
            let mut state = lock(&self.challenges);
            state.latest[index] = Latest::Limit;
            let discarded = state.prune();
            if let Some(charge) = charge
                && matches!(response.status, 401 | 403)
                && response.challenges.len() <= 8
                && bytes.is_some_and(|bytes| bytes <= 16 * 1024)
                && charge <= maximum
                && state.retained.len() < 2 * MAX_SERVERS
                && state
                    .charge()
                    .checked_add(charge)
                    .is_some_and(|total| total <= self.max_retained_bytes)
            {
                let observation = Arc::new(NativeMcpStartupAuthChallenge {
                    source: Arc::downgrade(&self.challenges),
                    server: server.into(),
                    response,
                    generations,
                    charge,
                });
                state.latest[index] = Latest::Observed(Arc::downgrade(&observation));
                state.retained.push(observation);
            }
            discarded
        };
        // Releasing a last lifetime token may release arbitrary registered
        // wakers; never destroy retired observations under the source lock.
        drop(discarded);
    }
}

impl fmt::Debug for NativeMcpStartupAuthChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeMcpStartupAuthChallenge { <redacted> }")
    }
}
